//! The session thread: owns the transport, runs the handshake once, then serves requests, timers and
//! incoming frames until the session is closed or dropped.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Instant;

use nx_proto::{Frame, FrameKind, Reassembler, Request, split_transfer};
use rmpv::Value;

use crate::link::{Link, LinkState, MAX_PAYLOAD};
use crate::{Channel, Config, Error, Event, Transport};

const TXN: u64 = 102;
const STATUS: u64 = 103;
const RESULT: u64 = 104;
const EVENT_ID: u64 = 105;
const EVENT_DATA: u64 = 106;
/// Announces completion of a deferred operation: `{102: txn, 103: status, 104: result}` [seen, 02].
const DEFERRED_DONE: u64 = 20;
const FIRST_TXN: u64 = 1000;

pub(crate) type Reply = Sender<Result<Value, Error>>;

pub(crate) enum Command {
    Request { channel: Channel, request: Request, reply: Reply },
    Subscribe(Sender<Event>, Sender<()>),
    Close(Sender<Result<(), Error>>),
}

struct Job {
    channel: Channel,
    request: Request,
    reply: Reply,
}

struct Inflight {
    txn: u64,
    reply: Reply,
    deadline: Instant,
}

struct Deferred {
    txn: u64,
    reply: Reply,
    deadline: Instant,
}

pub(crate) struct Engine<T: Transport> {
    transport: T,
    config: Config,
    links: [Link; 3],
    reassembler: Reassembler,
    /// Message bytes waiting for the device's window, per channel.
    outbox: [VecDeque<u8>; 3],
    ack_due: [Option<Instant>; 3],
    next_txn: [u64; 3],
    service: [u8; 3],
    /// First message received after a service open, per channel.
    service_reply: [Option<Vec<u8>>; 3],
    /// A channel frame other than HELLO arrived while its HELLO was outstanding.
    stray: [bool; 3],
    handshaken: bool,
    /// Timers (delayed acks, keep-alives, request deadlines) only run once the handshake is done.
    running: bool,
    queue: VecDeque<Job>,
    inflight: [Option<Inflight>; 3],
    deferred: Option<Deferred>,
    subscribers: Vec<Sender<Event>>,
    /// Why the device's state is unknown, once it is. From then on no request goes out on any
    /// channel; acknowledgements and keep-alives continue so the device is not left with unread data.
    poisoned: Option<String>,
}

fn get(map: &[(Value, Value)], key: u64) -> Option<&Value> {
    map.iter().find(|(k, _)| k.as_u64() == Some(key)).map(|(_, v)| v)
}

impl<T: Transport> Engine<T> {
    pub fn new(transport: T, config: Config) -> Self {
        Self {
            transport,
            config,
            links: Channel::ALL.map(Link::new),
            reassembler: Reassembler::default(),
            outbox: Default::default(),
            ack_due: [None; 3],
            next_txn: [FIRST_TXN; 3],
            service: [0; 3],
            service_reply: Default::default(),
            stray: [false; 3],
            handshaken: false,
            running: false,
            queue: VecDeque::new(),
            inflight: Default::default(),
            deferred: None,
            subscribers: Vec::new(),
            poisoned: None,
        }
    }

    fn link(&mut self, channel: Channel) -> &mut Link {
        &mut self.links[channel.index()]
    }

    fn write(&mut self, frame: Frame) -> Result<(), Error> {
        // One frame per transfer, as HX Edit writes them.
        self.transport.write(&frame.encode())?;
        Ok(())
    }

    /// Reads at most one transfer and handles its frames.
    fn pump(&mut self, timeout: std::time::Duration) -> Result<(), Error> {
        let Some(transfer) = self.transport.read(timeout)? else { return Ok(()) };
        // A transfer that does not split is noise from a previous session; there is nothing to
        // answer it with.
        let Ok(frames) = split_transfer(&transfer) else { return Ok(()) };
        for frame in frames {
            self.on_frame(frame)?;
        }
        Ok(())
    }

