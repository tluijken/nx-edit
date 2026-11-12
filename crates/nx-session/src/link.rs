//! Per-channel transport state: sequence numbers, acknowledgements and the device's window.
//!
//! The acknowledgement in every frame is the sender's receive limit: the base it announced in its
//! HELLO plus the payload bytes it has received on the channel. The host announces 0x1000, the
//! HX Stomp 0x200, and since the device stops after 4 KB of unacknowledged notifications [map]
//! the base doubles as the window: a side may send up to the limit its peer last acknowledged.

use std::time::Instant;

use nx_proto::{Frame, FrameKind};

use crate::Channel;

/// HX Edit's HELLO body, byte for byte from `06-reconnect`; the last word is the host's base.
pub(crate) const HOST_HELLO: [u8; 12] = [0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x21, 0x00, 0x10, 0x00, 0x00];
const HOST_BASE: u32 = 0x1000;

/// Largest payload per frame; the device never sends more and HX Edit never needed more.
pub(crate) const MAX_PAYLOAD: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkState {
    Idle,
    HelloSent,
    /// HELLO answered, service not yet requested.
    Hello,
    ServiceSent,
    Open,
    Closing,
    Closed,
}

#[derive(Debug)]
pub(crate) struct Link {
    pub channel: Channel,
    pub state: LinkState,
    seq: u8,
    received: u32,
    acked: u32,
    sent: u32,
    peer_limit: u32,
    pub last_sent: Option<Instant>,
}

impl Link {
    pub fn new(channel: Channel) -> Self {
        Self {
            channel,
            state: LinkState::Idle,
            seq: 0,
            received: 0,
            acked: HOST_BASE,
            sent: 0,
            peer_limit: 0,
            last_sent: None,
        }
    }

    /// Only a channel that was never opened, or was closed, may get a HELLO: a second HELLO on an
    /// open channel looks like a second client to the device and can wedge it [map].
    pub fn hello(&mut self, now: Instant) -> Option<Frame> {
        if !matches!(self.state, LinkState::Idle | LinkState::Closed) {
            return None;
        }
        *self = Self::new(self.channel);
        self.state = LinkState::HelloSent;
        // HELLO is seq 0 and the first frame after it is seq 2, on both sides [seen].
        self.seq = 2;
        self.last_sent = Some(now);
        Some(Frame::hello(self.channel.host_id(), self.channel.device_id(), &HOST_HELLO))
    }

    pub fn on_hello(&mut self, frame: &Frame) -> bool {
        if self.state != LinkState::HelloSent || frame.body.len() < 12 {
            return false;
        }
        self.peer_limit = u32::from_le_bytes(frame.body[8..12].try_into().unwrap());
        self.state = LinkState::Hello;
        true
    }

    /// Records a channel frame from the device; returns its payload length.
    pub fn on_frame(&mut self, frame: &Frame) -> usize {
        if let Some(ack) = frame.ack()
            && ack.wrapping_sub(self.peer_limit) < 0x8000_0000
        {
            self.peer_limit = ack;
        }
        let n = frame.payload().map_or(0, <[u8]>::len);
        self.received = self.received.wrapping_add(n as u32);
        n
    }

    pub fn ack_value(&self) -> u32 {
        HOST_BASE.wrapping_add(self.received)
    }

    pub fn owes_ack(&self) -> bool {
        self.ack_value() != self.acked
    }

    /// Payload bytes the device's last acknowledgement still allows.
    pub fn credit(&self) -> usize {
        let left = self.peer_limit.wrapping_sub(self.sent);
        if left >= 0x8000_0000 { 0 } else { left as usize }
    }

    fn frame(&mut self, kind: FrameKind, payload: &[u8], now: Instant) -> Frame {
        let ack = self.ack_value();
        let f = Frame::channel(self.channel.host_id(), self.channel.device_id(), self.seq, kind, ack, payload);
        self.seq = self.seq.wrapping_add(1);
        self.acked = ack;
        self.last_sent = Some(now);
        f
    }

