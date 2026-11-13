//! The preset list read on the control channel with [`crate::Request::ListPresets`].

use rmpv::Value;

use crate::ProtoError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    /// The preset's place on the pedal, 0 for 01A; what op 20 calls key 108.
    pub index: u64,
    pub name: String,
}

/// Parses the result of op 1: one single-entry map `{index: {109: name, ...}}` per preset.
pub fn parse_preset_list(result: &Value) -> Result<Vec<PresetEntry>, ProtoError> {
    let bad = |what: &str| ProtoError::MessagePack(format!("preset list: {what}"));
    let items = result.as_array().ok_or_else(|| bad("not an array"))?;
    items
        .iter()
        .map(|item| {
            let [(index, fields)] = item.as_map().map(Vec::as_slice).ok_or_else(|| bad("entry is not a map"))? else {
                return Err(bad("entry does not hold one preset"));
            };
            let index = index.as_u64().ok_or_else(|| bad("index is not a number"))?;
            let name = fields
                .as_map()
                .and_then(|m| m.iter().find(|(k, _)| k.as_u64() == Some(109)))
                .and_then(|(_, v)| v.as_str())
                .ok_or_else(|| bad("preset without a name"))?;
            Ok(PresetEntry { index, name: name.trim_end_matches('\0').to_owned() })
        })
        .collect()
}

/// The name the pedal shows for a preset place: banks of three, 01A to 42C.
pub fn slot_label(index: u64) -> String {
    format!("{:02}{}", index / 3 + 1, ['A', 'B', 'C'][(index % 3) as usize])
}
