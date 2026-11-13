//! Transport frames on the `MI_00` bulk endpoints, as observed in `local/captures/` (session 1).
//!
//! ```text
//! 0  u16 LE  body length (bytes after this 8-byte header)
//! 2  u8      always 0
//! 3  u8      header format: 0x18 for channel frames, 0x28 for HELLO
//! 4  u16 LE  source endpoint id   (host 0x1001/0x1002/0x1080, device 0x03ef/0x03f0/0x03ed)
//! 6  u16 LE  destination endpoint id
//! 8  body; for 0x18: u8 0, u8 sequence, u8 0, u8 kind, u32 LE acknowledgement, payload
//! ```
//!
//! The sequence is a single byte: it wraps from 0xff to 0x00 in `04` and `05`.
//!
//! A bulk transfer carries one or more frames, each padded to a multiple of 4 bytes.

use crate::ProtoError;

pub const FORMAT_CHANNEL: u8 = 0x18;
pub const FORMAT_HELLO: u8 = 0x28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Hello,
    /// Kind 0x02 on a channel frame: ends a channel; the device answers with its own.
    Close,
    Data,
    Ack,
    DataAck,
    KeepAlive,
}

impl FrameKind {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0x02 => Self::Close,
            0x04 => Self::Data,
            0x08 => Self::Ack,
            0x0c => Self::DataAck,
            0x10 => Self::KeepAlive,
            _ => return None,
        })
    }

    pub fn byte(self) -> u8 {
        match self {
            Self::Hello | Self::Close => 0x02,
            Self::Data => 0x04,
            Self::Ack => 0x08,
            Self::DataAck => 0x0c,
            Self::KeepAlive => 0x10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub format: u8,
    pub src: u16,
    pub dst: u16,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn channel(src: u16, dst: u16, seq: u8, kind: FrameKind, ack: u32, payload: &[u8]) -> Self {
        let mut body = Vec::with_capacity(8 + payload.len());
        body.extend_from_slice(&[0, seq, 0, kind.byte()]);
        body.extend_from_slice(&ack.to_le_bytes());
        body.extend_from_slice(payload);
        Self { format: FORMAT_CHANNEL, src, dst, body }
    }

    pub fn hello(src: u16, dst: u16, body: &[u8]) -> Self {
        Self { format: FORMAT_HELLO, src, dst, body: body.to_vec() }
    }

    pub fn kind(&self) -> Option<FrameKind> {
        if self.format == FORMAT_HELLO {
            return Some(FrameKind::Hello);
        }
        FrameKind::from_byte(*self.body.get(3)?)
    }

    pub fn seq(&self) -> Option<u8> {
        self.channel_body().map(|b| b[1])
    }

    pub fn ack(&self) -> Option<u32> {
        self.channel_body().map(|b| u32::from_le_bytes([b[4], b[5], b[6], b[7]]))
    }

    /// Stream bytes carried by a channel frame (empty for keep-alives and acks).
    pub fn payload(&self) -> Option<&[u8]> {
        self.channel_body().map(|b| &b[8..])
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(align4(8 + self.body.len()));
        out.extend_from_slice(&(self.body.len() as u16).to_le_bytes());
        out.push(0);
        out.push(self.format);
        out.extend_from_slice(&self.src.to_le_bytes());
        out.extend_from_slice(&self.dst.to_le_bytes());
        out.extend_from_slice(&self.body);
        out.resize(align4(out.len()), 0);
        out
    }

    fn channel_body(&self) -> Option<&[u8]> {
        (self.format == FORMAT_CHANNEL && self.body.len() >= 8).then_some(&self.body[..])
    }
}

fn align4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

pub fn split_transfer(data: &[u8]) -> Result<Vec<Frame>, ProtoError> {
    match split_leading(data) {
        (frames, None) => Ok(frames),
        (_, Some(e)) => Err(e),
    }
}

/// The frames of a transfer up to the first that does not fit, and the error that stopped there.
pub fn split_leading(data: &[u8]) -> (Vec<Frame>, Option<ProtoError>) {
    let mut frames = Vec::new();
    let mut at = 0;
    while data.len() - at >= 8 {
        let header = &data[at..at + 8];
        let len = u16::from_le_bytes([header[0], header[1]]) as usize;
        let end = at + 8 + len;
        if end > data.len() {
            return (frames, Some(ProtoError::Truncated { offset: at, need: 8 + len, have: data.len() - at }));
        }
        frames.push(Frame {
            format: header[3],
            src: u16::from_le_bytes([header[4], header[5]]),
            dst: u16::from_le_bytes([header[6], header[7]]),
            body: data[at + 8..end].to_vec(),
        });
        at = align4(end);
    }
    (frames, None)
}
