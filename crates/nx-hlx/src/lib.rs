//! Line 6 `.hlx` presets, kept as their full JSON document so nothing nx-edit does not model is lost.

mod preset;

pub use preset::{Block, HlxError, HX_STOMP_DEVICE, Preset, SplitInfo};