    fn on_frame(&mut self, frame: Frame) -> Result<(), Error> {
        let Some(channel) = Channel::from_host_id(frame.dst).filter(|c| c.device_id() == frame.src) else {
            return Ok(());
        };
        let now = Instant::now();
        match frame.kind() {
            Some(FrameKind::Hello) => {
                self.link(channel).on_hello(&frame);
            }
            Some(FrameKind::Close) => {
                let link = self.link(channel);
                let was_open = link.state != LinkState::Closing;
                link.state = LinkState::Closed;
                if was_open && self.running {
                    self.on_device_closed(channel);
                }
            }
            _ => {
                let link = &mut self.links[channel.index()];
                if link.state == LinkState::HelloSent && frame.kind() != Some(FrameKind::KeepAlive) {
                    self.stray[channel.index()] = true;
                }
                if matches!(link.state, LinkState::Idle | LinkState::HelloSent | LinkState::Closed) {
                    return Ok(());
                }
                if link.on_frame(&frame) == 0 {
                    return Ok(());
                }
                let messages = self.reassembler.push(&frame).unwrap_or_default();
                for msg in messages {
                    self.on_message(channel, msg.body);
                }
                if !self.running {
                    return Ok(());
                }
                if self.reassembler.buffered(frame.src, frame.dst) > 0 {
                    // The rest of the message is coming; keep the window open for it [seen].
                    let ack = self.link(channel).ack(now);
                    self.ack_due[channel.index()] = None;
                    self.write(ack)?;
                } else if self.ack_due[channel.index()].is_none() {
                    self.ack_due[channel.index()] = Some(now + self.config.ack_delay);
                }
            }
        }
        Ok(())
    }

    fn on_message(&mut self, channel: Channel, body: Vec<u8>) {
        let i = channel.index();
        if self.links[i].state == LinkState::ServiceSent {
            self.links[i].state = LinkState::Open;
            self.service_reply[i] = Some(body);
            return;
        }
        let Ok(value) = rmpv::decode::read_value(&mut &body[..]) else { return };
        let Some(map) = value.as_map() else { return };
        if channel == Channel::Events {
            if let (Some(id), Some(data)) = (get(map, EVENT_ID).and_then(Value::as_u64), get(map, EVENT_DATA)) {
                if id == DEFERRED_DONE {
                    self.on_deferred_done(data);
                }
                self.broadcast(Event::Notification { id, data: data.clone() });
            }
            return;
        }
        let (Some(txn), Some(status)) = (get(map, TXN).and_then(Value::as_u64), get(map, STATUS).and_then(Value::as_u64))
        else {
            return;
        };
        let result = get(map, RESULT).cloned().unwrap_or(Value::Nil);
        // A reply whose request already timed out matches nothing and is dropped.
        let Some(inflight) = self.inflight[i].take_if(|f| f.txn == txn) else { return };
        match status {
            0 => _ = inflight.reply.send(Ok(result)),
            1 => {
                self.deferred =
                    Some(Deferred { txn, reply: inflight.reply, deadline: Instant::now() + self.config.deferred_timeout })
            }
            status => _ = inflight.reply.send(Err(Error::Status { status, result })),
        }
    }

    fn on_deferred_done(&mut self, data: &Value) {
        let Some(map) = data.as_map() else { return };
        let Some(txn) = get(map, TXN).and_then(Value::as_u64) else { return };
        let Some(deferred) = self.deferred.take_if(|d| d.txn == txn) else { return };
        let status = get(map, STATUS).and_then(Value::as_u64).unwrap_or(0);
        let result = get(map, RESULT).cloned().unwrap_or(Value::Nil);
        let outcome = if status == 0 { Ok(result) } else { Err(Error::Status { status, result }) };
        _ = deferred.reply.send(outcome);
    }

    fn on_device_closed(&mut self, channel: Channel) {
        let i = channel.index();
        if let Some(f) = self.inflight[i].take() {
            _ = f.reply.send(Err(Error::ChannelClosed(channel)));
        }
        self.outbox[i].clear();
        self.ack_due[i] = None;
        self.broadcast(Event::ChannelClosed(channel));
    }

