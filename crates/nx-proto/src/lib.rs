//! The HX Stomp editor protocol on interface `MI_00`, derived from nx-edit's own captures with
//! tonepush's PROTOCOL.md as a map. Pure codec: no I/O.

mod document;
mod frame;
mod message;
mod presets;
mod rpc;

pub use document::{
    Assignment, Block, ControllerValue, Cursor, DocError, Document, Dsp, Endpoint, Firmware, IntForm, Item, Junction,
    LenForm, ParamValue, Placed, Preset, SNAPSHOT_NAME_MAX, SlotKind, SlotState, Snapshot, Snapshots, Values,
};
pub use frame::{FORMAT_CHANNEL, FORMAT_HELLO, Frame, FrameKind, split_leading, split_transfer};
pub use message::{Message, Reassembler};
pub use presets::{PresetEntry, parse_preset_list, slot_label};
pub use rpc::{RPC_CHANNEL, Request};

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("frame at offset {offset} needs {need} bytes, transfer has {have}")]
    Truncated { offset: usize, need: usize, have: usize },
    #[error("MessagePack: {0}")]
    MessagePack(String),
}
