//! Messages carried in the payload stream of a channel: an 8-byte header followed by MessagePack.
//!
//! ```text
//! 0  u16 LE  originator: 1 host, 0 device
//! 2  u16 LE  service id (unreliable on some device replies)
//! 4  u32 LE  MessagePack body length
//! 8  MessagePack body
//! ```
//!
//! A message may span several frames; each direction of each channel is its own stream.

use std::collections::HashMap;

use crate::{Frame, ProtoError};

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub src: u16,
    pub dst: u16,
    pub from_host: bool,
    pub service: u16,
    pub body: Vec<u8>,
}

impl Message {
    pub fn decode(&self) -> Result<rmpv::Value, ProtoError> {
        rmpv::decode::read_value(&mut &self.body[..]).map_err(|e| ProtoError::MessagePack(e.to_string()))
    }
}

#[derive(Debug, Default)]
pub struct Reassembler {
    streams: HashMap<(u16, u16), Vec<u8>>,
}

impl Reassembler {
    pub fn push(&mut self, frame: &Frame) -> Result<Vec<Message>, ProtoError> {
        let Some(payload) = frame.payload().filter(|p| !p.is_empty()) else { return Ok(Vec::new()) };
        let buf = self.streams.entry((frame.src, frame.dst)).or_default();
        buf.extend_from_slice(payload);
        let mut out = Vec::new();
        while let Some(total) = message_len(buf)
            && buf.len() >= total
        {
            let rest = buf.split_off(total);
            let msg = std::mem::replace(buf, rest);
            out.push(Message {
                src: frame.src,
                dst: frame.dst,
                from_host: u16::from_le_bytes([msg[0], msg[1]]) == 1,
                service: u16::from_le_bytes([msg[2], msg[3]]),
                body: msg[8..].to_vec(),
            });
        }
        Ok(out)
    }

    /// Total size (header included) of the message being assembled on a stream, if one is open.
    pub fn pending(&self, src: u16, dst: u16) -> Option<usize> {
        self.streams.get(&(src, dst)).and_then(|b| message_len(b))
    }
}

fn message_len(buf: &[u8]) -> Option<usize> {
    (buf.len() >= 8).then(|| 8 + u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize)
}