    /// A data frame; it is marked as carrying an acknowledgement only when that is news [seen].
    pub fn data(&mut self, payload: &[u8], now: Instant) -> Frame {
        let kind = if self.owes_ack() { FrameKind::DataAck } else { FrameKind::Data };
        self.sent = self.sent.wrapping_add(payload.len() as u32);
        self.frame(kind, payload, now)
    }

    pub fn ack(&mut self, now: Instant) -> Frame {
        self.frame(FrameKind::Ack, &[], now)
    }

    pub fn keepalive(&mut self, now: Instant) -> Frame {
        self.frame(FrameKind::KeepAlive, &[], now)
    }

    pub fn close(&mut self, now: Instant) -> Frame {
        self.state = LinkState::Closing;
        self.frame(FrameKind::Close, &[], now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_frame(seq: u8, kind: FrameKind, ack: u32, payload: &[u8]) -> Frame {
        Frame::channel(0x03ed, 0x1080, seq, kind, ack, payload)
    }

    fn device_hello() -> Frame {
        Frame::hello(0x03ed, 0x1080, &[0, 0, 0, 2, 0, 1, 0, 1, 0, 2, 0, 0])
    }

    #[test]
    fn follows_hx_edits_numbering_on_0x1080() {
        // 06-reconnect, 29.11 s onwards.
        let now = Instant::now();
        let mut link = Link::new(Channel::Rpc);
        assert_eq!(link.hello(now).unwrap().encode(), hex("0c0000288010ed03000000020001002100100000"));
        assert!(link.on_hello(&device_hello()));
        let svc = link.data(&[1, 0, 6, 0, 1, 0, 0, 0, 6], now);
        assert_eq!(svc.encode(), hex("110000188010ed030002000400100000010006000100000006000000"));
        link.on_frame(&device_frame(2, FrameKind::Data, 0x209, &[0, 0, 6, 0, 1, 0, 0, 0, 6]));
        assert_eq!(link.ack(now).encode(), hex("080000188010ed030003000809100000"));
        assert!(!link.owes_ack());
        let req = link.data(&hex("010006000900000083 66cd03e8644c6580".replace(' ', "").as_str()), now);
        assert_eq!(req.kind(), Some(FrameKind::Data));
        assert_eq!(req.seq(), Some(4));
        link.on_frame(&device_frame(4, FrameKind::Data, 0x21a, &[0; 0x4c]));
        let next = link.data(&[0; 17], now);
        assert_eq!((next.kind(), next.ack()), (Some(FrameKind::DataAck), Some(0x1055)));
    }

    #[test]
    fn refuses_a_second_hello_on_an_open_channel() {
        let now = Instant::now();
        let mut link = Link::new(Channel::Rpc);
        link.hello(now).unwrap();
        assert!(link.hello(now).is_none());
        link.on_hello(&device_hello());
        link.state = LinkState::Open;
        assert!(link.hello(now).is_none());
        link.close(now);
        assert!(link.hello(now).is_none(), "not before the device answered the close");
        link.state = LinkState::Closed;
        assert!(link.hello(now).is_some());
    }

    #[test]
    fn credit_is_the_devices_window() {
        let now = Instant::now();
        let mut link = Link::new(Channel::Rpc);
        link.hello(now);
        link.on_hello(&device_hello());
        assert_eq!(link.credit(), 0x200);
        link.data(&[0; 0x100], now);
        assert_eq!(link.credit(), 0x100);
        link.data(&[0; 0x100], now);
        assert_eq!(link.credit(), 0);
        link.on_frame(&device_frame(2, FrameKind::Ack, 0x300, &[]));
        assert_eq!(link.credit(), 0x100);
        link.on_frame(&device_frame(3, FrameKind::Ack, 0x2ff, &[]));
        assert_eq!(link.credit(), 0x100, "a stale acknowledgement does not shrink the window");
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
}
