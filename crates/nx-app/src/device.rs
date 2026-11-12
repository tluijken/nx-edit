//! Presets as the pedal holds them: the op-22 document plus the name from op 23, read from a capture
//! for now (the live session comes later), and turned into an `.hlx` view for the editor.

use std::fs::File;
use std::path::Path;

use nx_capture::{Filter, read_transfers};
use nx_catalog::ModelSymbols;
use nx_proto::{Document, ParamValue, RPC_CHANNEL, Reassembler, Request, SlotKind, split_transfer};
use rmpv::Value;
use serde_json::{Map, json};

pub struct DevicePreset {
    pub name: String,
    pub document: Vec<u8>,
}

/// The last preset HX Edit read in a capture.
pub fn preset_from_capture(path: &Path) -> Result<DevicePreset, String> {
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let transfers = read_transfers(file, &filter).map_err(|e| e.to_string())?;
    let mut reassembler = Reassembler::default();
    let mut asked: Vec<(u64, u64)> = Vec::new();
    let (mut name, mut document) = (None, None);
    for t in &transfers {
        for frame in split_transfer(&t.data).map_err(|e| e.to_string())? {
            for msg in reassembler.push(&frame).map_err(|e| e.to_string())? {
                let Ok(value) = msg.decode() else { continue };
                if msg.from_host && msg.src == RPC_CHANNEL {
                    if let Some((txn, request)) = Request::from_value(&value) {
                        asked.push((txn, request.opcode()));
                    }
                    continue;
                }
                if msg.from_host || msg.dst != RPC_CHANNEL {
                    continue;
                }
                let Some(reply) = value.as_map() else { continue };
                let field = |k| reply.iter().find(|(key, _)| key.as_u64() == Some(k)).map(|(_, v)| v);
                let Some(txn) = field(102).and_then(Value::as_u64) else { continue };
                let Some(&(_, opcode)) = asked.iter().find(|(t, _)| *t == txn) else { continue };
                match (opcode, field(104)) {
                    // The document travels as a MessagePack str, not bin, although it is not UTF-8.
                    (22, Some(Value::String(s))) => document = Some(s.as_bytes().to_vec()),
                    (23, Some(info)) => {
                        name = info
                            .as_map()
                            .and_then(|m| m.iter().find(|(k, _)| k.as_u64() == Some(109)))
                            .and_then(|(_, v)| v.as_str())
                            .map(|s| s.trim_end_matches('\0').to_owned());
                    }
                    _ => {}
                }
            }
        }
    }
    let document = document.ok_or_else(|| format!("{}: no preset document in this capture", path.display()))?;
    Ok(DevicePreset { name: name.unwrap_or_default(), document })
}

impl DevicePreset {
    /// The document as an `.hlx` preset, with blocks numbered in slot order like HX Edit's exports.
    pub fn to_hlx(&self, syms: &ModelSymbols) -> Result<nx_hlx::Preset, String> {
        let preset = Document::decode(&self.document).and_then(|d| d.preset()).map_err(|e| e.to_string())?;
        let symbol = |model: u32| {
            syms.get(model).map(|s| s.symbol.clone()).ok_or_else(|| format!("model {model} is not in Helix.sym"))
        };
        let mut dsp0 = Map::new();
        for (i, placed) in preset.blocks().filter(|p| p.dsp == 0).enumerate() {
            let block = placed.block;
            let sym = syms.get(block.model);
            let mut obj = Map::new();
            obj.insert("@model".into(), json!(nx_catalog::shared_id(&symbol(block.model)?)));
            obj.insert("@path".into(), json!(placed.path));
            obj.insert("@position".into(), json!(placed.position));
            obj.insert("@enabled".into(), json!(block.enabled));
            let names = sym.map(|s| s.parameters.as_slice()).unwrap_or_default();
            for (name, value) in names.iter().zip(&block.values.values).take(block.values.params as usize) {
                obj.insert(name.clone(), param_json(value));
            }
            dsp0.insert(format!("block{i}"), obj.into());
        }
        if let Some(Some(dsp)) = preset.dsps.first() {
            for slot in &dsp.slots {
                let (key, junction) = match slot {
                    SlotKind::Split(j) => ("split", j),
                    SlotKind::Join(j) => ("join", j),
                    _ => continue,
                };
                // The document stores the path-0 slot a junction precedes; slot 0 is the input.
                dsp0.insert(
                    key.into(),
                    json!({"@model": symbol(junction.model)?, "@position": junction.before_slot - 1, "@enabled": junction.enabled}),
                );
            }
        }
        let mut tone = Map::new();
        tone.insert("dsp0".into(), dsp0.into());
        tone.insert("global".into(), json!({"@current_snapshot": preset.snapshots.current}));
        for (i, snap) in preset.snapshots.list.iter().enumerate() {
            tone.insert(format!("snapshot{i}"), json!({"@name": snap.name, "@valid": snap.valid}));
        }
        let hlx = json!({
            "data": {
                "device": nx_hlx::HX_STOMP_DEVICE,
                "device_version": preset.firmware.version,
                "meta": {"name": self.name},
                "tone": tone,
            },
            "schema": "L6Preset",
            "version": 6,
        });
        nx_hlx::Preset::from_slice(&serde_json::to_vec(&hlx).expect("JSON values serialise")).map_err(|e| e.to_string())
    }
}

