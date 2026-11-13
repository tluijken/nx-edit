//! Replays the device side of `local/captures/06-reconnect.pcapng` and checks that the session writes what
//! HX Edit wrote: the whole handshake byte for byte and in order, then every frame of the requests
//! that followed, per channel.
//!
//! A replay cannot check timing, so the device's frames are released per channel once the session
//! has written as many frames on that channel as HX Edit had at that point. Whether HX Edit
//! piggybacked an acknowledgement on its next request or sent a bare ACK first depended on how soon
//! it issued that request; the test reproduces that by issuing the request at once or only after the
//! session's ACK delay has passed.

use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nx_capture::{Filter, read_transfers};
use nx_proto::{FORMAT_HELLO, Frame, FrameKind, Reassembler, Request, split_transfer};
use nx_session::{Channel, Config, Session, Transport};

const ACK_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone)]
struct Captured {
    from_host: bool,
    channel: u16,
    raw: Vec<u8>,
    frame: Frame,
    ts_us: u64,
}

fn channel_of(f: &Frame, from_host: bool) -> u16 {
    if from_host { f.src } else { f.dst }
}

/// Frames of 06-reconnect from HX Edit's first HELLO up to its first keep-alive after connecting.
/// The captures stay out of git; where `local/captures` is absent the tests that need them pass
/// without checking anything.
fn captures_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/captures");
    if dir.exists() {
        Some(dir)
    } else {
        eprintln!("skipped: no {}", dir.display());
        None
    }
}

