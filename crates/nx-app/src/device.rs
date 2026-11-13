//! Presets as the pedal holds them: the op-22 document plus the name from op 23, read from a
//! capture or from the pedal itself, and turned into an `.hlx` view for the editor.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use nx_capture::{Filter, read_transfers};
use nx_catalog::ModelSymbols;
use nx_proto::{Document, Frame, ParamValue, RPC_CHANNEL, Reassembler, Request, SlotKind, split_transfer};
use rmpv::Value;
use serde_json::{Map, json};

#[derive(Debug)]
pub struct DevicePreset {
    pub name: String,
    pub document: Vec<u8>,
}

/// The last preset HX Edit read in a capture.
pub fn preset_from_capture(path: &Path) -> Result<DevicePreset, String> {
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let transfers = read_transfers(file, &filter).map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    for t in &transfers {
        frames.extend(split_transfer(&t.data).map_err(|e| e.to_string())?);
    }
    preset_from_frames(frames)?.ok_or_else(|| format!("{}: no preset document in this capture", path.display()))
}

/// Connects to the HX Stomp and reads the preset it has selected; the session stays open for
/// editing and closes when dropped.
pub fn connect_device() -> Result<(nx_session::Session, DevicePreset), String> {
    let transport = nx_usb::UsbTransport::open().map_err(|e| format!("HX Stomp: {e}"))?;
    let session = nx_session::Session::connect(transport).map_err(|e| format!("HX Stomp: {e}"))?;
    let preset = preset_from_session(&session)?;
    Ok((session, preset))
}

/// Reads only: preset info (op 23), then the document (op 22), as HX Edit does after a switch.
pub fn preset_from_session(session: &nx_session::Session) -> Result<DevicePreset, String> {
    let info = session.request(Request::PresetInfo).map_err(|e| format!("preset info: {e}"))?;
    let document = match session.request(Request::PresetDocument).map_err(|e| format!("preset document: {e}"))? {
        // The document travels as a MessagePack str, not bin, although it is not UTF-8.
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Binary(b) => b,
        other => return Err(format!("preset document: unexpected reply {other}")),
    };
    Ok(DevicePreset { name: preset_name(&info).unwrap_or_default(), document })
}

fn preset_name(info: &Value) -> Option<String> {
    info.as_map()
        .and_then(|m| m.iter().find(|(k, _)| k.as_u64() == Some(109)))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.trim_end_matches('\0').to_owned())
}

/// The last preset read in a stream of frames, if any.
fn preset_from_frames(frames: impl IntoIterator<Item = Frame>) -> Result<Option<DevicePreset>, String> {
    let mut reassembler = Reassembler::default();
    let mut asked: Vec<(u64, u64)> = Vec::new();
    let (mut name, mut document) = (None, None);
    for frame in frames {
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
            // Transaction ids start again on every connect; a reply answers the latest request.
            let Some(&(_, opcode)) = asked.iter().rev().find(|(t, _)| *t == txn) else { continue };
            match (opcode, field(104)) {
                // The document travels as a MessagePack str, not bin, although it is not UTF-8.
                (22, Some(Value::String(s))) => document = Some(s.as_bytes().to_vec()),
                (23, Some(info)) => name = preset_name(info).or(name),
                _ => {}
            }
        }
    }
    Ok(document.map(|document| DevicePreset { name: name.unwrap_or_default(), document }))
}

/// Where an `.hlx` block lives on the pedal: what op 78 and op 30 call slot (key 98) and key 26,
/// and its parameters in the order op 30 counts them (key 28).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockAddress {
    pub slot: u64,
    /// Key 26, which is not the path: the slot number already places a block on path A or B, and
    /// the pedal reports its path B cab as `{98: 17, 26: 0}`. 26: 1 selected a second cab that a
    /// single cab block does not have (the pedal showed None and refused every edit), so it is
    /// probably the sub-block of a dual cab or an amp+cab.
    pub sub: u64,
    pub params: Vec<String>,
}

impl BlockAddress {
    pub fn param(&self, name: &str) -> Option<u64> {
        self.params.iter().position(|p| p == name).map(|i| i as u64)
    }
}

impl DevicePreset {
    /// Addresses by `.hlx` block key, numbered as [`DevicePreset::to_hlx`] numbers them.
    pub fn addresses(&self, syms: &ModelSymbols) -> Result<HashMap<String, BlockAddress>, String> {
        let preset = Document::decode(&self.document).and_then(|d| d.preset()).map_err(|e| e.to_string())?;
        Ok(preset
            .blocks()
            .filter(|p| p.dsp == 0)
            .enumerate()
            .map(|(i, placed)| {
                let names = syms.get(placed.block.model).map(|s| s.parameters.clone()).unwrap_or_default();
                let count = (placed.block.values.params as usize).min(names.len());
                let address = BlockAddress { slot: placed.slot as u64, sub: 0, params: names[..count].to_vec() };
                (format!("block{i}"), address)
            })
            .collect())
    }

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