fn param_json(value: &ParamValue) -> serde_json::Value {
    match value {
        ParamValue::Float(f) => json!(f64::from(*f)),
        ParamValue::Int(i) => json!(i),
        ParamValue::Bool(b) => json!(b),
        ParamValue::Other(_) => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::layout::lanes;

    fn capture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures").join(format!("{name}.pcapng"))
    }

    fn fixture() -> nx_hlx::Preset {
        nx_hlx::Preset::from_slice(include_bytes!("../../nx-hlx/tests/fixtures/Revv_DM.hlx")).unwrap()
    }

    fn helix_sym() -> Option<ModelSymbols> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        let path = dir.join("Helix.sym");
        if !path.exists() {
            eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
            return None;
        }
        Some(ModelSymbols::load(&path).unwrap())
    }

    #[test]
    fn capture_02_holds_revv_dm() {
        let captured = preset_from_capture(&capture("02-select-preset-01C")).unwrap();
        assert_eq!(captured.name, "Revv DM");
        assert_eq!(captured.document.len(), 2801);
    }

    #[test]
    fn converted_capture_matches_the_imported_hlx() {
        let Some(syms) = helix_sym() else { return };
        let captured = preset_from_capture(&capture("02-select-preset-01C")).unwrap();
        let converted = captured.to_hlx(&syms).unwrap();
        assert_eq!(converted.name(), "Revv DM");
        assert_eq!(converted.snapshot_names(), vec!["808", "33", "Lead"]);
        assert_eq!(converted.split().unwrap().position, fixture().split().unwrap().position);
        let got = converted.blocks();
        for want in fixture().blocks() {
            let block = got.iter().find(|b| b.path == want.path && b.position == want.position).unwrap();
            assert_eq!(block.model, want.model, "{}", want.key);
            assert_eq!(block.enabled, want.enabled, "{}", want.key);
            for (name, value) in &want.params {
                // The generator wrote Scale, which only the stereo Ducked Delay has; the device dropped it.
                if name == "Scale" {
                    continue;
                }
                let have = block.params.iter().find(|(k, _)| k == name).unwrap_or_else(|| panic!("{}.{name}", want.key)).1.clone();
                let num = |v: &serde_json::Value| v.as_f64().unwrap_or_else(|| f64::from(u8::from(v.as_bool().unwrap())));
                assert!((num(&have) - num(value)).abs() < 1e-6, "{}.{name}: {have} vs {value}", want.key);
            }
        }
    }

    #[test]
    fn converted_capture_lays_out_like_the_hlx() {
        let Some(syms) = helix_sym() else { return };
        let converted = preset_from_capture(&capture("02-select-preset-01C")).unwrap().to_hlx(&syms).unwrap();
        assert_eq!(lanes(&converted, None), lanes(&fixture(), None));
    }

    #[test]
    fn converted_capture_keeps_the_active_snapshot() {
        let Some(syms) = helix_sym() else { return };
        let lead = preset_from_capture(&capture("05-snapshot-switch")).unwrap().to_hlx(&syms).unwrap();
        assert_eq!(lead.current_snapshot(), 2);
    }

    #[test]
    fn a_capture_without_a_document_is_an_error() {
        assert!(preset_from_capture(&capture("01-connect")).is_err());
    }
}