fn connect_window() -> Option<Vec<Captured>> {
    let path = captures_dir()?.join("06-reconnect.pcapng");
    let filter = Filter { device: Some(42), endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let mut out = Vec::new();
    let mut started = false;
    for t in read_transfers(File::open(path).unwrap(), &filter).unwrap() {
        let from_host = t.endpoint == 0x01;
        let frames = split_transfer(&t.data).unwrap();
        // Each transfer in this window holds one frame; keeping the raw bytes keeps the device's
        // junk padding, which the session must tolerate.
        for frame in frames {
            started |= from_host && frame.format == FORMAT_HELLO;
            if !started {
                continue;
            }
            if frame.kind() == Some(FrameKind::KeepAlive) {
                if from_host {
                    return Some(out);
                }
                continue;
            }
            let channel = channel_of(&frame, from_host);
            out.push(Captured { from_host, channel, raw: t.data.clone(), frame, ts_us: t.ts_us });
        }
    }
    Some(out)
}

#[derive(Default)]
struct ReplayState {
    written: Vec<Frame>,
    delivered: Vec<bool>,
}

#[derive(Clone)]
struct Replay {
    capture: Arc<Vec<Captured>>,
    state: Arc<Mutex<ReplayState>>,
}

impl Replay {
    fn new(capture: Vec<Captured>) -> Self {
        let n = capture.len();
        Self {
            capture: Arc::new(capture),
            state: Arc::new(Mutex::new(ReplayState { written: Vec::new(), delivered: vec![false; n] })),
        }
    }

    fn written(&self) -> Vec<Frame> {
        self.state.lock().unwrap().written.clone()
    }

    fn next_ready(&self) -> Option<Vec<u8>> {
        let mut st = self.state.lock().unwrap();
        let mut blocked: Vec<u16> = Vec::new();
        for (i, c) in self.capture.iter().enumerate() {
            if c.from_host || st.delivered[i] || blocked.contains(&c.channel) {
                continue;
            }
            let needed = self.capture[..i].iter().filter(|h| h.from_host && h.channel == c.channel).count();
            let have = st.written.iter().filter(|f| f.src == c.channel).count();
            if have >= needed {
                st.delivered[i] = true;
                return Some(c.raw.clone());
            }
            blocked.push(c.channel);
        }
        None
    }
}

impl Transport for Replay {
    fn write(&mut self, transfer: &[u8]) -> io::Result<()> {
        let frames = split_transfer(transfer).map_err(|e| io::Error::other(e.to_string()))?;
        self.state.lock().unwrap().written.extend(frames);
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(t) = self.next_ready() {
                return Ok(Some(t));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn replay_config() -> Config {
    Config {
        ack_delay: ACK_DELAY,
        keepalive_interval: Duration::from_secs(60),
        probe_linger: Duration::from_millis(50),
        ..Config::default()
    }
}

/// The requests HX Edit sent on one channel after connecting, each with whether its first frame
/// carried fresh acknowledgement (kind 0x0c). A plain data frame (0x04) means HX Edit had already
/// acknowledged the previous reply with a bare ACK, so the test waits out the ACK delay first.
fn captured_requests(capture: &[Captured], channel: u16) -> VecDeque<(Request, bool)> {
    let mut reassembler = Reassembler::default();
    let mut out = VecDeque::new();
    let mut first_kind = None;
    for c in capture.iter().filter(|c| c.from_host && c.channel == channel) {
        if reassembler.buffered(c.frame.src, c.frame.dst) == 0 {
            first_kind = c.frame.kind();
        }
        for msg in reassembler.push(&c.frame).unwrap() {
            if let Some((_, req)) = Request::from_value(&msg.decode().unwrap()) {
                out.push_back((req, first_kind == Some(FrameKind::Data)));
            }
        }
    }
    out
}

fn hex(f: &Frame) -> String {
    f.encode().iter().map(|b| format!("{b:02x}")).collect()
}

fn drive(session: &Session, channel: Channel, requests: VecDeque<(Request, bool)>) -> Vec<Result<rmpv::Value, nx_session::Error>> {
    let mut out = Vec::new();
    for (req, after_bare_ack) in requests {
        if after_bare_ack {
            thread::sleep(ACK_DELAY * 3);
        }
        out.push(session.request_on(channel, req));
    }
    out
}

#[test]
fn replays_hx_edit_connect_byte_for_byte() {
    let Some(capture) = connect_window() else { return };
    let replay = Replay::new(capture.clone());
    let session = Session::connect_with(replay.clone(), replay_config()).unwrap();

    let host: Vec<&Captured> = capture.iter().filter(|c| c.from_host).collect();
    let handshake_len = host.iter().position(|c| Request::from_value(&first_value(&c.frame)).is_some()).unwrap();
    let written = replay.written();
    let expected: Vec<String> = host[..handshake_len].iter().map(|c| hex(&c.frame)).collect();
    let got: Vec<String> = written.iter().take(handshake_len).map(hex).collect();
    assert_eq!(got, expected, "handshake differs");
    for c in &host[..handshake_len] {
        let exact = written.iter().map(Frame::encode).any(|w| w == c.raw);
        assert!(exact, "handshake transfer {} at {} us not written verbatim", hex(&c.frame), c.ts_us);
    }

    let rpc = captured_requests(&capture, Channel::Rpc.host_id());
    let control = captured_requests(&capture, Channel::Control.host_id());
    assert_eq!((rpc.len(), control.len()), (9, 5));
    let (rpc_results, control_results) = thread::scope(|s| {
        let a = s.spawn(|| drive(&session, Channel::Rpc, rpc));
        let b = s.spawn(|| drive(&session, Channel::Control, control));
        (a.join().unwrap(), b.join().unwrap())
    });
    thread::sleep(ACK_DELAY * 3);

    let written = replay.written();
    for channel in [Channel::Rpc, Channel::Control, Channel::Events] {
        let id = channel.host_id();
        let expected: Vec<String> = host.iter().filter(|c| c.channel == id).map(|c| hex(&c.frame)).collect();
        let got: Vec<String> = written.iter().filter(|f| f.src == id).map(hex).collect();
        assert_eq!(got, expected, "{channel:?}");
    }

    let rpc_results: Vec<rmpv::Value> = rpc_results.into_iter().map(Result::unwrap).collect();
    let control_results: Vec<rmpv::Value> = control_results.into_iter().map(Result::unwrap).collect();
    let read_global_128 = rmpv::Value::Map(vec![(118.into(), 128.into()), (119.into(), 0.into())]);
    assert_eq!(rpc_results[1], read_global_128);
    assert!(rpc_results[3].as_slice().is_some_and(|doc| doc.len() > 2000), "op 22 returns the preset document");
    assert!(control_results[2].as_array().is_some_and(|presets| !presets.is_empty()));
    drop(session);
}

fn first_value(f: &Frame) -> rmpv::Value {
    let payload = f.payload().unwrap_or_default();
    if payload.len() <= 8 {
        return rmpv::Value::Nil;
    }
    rmpv::decode::read_value(&mut &payload[8..]).unwrap_or(rmpv::Value::Nil)
}