    fn message(from_host: bool, body: &Value) -> Frame {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, body).unwrap();
        let mut msg = vec![u8::from(from_host), 0, 6, 0];
        msg.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        msg.extend_from_slice(&bytes);
        let (src, dst) = if from_host { (RPC_CHANNEL, 0x03ed) } else { (0x03ed, RPC_CHANNEL) };
        Frame::channel(src, dst, 2, nx_proto::FrameKind::Data, 0x1000, &msg)
    }

    fn request(txn: u64, request: Request) -> Frame {
        message(true, &rmpv::decode::read_value(&mut &request.encode(txn)[..]).unwrap())
    }

    fn reply(txn: u64, result: Value) -> Frame {
        message(false, &Value::Map(vec![(102.into(), txn.into()), (103.into(), 0.into()), (104.into(), result)]))
    }

    fn info(name: Option<&str>) -> Value {
        Value::Map(name.map(|n| (109.into(), Value::from(format!("{n}\0")))).into_iter().collect())
    }

    #[test]
    fn a_reused_transaction_id_belongs_to_the_latest_request() {
        // A reconnect starts the transaction ids again, so 1000 is op 23 first and op 22 later.
        let frames = [
            request(1000, Request::PresetInfo),
            reply(1000, info(Some("Old"))),
            request(1001, Request::PresetDocument),
            reply(1001, Value::from("old document")),
            request(1000, Request::PresetDocument),
            reply(1000, Value::from("new document")),
            request(1001, Request::PresetInfo),
            reply(1001, info(Some("New"))),
        ];
        let preset = preset_from_frames(frames).unwrap().unwrap();
        assert_eq!(preset.document, b"new document");
        assert_eq!(preset.name, "New");
    }

    #[test]
    fn a_preset_info_without_a_name_keeps_the_name() {
        let frames = [
            request(1000, Request::PresetInfo),
            reply(1000, info(Some("Revv DM"))),
            request(1001, Request::PresetDocument),
            reply(1001, Value::from("document")),
            request(1002, Request::PresetInfo),
            reply(1002, info(None)),
        ];
        assert_eq!(preset_from_frames(frames).unwrap().unwrap().name, "Revv DM");
    }

    #[test]
    fn a_capture_without_a_document_is_an_error() {
        assert!(preset_from_capture(&capture("01-connect")).is_err());
    }

    fn fake_stomp(reply: impl Fn(&Request) -> nx_session::fake::Reply + Send + 'static) -> nx_session::fake::FakeDevice {
        let device = nx_session::fake::FakeDevice::new();
        device.strict(true);
        device.on_request(move |_, _, req| reply(req));
        device
    }

    #[test]
    fn reads_the_current_preset_over_a_session() {
        use nx_session::fake::Reply;
        let device = fake_stomp(|req| match req {
            Request::PresetInfo => Reply::Ok(info(Some("Revv DM"))),
            Request::PresetDocument => Reply::Ok(Value::from("document")),
            _ => Reply::Silent,
        });
        let session = nx_session::Session::connect(device.transport()).unwrap();
        let preset = preset_from_session(&session).unwrap();
        assert_eq!(preset.name, "Revv DM");
        assert_eq!(preset.document, b"document");
        let asked: Vec<u64> = device.requests().iter().map(|(_, _, r)| r.opcode()).collect();
        assert_eq!(asked, vec![23, 22], "read only: preset info, then the document");
    }

    #[test]
    fn a_refused_document_is_reported() {
        use nx_session::fake::Reply;
        let device = fake_stomp(|req| match req {
            Request::PresetInfo => Reply::Ok(info(Some("Revv DM"))),
            _ => Reply::Status(5, Value::Nil),
        });
        let session = nx_session::Session::connect(device.transport()).unwrap();
        let err = preset_from_session(&session).unwrap_err();
        assert!(err.contains("status 5"), "{err}");
    }

    #[test]
    fn a_document_that_is_not_a_string_is_reported() {
        use nx_session::fake::Reply;
        let device = fake_stomp(|req| match req {
            Request::PresetInfo => Reply::Ok(info(Some("Revv DM"))),
            _ => Reply::Ok(Value::from(7)),
        });
        let session = nx_session::Session::connect(device.transport()).unwrap();
        assert!(preset_from_session(&session).is_err());
    }

    #[test]
    fn blocks_keep_their_address_on_the_pedal() {
        let Some(syms) = helix_sym() else { return };
        let captured = preset_from_capture(&capture("02-select-preset-01C")).unwrap();
        let addresses = captured.addresses(&syms).unwrap();
        let amp = &addresses["block2"];
        assert_eq!((amp.slot, amp.sub), (3, 0));
        let cab = addresses.values().find(|a| a.slot == 17).expect("the cab on path B is slot 17");
        assert_eq!(cab.sub, 0, "key 26 is not the path: 1 selects a second cab that is not there");
        assert!(addresses.values().all(|a| a.sub == 0));
        assert_eq!(amp.param("Drive"), Some(0));
        assert_eq!(amp.param("Nonexistent"), None);
        let hlx = captured.to_hlx(&syms).unwrap();
        for block in hlx.blocks() {
            assert!(addresses.contains_key(&block.key), "{} has no address", block.key);
        }
    }
}
