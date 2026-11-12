//! The HX Stomp editor protocol on interface `MI_00`, derived from nx-edit's own captures with
//! tonepush's PROTOCOL.md as a map. Pure codec: no I/O.

mod frame;
mod message;

pub use frame::{FORMAT_CHANNEL, FORMAT_HELLO, Frame, FrameKind, split_transfer};
pub use message::{Message, Reassembler};

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("frame at offset {offset} needs {need} bytes, transfer has {have}")]
    Truncated { offset: usize, need: usize, have: usize },
    #[error("MessagePack: {0}")]
    MessagePack(String),
}
