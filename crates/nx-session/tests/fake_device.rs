//! The session against a scripted HX Stomp (`nx_session::fake`).

use std::thread;
use std::time::{Duration, Instant};

use nx_proto::{FrameKind, Request};
use nx_session::fake::{FakeDevice, Reply};
use nx_session::{Channel, Config, Error, Event, Session};
use rmpv::Value;

fn config() -> Config {
    Config {
        ack_delay: Duration::from_millis(5),
        keepalive_interval: Duration::from_secs(60),
        probe_linger: Duration::from_millis(10),
        reply_timeout: Duration::from_millis(300),
        deferred_timeout: Duration::from_millis(500),
        handshake_timeout: Duration::from_millis(300),
        ..Config::default()
    }
}

fn map(pairs: &[(u64, Value)]) -> Value {
    Value::Map(pairs.iter().map(|(k, v)| (Value::from(*k), v.clone())).collect())
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn connects_in_hx_edit_order() {
    let device = FakeDevice::new();
    let _session = Session::connect_with(device.transport(), config()).unwrap();
    let opened: Vec<(u16, u8)> = device.service_opens();
    assert_eq!(opened, vec![(0x1001, 5), (0x1080, 6), (0x1002, 4), (0x1001, 2)]);
    assert_eq!(device.hellos(), vec![0x1001, 0x1080, 0x1002, 0x1001]);
    assert_eq!(device.closes(), vec![0x1001]);
}

#[test]
fn matches_replies_to_requests_by_transaction() {
    let device = FakeDevice::new();
    device.on_request(|_, _, req| match req {
        Request::ReadGlobal { id } => Reply::Ok(map(&[(118, (*id).into()), (119, (id * 2).into())])),
        _ => Reply::Ok(Value::Nil),
    });
    let session = Session::connect_with(device.transport(), config()).unwrap();
    for id in [3u64, 14, 73] {
        let got = session.request(Request::ReadGlobal { id }).unwrap();
        assert_eq!(got, map(&[(118, id.into()), (119, (id * 2).into())]));
    }
    let txns: Vec<u64> = device.requests().iter().filter(|r| r.0 == Channel::Rpc).map(|r| r.1).collect();
    assert_eq!(txns, vec![1000, 1001, 1002]);
}

#[test]
fn control_and_rpc_channels_count_transactions_separately() {
    let device = FakeDevice::new();
    let session = Session::connect_with(device.transport(), config()).unwrap();
    session.request(Request::PresetInfo).unwrap();
    session.request_on(Channel::Control, Request::Other { opcode: 254, args: Value::Map(vec![]) }).unwrap();
    session.request(Request::PresetInfo).unwrap();
    let seen: Vec<(Channel, u64)> = device.requests().iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(seen, vec![(Channel::Rpc, 1000), (Channel::Control, 1000), (Channel::Rpc, 1001)]);
}

#[test]
fn a_reply_that_arrives_out_of_order_still_finds_its_request() {
    let device = FakeDevice::new();
    device.on_request(|_, txn, _| Reply::Ok(Value::from(txn)));
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let a = session.send(Channel::Rpc, Request::PresetInfo);
    let b = session.send(Channel::Control, Request::PresetInfo);
    assert_eq!(b.wait().unwrap(), Value::from(1000));
    assert_eq!(a.wait().unwrap(), Value::from(1000));
}

#[test]
fn notifications_are_acknowledged_so_the_device_keeps_sending() {
    let device = FakeDevice::new();
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let events = session.events();
    // 300 notifications of about 20 bytes is well past the 4 KB the device sends unacknowledged.
    for i in 0..300u64 {
        device.notify(30, map(&[(98, 3.into()), (119, Value::F64(i as f64 / 100.0))]));
    }
    let mut got = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while got < 300 {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(Event::Notification { id: 30, .. }) => got += 1,
            Ok(_) => {}
            Err(_) => panic!("only {got} notifications arrived; {} bytes unacknowledged", device.events_unacked()),
        }
    }
    wait_for("ACK of the last notification", || device.events_unacked() == 0);
    let acks = device.host_frames(Channel::Events).iter().filter(|f| f.kind() == Some(FrameKind::Ack)).count();
    assert!(acks > 0);
}

