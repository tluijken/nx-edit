//! A scripted HX Stomp: answers HELLO, service opens, closes and RPC the way the captures show,
//! acknowledges what it receives, and holds back notifications beyond 4 KB unacknowledged as the
//! real device does [map]. For tests, and for working on the editor without the pedal.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nx_proto::{Frame, FrameKind, Reassembler, Request, split_transfer};
use rmpv::Value;

use crate::{Channel, Transport};

/// The device's HELLO answer from `06-reconnect`; it announces a base (and window) of 0x200.
const DEVICE_HELLO: [u8; 12] = [0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00];
const DEVICE_BASE: u32 = 0x200;
const HOST_BASE: u32 = 0x1000;
const EVENTS_WINDOW: u32 = 4096;

/// How the device answers one request.
#[derive(Debug, Clone)]
pub enum Reply {
    /// Status 0 with this result.
    Ok(Value),
    /// Any other status with this result.
    Status(u64, Value),
    /// Status 1 now; completion follows when the test calls [`FakeDevice::complete_deferred`].
    Deferred,
    /// No answer at all.
    Silent,
    /// Status 0 with the transaction id as result, after this delay.
    Late(Duration),
    /// Status 1 (accepted, deferred) after this delay.
    LateDeferred(Duration),
    /// Notification 20 with status 0 and this result, and only then the status 1 reply.
    DoneBeforeReply(Value),
}

type Handler = Box<dyn FnMut(Channel, u64, &Request) -> Reply + Send>;

#[derive(Default)]
struct DeviceLink {
    open: bool,
    awaiting_service: bool,
    service: u8,
    seq: u8,
    received: u32,
    advertised: u32,
    sent: u32,
    host_acked: u32,
}

struct State {
    links: [DeviceLink; 3],
    reassembler: Reassembler,
    out: VecDeque<(Vec<u8>, bool)>,
    scheduled: Vec<(Instant, Channel, Vec<u8>)>,
    events: VecDeque<Vec<u8>>,
    handler: Handler,
    host_frames: Vec<Frame>,
    hellos: Vec<u16>,
    closes: Vec<u16>,
    service_opens: Vec<(u16, u8)>,
    requests: Vec<(Channel, u64, Request)>,
    max_unacked_from_host: u32,
    ignore_hello: bool,
    wedged: bool,
    withhold_acks: bool,
    refused_services: Vec<u8>,
    ignored_closes: Vec<Channel>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Clone)]
pub struct FakeDevice(Arc<Shared>);

/// The host's side of the fake's USB interface.
pub struct FakeTransport(Arc<Shared>);

fn index(channel: Channel) -> usize {
    channel as usize
}

fn map(pairs: Vec<(u64, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (Value::from(k), v)).collect())
}

fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    rmpv::encode::write_value(&mut out, value).expect("writing to a Vec cannot fail");
    out
}

impl Default for FakeDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeDevice {
    pub fn new() -> Self {
        let state = State {
            links: Default::default(),
            reassembler: Reassembler::default(),
            out: VecDeque::new(),
            scheduled: Vec::new(),
            events: VecDeque::new(),
            handler: Box::new(|_, _, _| Reply::Ok(Value::Nil)),
            host_frames: Vec::new(),
            hellos: Vec::new(),
            closes: Vec::new(),
            service_opens: Vec::new(),
            requests: Vec::new(),
            max_unacked_from_host: 0,
            ignore_hello: false,
            wedged: false,
            withhold_acks: false,
            refused_services: Vec::new(),
            ignored_closes: Vec::new(),
        };
        Self(Arc::new(Shared { state: Mutex::new(state), wake: Condvar::new() }))
    }

    pub fn transport(&self) -> FakeTransport {
        FakeTransport(self.0.clone())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap()
    }

    fn changed(&self) {
        self.0.wake.notify_all();
    }

    pub fn on_request(&self, handler: impl FnMut(Channel, u64, &Request) -> Reply + Send + 'static) {
        self.lock().handler = Box::new(handler);
    }

    pub fn ignore_hello(&self, on: bool) {
        self.lock().ignore_hello = on;
    }

    /// Stops acknowledging host data with bare ACKs, so the host's window fills up. Turning it off
    /// acknowledges everything received at once.
    pub fn withhold_acks(&self, on: bool) {
        let mut st = self.lock();
        st.withhold_acks = on;
        if !on {
            for channel in Channel::ALL {
                let l = &st.links[index(channel)];
                if l.open && l.received != l.advertised {
                    let f = st.frame(channel, FrameKind::Ack, &[]);
                    st.out.push_back((f.encode(), false));
                }
            }
        }
        drop(st);
        self.changed();
    }

    /// Leaves opens of `service` unanswered.
    pub fn refuse_service(&self, service: u8) {
        self.lock().refused_services.push(service);
    }