    fn broadcast(&mut self, event: Event) {
        self.subscribers.retain(|s| s.send(event.clone()).is_ok());
    }

    fn wait_for(&mut self, what: &'static str, mut done: impl FnMut(&Self) -> bool) -> Result<(), Error> {
        let deadline = Instant::now() + self.config.handshake_timeout;
        while !done(self) {
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout(what));
            }
            self.pump((deadline - now).min(self.config.poll))?;
        }
        Ok(())
    }

    fn send_hello(&mut self, channel: Channel) -> Result<(), Error> {
        let Some(hello) = self.link(channel).hello(Instant::now()) else { return Err(Error::AlreadyOpen(channel)) };
        self.stray[channel.index()] = false;
        self.write(hello)?;
        let i = channel.index();
        self.wait_for("HELLO", |e| e.links[i].state != LinkState::HelloSent || e.stray[i])?;
        if self.links[channel.index()].state != LinkState::Hello {
            // A wedged device answers HELLO with bare acknowledgements carrying old sequence
            // numbers; only a power cycle helps [map].
            return Err(Error::Handshake { channel, reason: "device answered HELLO without a HELLO".into() });
        }
        Ok(())
    }

    fn open_service(&mut self, channel: Channel, service: u8) -> Result<(), Error> {
        let i = channel.index();
        self.service[i] = service;
        self.links[i].state = LinkState::ServiceSent;
        self.service_reply[i] = None;
        let msg = message(service, &[service]);
        self.queue_bytes(channel, &msg);
        self.flush_outbox(channel)?;
        self.wait_for("service open", |e| e.service_reply[i].is_some())?;
        let reply = self.service_reply[i].as_deref().unwrap_or_default();
        if reply.first() != Some(&service) {
            return Err(Error::Handshake { channel, reason: format!("service {service} answered with {reply:02x?}") });
        }
        Ok(())
    }

    fn send_ack(&mut self, channel: Channel) -> Result<(), Error> {
        let ack = self.link(channel).ack(Instant::now());
        self.ack_due[channel.index()] = None;
        self.write(ack)
    }

    fn close_channel(&mut self, channel: Channel) -> Result<(), Error> {
        let close = self.link(channel).close(Instant::now());
        self.write(close)?;
        self.wait_for("close", |e| e.links[channel.index()].state == LinkState::Closed)
    }

    /// Drops whatever an earlier client left queued in the device, so the HELLO replies are ours.
    fn drain(&mut self) -> Result<(), Error> {
        let deadline = Instant::now() + self.config.drain_limit;
        while self.transport.read(self.config.poll * 4)?.is_some() {
            if Instant::now() >= deadline {
                return Err(Error::Backlog);
            }
        }
        Ok(())
    }

    /// HX Edit's connect from `06-reconnect`, frame for frame: probe the control channel with
    /// service 5 and close it, then open RPC (6), events (4) and control again (2). Each channel's
    /// service reply is acknowledged after the next channel's HELLO, as HX Edit does. Any failure
    /// is final: the handshake is never repeated.
    pub fn handshake(&mut self) -> Result<Vec<u8>, Error> {
        if self.handshaken {
            return Err(Error::AlreadyOpen(Channel::Control));
        }
        self.handshaken = true;
        self.drain()?;
        self.send_hello(Channel::Control)?;
        self.open_service(Channel::Control, 5)?;
        let info = self.service_reply[Channel::Control.index()].clone().unwrap_or_default();
        self.send_ack(Channel::Control)?;
        let linger = Instant::now() + self.config.probe_linger;
        while Instant::now() < linger {
            self.pump(self.config.poll.min(linger - Instant::now()))?;
        }
        self.close_channel(Channel::Control)?;

        let order = [(Channel::Rpc, 6), (Channel::Events, 4), (Channel::Control, 2)];
        let mut previous: Option<Channel> = None;
        for (channel, service) in order {
            self.send_hello(channel)?;
            if let Some(p) = previous {
                self.send_ack(p)?;
            }
            self.open_service(channel, service)?;
            previous = Some(channel);
        }
        self.send_ack(Channel::Control)?;
        self.running = true;
        Ok(info)
    }

    pub fn run(mut self, commands: Receiver<Command>) {
        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Request { channel, request, reply }) => self.queue.push_back(Job { channel, request, reply }),
                    Ok(Command::Subscribe(s, done)) => {
                        self.subscribers.push(s);
                        _ = done.send(());
                    }
                    Ok(Command::Close(done)) => {
                        let outcome = self.close();
                        self.fail_all(Error::SessionClosed);
                        _ = done.send(outcome);
                        return;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        _ = self.flush_acks();
                        self.fail_all(Error::SessionClosed);
                        return;
                    }
                }
            }
            if let Err(e) = self.step() {
                self.fail_all(e);
                return;
            }
        }
    }

    fn step(&mut self) -> Result<(), Error> {
        self.start_jobs()?;
        for channel in Channel::ALL {
            self.flush_outbox(channel)?;
        }
        self.timers()?;
        self.pump(self.config.poll)
    }

    /// One request in flight per channel, and nothing at all while a deferred operation runs: HX
    /// Edit waits for notification 20 before its next operation, and a device that is raced by a
    /// new commit before the last one finished stops accepting writes [map].
    fn start_jobs(&mut self) -> Result<(), Error> {
        let mut i = 0;
        while i < self.queue.len() {
            if self.deferred.is_some() {
                return Ok(());
            }
            let channel = self.queue[i].channel;
            let c = channel.index();
            if self.inflight[c].is_some() {
                i += 1;
                continue;
            }
            let job = self.queue.remove(i).expect("index is in range");
            if let Some(reason) = &self.poisoned {
                _ = job.reply.send(Err(Error::Poisoned(reason.clone())));
                continue;
            }
            if self.links[c].state != LinkState::Open || channel == Channel::Events {
                _ = job.reply.send(Err(Error::ChannelClosed(channel)));
                continue;
            }
            let txn = self.next_txn[c];
            self.next_txn[c] += 1;
            let msg = message(self.service[c], &job.request.encode(txn));
            self.queue_bytes(channel, &msg);
            self.inflight[c] = Some(Inflight { txn, reply: job.reply, deadline: Instant::now() + self.config.reply_timeout });
            self.flush_outbox(channel)?;
        }
        Ok(())
    }

    fn queue_bytes(&mut self, channel: Channel, msg: &[u8]) {
        self.outbox[channel.index()].extend(msg);
    }

    fn flush_outbox(&mut self, channel: Channel) -> Result<(), Error> {
        let i = channel.index();
        loop {
            let n = self.outbox[i].len().min(MAX_PAYLOAD).min(self.links[i].credit());
            if n == 0 {
                return Ok(());
            }
            let chunk: Vec<u8> = self.outbox[i].drain(..n).collect();
            let frame = self.links[i].data(&chunk, Instant::now());
            self.ack_due[i] = None;
            self.write(frame)?;
        }
    }

    fn timers(&mut self) -> Result<(), Error> {
        let now = Instant::now();
        for channel in Channel::ALL {
            let i = channel.index();
            if self.ack_due[i].is_some_and(|t| now >= t) {
                self.ack_due[i] = None;
                if self.links[i].owes_ack() {
                    self.send_ack(channel)?;
                }
            }
            let link = &self.links[i];
            let idle = link.last_sent.is_none_or(|t| now.duration_since(t) >= self.config.keepalive_interval);
            if link.state == LinkState::Open && idle {
                // The device does not count a keep-alive as an acknowledgement [map].
                let frame = if link.owes_ack() { self.links[i].ack(now) } else { self.links[i].keepalive(now) };
                self.ack_due[i] = None;
                self.write(frame)?;
            }
            if self.inflight[i].as_ref().is_some_and(|f| now >= f.deadline) {
                let f = self.inflight[i].take().expect("checked above");
                _ = f.reply.send(Err(Error::Timeout("reply")));
                // The reply may still come, as status 1 of an operation the device then commits.
                self.poison(format!("a request on {channel:?} timed out"));
            }
        }
        if self.deferred.as_ref().is_some_and(|d| now >= d.deadline) {
            let d = self.deferred.take().expect("checked above");
            _ = d.reply.send(Err(Error::Timeout("notification 20")));
            self.poison("a deferred operation timed out".into());
        }
        Ok(())
    }

    /// Marks the device's state as unknown: everything pending fails, unsent request bytes are
    /// dropped, and no request goes out again in this session.
    fn poison(&mut self, reason: String) {
        if self.poisoned.is_some() {
            return;
        }
        for outbox in &mut self.outbox {
            outbox.clear();
        }
        self.poisoned = Some(reason.clone());
        self.fail_all(Error::Poisoned(reason));
    }

    fn flush_acks(&mut self) -> Result<(), Error> {
        for channel in Channel::ALL {
            let link = &self.links[channel.index()];
            if link.state == LinkState::Open && link.owes_ack() {
                self.send_ack(channel)?;
            }
        }
        Ok(())
    }

    /// HX Edit's quit in `06-reconnect`: control, then events, each answered by the device; the
    /// RPC channel was left open and the next connect opened it again without trouble [seen].
    fn close(&mut self) -> Result<(), Error> {
        // Let notifications that are already on their way arrive and be acknowledged.
        let settle = Instant::now() + self.config.ack_delay;
        while Instant::now() < settle {
            self.pump(self.config.poll)?;
        }
        self.flush_acks()?;
        for channel in [Channel::Control, Channel::Events] {
            if self.links[channel.index()].state == LinkState::Open {
                self.close_channel(channel)?;
            }
        }
        Ok(())
    }

    fn fail_all(&mut self, error: Error) {
        for f in self.inflight.iter_mut().filter_map(Option::take) {
            _ = f.reply.send(Err(error.clone()));
        }
        if let Some(d) = self.deferred.take() {
            _ = d.reply.send(Err(error.clone()));
        }
        for job in self.queue.drain(..) {
            _ = job.reply.send(Err(error.clone()));
        }
    }
}