#[test]
fn a_deferred_operation_completes_on_notification_20() {
    let device = FakeDevice::new();
    device.on_request(|_, _, req| match req {
        Request::SelectPreset { .. } => Reply::Deferred,
        _ => Reply::Ok(Value::Nil),
    });
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let first = session.send(Channel::Rpc, Request::SelectPreset { setlist: 0, index: 2 });
    wait_for("the first select", || device.requests().len() == 1);
    thread::sleep(Duration::from_millis(30));
    let second = session.send(Channel::Rpc, Request::SelectPreset { setlist: 0, index: 3 });
    let info = session.send(Channel::Control, Request::PresetInfo);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(device.requests().len(), 1, "nothing may be sent while a deferred operation runs");
    assert!(first.try_take().is_none(), "status 1 is not completion");

    device.complete_deferred(1000, 0, Value::Nil);
    assert_eq!(first.wait().unwrap(), Value::Nil);
    wait_for("the second select", || device.requests().len() >= 2);
    device.complete_deferred(1001, 0, Value::Nil);
    assert_eq!(second.wait().unwrap(), Value::Nil);
    assert_eq!(info.wait().unwrap(), Value::Nil);
    let order: Vec<(Channel, u64)> = device.requests().iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(order, vec![(Channel::Rpc, 1000), (Channel::Rpc, 1001), (Channel::Control, 1000)]);
}