    /// Leaves the host's close of `channel` unanswered (it is still recorded).
    pub fn ignore_close(&self, channel: Channel) {
        self.lock().ignored_closes.push(channel);
    }

    /// Answers HELLO with a bare acknowledgement, like a device left wedged by an earlier client.
    pub fn wedged(&self, on: bool) {
        self.lock().wedged = on;
    }

    /// Queues stale keep-alives from an earlier session before anything is connected.
    pub fn backlog(&self, frames: usize) {
        let mut st = self.lock();
        for seq in 0..frames {
            let f = Frame::channel(0x03ef, 0x1001, 0x80u8.wrapping_add(seq as u8), FrameKind::KeepAlive, 0x265, &[]);
            st.out.push_back((f.encode(), true));
        }
    }

    pub fn backlog_left(&self) -> usize {
        self.lock().out.iter().filter(|(_, stale)| *stale).count()
    }

    pub fn notify(&self, id: u64, data: Value) {
        let body = encode(&map(vec![(105, Value::from(id)), (106, data)]));
        let mut st = self.lock();
        st.events.push_back(body);
        st.send_events();
        drop(st);
        self.changed();
    }

    /// Notification 20 for a deferred operation.
    pub fn complete_deferred(&self, txn: u64, status: u64, result: Value) {
        self.notify(20, map(vec![(102, Value::from(txn)), (103, Value::from(status)), (104, result)]));
    }

    pub fn close_channel(&self, channel: Channel) {
        let mut st = self.lock();
        let f = st.frame(channel, FrameKind::Close, &[]);
        st.links[index(channel)].open = false;
        st.out.push_back((f.encode(), false));
        drop(st);
        self.changed();
    }

    pub fn hellos(&self) -> Vec<u16> {
        self.lock().hellos.clone()
    }

    pub fn closes(&self) -> Vec<u16> {
        self.lock().closes.clone()
    }

    pub fn service_opens(&self) -> Vec<(u16, u8)> {
        self.lock().service_opens.clone()
    }

    pub fn requests(&self) -> Vec<(Channel, u64, Request)> {
        self.lock().requests.clone()
    }

    pub fn host_frames(&self, channel: Channel) -> Vec<Frame> {
        self.lock().host_frames.iter().filter(|f| f.src == channel.host_id()).cloned().collect()
    }

    /// Notification bytes the host has not acknowledged with an ACK frame.
    pub fn events_unacked(&self) -> u32 {
        let st = self.lock();
        let l = &st.links[index(Channel::Events)];
        l.sent.wrapping_sub(l.host_acked)
    }

    /// The acknowledgement a host that kept up would now send on `channel`.
    pub fn expected_ack(&self, channel: Channel) -> u32 {
        HOST_BASE.wrapping_add(self.lock().links[index(channel)].sent)
    }

    /// The most host bytes ever received beyond what the device had acknowledged.
    pub fn max_unacked_from_host(&self) -> u32 {
        self.lock().max_unacked_from_host
    }
}

impl State {
    fn frame(&mut self, channel: Channel, kind: FrameKind, payload: &[u8]) -> Frame {
        let l = &mut self.links[index(channel)];
        let f = Frame::channel(channel.device_id(), channel.host_id(), l.seq, kind, DEVICE_BASE + l.received, payload);
        l.seq = l.seq.wrapping_add(1);
        l.advertised = l.received;
        l.sent = l.sent.wrapping_add(payload.len() as u32);
        f
    }

    fn send_message(&mut self, channel: Channel, body: &[u8]) {
        let mut msg = Vec::with_capacity(8 + body.len());
        msg.extend_from_slice(&0u16.to_le_bytes());
        msg.extend_from_slice(&u16::from(self.links[index(channel)].service).to_le_bytes());
        msg.extend_from_slice(&(body.len() as u32).to_le_bytes());
        msg.extend_from_slice(body);
        for chunk in msg.chunks(256) {
            let f = self.frame(channel, FrameKind::Data, chunk);
            self.out.push_back((f.encode(), false));
        }
    }

    fn send_events(&mut self) {
        let e = index(Channel::Events);
        while self.links[e].open
            && let Some(body) = self.events.front()
        {
            let unacked = self.links[e].sent.wrapping_sub(self.links[e].host_acked);
            if unacked + 8 + body.len() as u32 > EVENTS_WINDOW {
                return;
            }
            let body = self.events.pop_front().expect("checked above");
            self.send_message(Channel::Events, &body);
        }
    }

    fn release_due(&mut self, now: Instant) {
        let (due, later): (Vec<_>, Vec<_>) = self.scheduled.drain(..).partition(|(t, _, _)| *t <= now);
        self.scheduled = later;
        for (_, channel, body) in due {
            self.send_message(channel, &body);
        }
    }