/// A host message: originator 1, the channel's service, MessagePack length, body.
fn message(service: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&u16::from(service).to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

#[cfg(all(test, feature = "fake"))]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::fake::FakeDevice;

    fn config() -> Config {
        Config { probe_linger: Duration::from_millis(1), ..Config::default() }
    }

    #[test]
    fn the_handshake_runs_once_per_session() {
        let device = FakeDevice::new();
        let mut engine = Engine::new(device.transport(), config());
        engine.handshake().unwrap();
        let hellos = device.hellos().len();
        assert!(matches!(engine.handshake(), Err(Error::AlreadyOpen(_))));
        assert_eq!(device.hellos().len(), hellos);
    }

    #[test]
    fn a_failed_handshake_is_not_tried_again() {
        let device = FakeDevice::new();
        device.ignore_hello(true);
        let mut engine = Engine::new(device.transport(), Config { handshake_timeout: Duration::from_millis(20), ..config() });
        assert!(matches!(engine.handshake(), Err(Error::Timeout(_))));
        device.ignore_hello(false);
        assert!(matches!(engine.handshake(), Err(Error::AlreadyOpen(_))));
        assert_eq!(device.hellos(), vec![0x1001]);
    }

    #[test]
    fn no_hello_on_an_open_channel() {
        let device = FakeDevice::new();
        let mut engine = Engine::new(device.transport(), config());
        engine.handshake().unwrap();
        assert!(matches!(engine.send_hello(Channel::Rpc), Err(Error::AlreadyOpen(Channel::Rpc))));
        assert_eq!(device.hellos().len(), 4);
    }
}