#[test]
fn a_failed_deferred_operation_reports_its_status() {
    let device = FakeDevice::new();
    device.on_request(|_, _, _| Reply::Deferred);
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let pending = session.send(Channel::Rpc, Request::SelectPreset { setlist: 0, index: 9 });
    wait_for("the select", || device.requests().len() == 1);
    device.complete_deferred(1000, 5, Value::from("nope"));
    match pending.wait() {
        Err(Error::Status { status: 5, .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_deferred_operation_that_never_completes_times_out() {
    let device = FakeDevice::new();
    device.on_request(|_, _, _| Reply::Deferred);
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let started = Instant::now();
    assert!(matches!(session.request(Request::SelectPreset { setlist: 0, index: 1 }), Err(Error::Timeout(_))));
    assert!(started.elapsed() >= config().deferred_timeout);
}

#[test]
fn a_timeout_is_reported_and_never_answered_with_a_new_handshake() {
    let device = FakeDevice::new();
    device.on_request(|_, _, req| match req {
        Request::PresetInfo => Reply::Silent,
        _ => Reply::Ok(Value::from(1)),
    });
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let hellos = device.hellos().len();
    assert!(matches!(session.request(Request::PresetInfo), Err(Error::Timeout(_))));
    assert!(matches!(session.request(Request::PresetInfo), Err(Error::Timeout(_))));
    assert_eq!(session.request(Request::ReadGlobal { id: 1 }).unwrap(), Value::from(1));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(device.hellos().len(), hellos);
}

#[test]
fn a_late_reply_is_dropped_not_matched_to_the_next_request() {
    let device = FakeDevice::new();
    device.on_request(|_, txn, _| if txn == 1000 { Reply::Late(Duration::from_millis(400)) } else { Reply::Ok(Value::from(txn)) });
    let session = Session::connect_with(device.transport(), config()).unwrap();
    assert!(matches!(session.request(Request::PresetInfo), Err(Error::Timeout(_))));
    assert_eq!(session.request(Request::PresetInfo).unwrap(), Value::from(1001));
    thread::sleep(Duration::from_millis(200));
    assert_eq!(session.request(Request::PresetInfo).unwrap(), Value::from(1002));
}

#[test]
fn a_device_that_does_not_answer_hello_fails_connect_once() {
    let device = FakeDevice::new();
    device.ignore_hello(true);
    let started = Instant::now();
    assert!(matches!(Session::connect_with(device.transport(), config()), Err(Error::Timeout(_))));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(device.hellos(), vec![0x1001], "one HELLO, no retry");
}

#[test]
fn a_wedged_device_is_reported_not_retried() {
    // PROTOCOL.md (map): a wedged device answers HELLO with bare acknowledgements.
    let device = FakeDevice::new();
    device.wedged(true);
    assert!(matches!(Session::connect_with(device.transport(), config()), Err(Error::Handshake { .. })));
    assert_eq!(device.hellos(), vec![0x1001]);
}

#[test]
fn a_stale_backlog_is_drained_before_hello() {
    let device = FakeDevice::new();
    device.backlog(50);
    let _session = Session::connect_with(device.transport(), config()).unwrap();
    assert_eq!(device.backlog_left(), 0);
}

#[test]
fn idle_channels_get_keep_alives_with_the_current_acknowledgement() {
    let device = FakeDevice::new();
    let cfg = Config { keepalive_interval: Duration::from_millis(40), ..config() };
    let session = Session::connect_with(device.transport(), cfg).unwrap();
    device.notify(30, Value::Nil);
    thread::sleep(Duration::from_millis(250));
    for ch in [Channel::Control, Channel::Rpc, Channel::Events] {
        let frames = device.host_frames(ch);
        let kas: Vec<_> = frames.iter().filter(|f| f.kind() == Some(FrameKind::KeepAlive)).collect();
        assert!(kas.len() >= 3, "{ch:?}: {} keep-alives", kas.len());
        let last = frames.last().unwrap();
        assert_eq!(last.ack(), Some(device.expected_ack(ch)), "{ch:?}");
    }
    drop(session);
}

#[test]
fn sequence_numbers_count_every_frame_and_wrap() {
    let device = FakeDevice::new();
    device.on_request(|_, _, _| Reply::Ok(Value::Nil));
    let session = Session::connect_with(device.transport(), config()).unwrap();
    for _ in 0..300 {
        session.request(Request::PresetInfo).unwrap();
    }
    let frames = device.host_frames(Channel::Rpc);
    assert_eq!(frames[0].kind(), Some(FrameKind::Hello));
    assert_eq!(frames[0].body[1], 0, "HELLO is seq 0");
    let seqs: Vec<u8> = frames[1..].iter().map(|f| f.seq().unwrap()).collect();
    assert_eq!(seqs[0], 2, "first frame after HELLO");
    for w in seqs.windows(2) {
        assert_eq!(w[1], w[0].wrapping_add(1));
    }
    assert!(seqs.len() > 300);
}

#[test]
fn a_large_request_waits_for_the_devices_window() {
    let device = FakeDevice::new();
    device.on_request(|_, _, _| Reply::Ok(Value::Nil));
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let blob = Value::Binary(vec![0xab; 3000]);
    session.request(Request::Other { opcode: 200, args: blob.clone() }).unwrap();
    assert!(device.max_unacked_from_host() <= 0x200, "{}", device.max_unacked_from_host());
    let got = device.requests().last().unwrap().2.clone();
    assert_eq!(got, Request::Other { opcode: 200, args: blob });
}

#[test]
fn the_device_closing_a_channel_fails_its_requests() {
    let device = FakeDevice::new();
    device.on_request(|_, _, _| Reply::Silent);
    let session = Session::connect_with(device.transport(), config()).unwrap();
    let events = session.events();
    let pending = session.send(Channel::Rpc, Request::PresetInfo);
    wait_for("the request", || device.requests().len() == 1);
    device.close_channel(Channel::Rpc);
    assert!(matches!(pending.wait(), Err(Error::ChannelClosed(Channel::Rpc))));
    assert!(matches!(session.request(Request::PresetInfo), Err(Error::ChannelClosed(Channel::Rpc))));
    let closed = events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(closed, Event::ChannelClosed(Channel::Rpc)));
    assert_eq!(device.hellos().len(), 4, "no reopen");
}

#[test]
fn close_ends_control_and_events_like_hx_edit() {
    let device = FakeDevice::new();
    let session = Session::connect_with(device.transport(), config()).unwrap();
    device.notify(30, Value::Nil);
    thread::sleep(Duration::from_millis(30));
    session.close().unwrap();
    assert_eq!(device.closes(), vec![0x1001, 0x1001, 0x1002]);
    assert_eq!(device.events_unacked(), 0);
}