    fn on_host_frame(&mut self, frame: Frame) {
        self.host_frames.push(frame.clone());
        let Some(channel) = Channel::from_host_id(frame.src).filter(|c| c.device_id() == frame.dst) else { return };
        let i = index(channel);
        match frame.kind() {
            Some(FrameKind::Hello) => {
                self.hellos.push(frame.src);
                if self.ignore_hello {
                    return;
                }
                if self.wedged {
                    let f = Frame::channel(channel.device_id(), channel.host_id(), 0x86, FrameKind::Ack, 0x265, &[]);
                    self.out.push_back((f.encode(), false));
                    return;
                }
                self.links[i] = DeviceLink { open: true, awaiting_service: true, seq: 2, ..Default::default() };
                let f = Frame::hello(channel.device_id(), channel.host_id(), &DEVICE_HELLO);
                self.out.push_back((f.encode(), false));
            }
            Some(FrameKind::Close) => {
                self.closes.push(frame.src);
                if self.ignored_closes.contains(&channel) {
                    return;
                }
                let f = self.frame(channel, FrameKind::Close, &[]);
                self.links[i].open = false;
                self.out.push_back((f.encode(), false));
            }
            Some(kind) => {
                if !self.links[i].open {
                    return;
                }
                // Only ACK frames count as acknowledgement of notifications [map]; keep-alives
                // carry the same field but the device ignores it.
                let counts = channel != Channel::Events || matches!(kind, FrameKind::Ack | FrameKind::DataAck);
                if counts && let Some(ack) = frame.ack() {
                    self.links[i].host_acked = ack.wrapping_sub(HOST_BASE);
                }
                let payload = frame.payload().unwrap_or_default();
                if !payload.is_empty() {
                    let l = &mut self.links[i];
                    l.received = l.received.wrapping_add(payload.len() as u32);
                    self.max_unacked_from_host = self.max_unacked_from_host.max(l.received - l.advertised);
                    let before = self.out.len();
                    for msg in self.reassembler.push(&frame).unwrap_or_default() {
                        self.on_host_message(channel, &msg.body);
                    }
                    if self.out.len() == before && !self.withhold_acks {
                        let f = self.frame(channel, FrameKind::Ack, &[]);
                        self.out.push_back((f.encode(), false));
                    }
                }
                if channel == Channel::Events {
                    self.send_events();
                }
            }
            None => {}
        }
    }

    fn on_host_message(&mut self, channel: Channel, body: &[u8]) {
        let i = index(channel);
        if self.links[i].awaiting_service {
            let service = body.first().copied().unwrap_or_default();
            self.links[i].awaiting_service = false;
            self.links[i].service = service;
            self.service_opens.push((channel.host_id(), service));
            if self.refused_services.contains(&service) {
                return;
            }
            self.send_message(channel, &[service]);
            return;
        }
        if channel == Channel::Events {
            return;
        }
        let Ok(value) = rmpv::decode::read_value(&mut &body[..]) else { return };
        let Some((txn, request)) = Request::from_value(&value) else { return };
        self.requests.push((channel, txn, request.clone()));
        let reply = |status: u64, result: Value| {
            encode(&map(vec![(102, Value::from(txn)), (103, Value::from(status)), (104, result)]))
        };
        match (self.handler)(channel, txn, &request) {
            Reply::Ok(v) => self.send_message(channel, &reply(0, v)),
            Reply::Status(s, v) => self.send_message(channel, &reply(s, v)),
            Reply::Deferred => self.send_message(channel, &reply(1, Value::Nil)),
            Reply::Silent => {}
            Reply::Late(after) => self.scheduled.push((Instant::now() + after, channel, reply(0, Value::from(txn)))),
            Reply::LateDeferred(after) => self.scheduled.push((Instant::now() + after, channel, reply(1, Value::Nil))),
            Reply::DoneBeforeReply(result) => {
                let done = map(vec![(102, Value::from(txn)), (103, Value::from(0u64)), (104, result)]);
                self.events.push_back(encode(&map(vec![(105, Value::from(20u64)), (106, done)])));
                self.send_events();
                self.send_message(channel, &reply(1, Value::Nil));
            }
        }
    }
}

impl Transport for FakeTransport {
    fn write(&mut self, transfer: &[u8]) -> io::Result<()> {
        let frames = split_transfer(transfer).map_err(|e| io::Error::other(e.to_string()))?;
        let mut st = self.0.state.lock().unwrap();
        for f in frames {
            st.on_host_frame(f);
        }
        drop(st);
        self.0.wake.notify_all();
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        let mut st = self.0.state.lock().unwrap();
        loop {
            let now = Instant::now();
            st.release_due(now);
            if let Some((t, _)) = st.out.pop_front() {
                return Ok(Some(t));
            }
            if now >= deadline {
                return Ok(None);
            }
            st = self.0.wake.wait_timeout(st, deadline - now).unwrap().0;
        }
    }
}
