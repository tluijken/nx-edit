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
    /// The preset's place on the pedal (key 108 of op 23), 0 for 01A.
    pub index: Option<u64>,
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

/// Connects to the HX Stomp and reads its preset list and the preset it has selected; the session
/// stays open for editing and closes when dropped. A preset list that cannot be read is reported,
/// not fatal: editing the selected preset does not need it.
pub struct Connected {
    pub session: nx_session::Session,
    pub presets: Result<Vec<nx_proto::PresetEntry>, String>,
    pub preset: DevicePreset,
}

pub fn connect_device() -> Result<Connected, String> {
    let transport = nx_usb::UsbTransport::open().map_err(|e| format!("HX Stomp: {e}"))?;
    let session = nx_session::Session::connect(transport).map_err(|e| format!("HX Stomp: {e}"))?;
    let presets = preset_list(&session);
    let preset = preset_from_session(&session)?;
    Ok(Connected { session, presets, preset })
}

/// The presets of the pedal's setlist, read as HX Edit reads them after connecting [seen, 06].
pub fn preset_list(session: &nx_session::Session) -> Result<Vec<nx_proto::PresetEntry>, String> {
    use nx_session::Channel::Control;
    session.request_on(Control, Request::ControlStart).map_err(|e| format!("preset list: {e}"))?;
    session.request_on(Control, Request::ListSetlists).map_err(|e| format!("setlists: {e}"))?;
    let list = session.request_on(Control, Request::ListPresets { setlist: 0 }).map_err(|e| format!("preset list: {e}"))?;
    nx_proto::parse_preset_list(&list).map_err(|e| e.to_string())
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
    Ok(DevicePreset { name: preset_name(&info).unwrap_or_default(), index: info_field(&info, 108).and_then(Value::as_u64), document })
}

fn info_field(info: &Value, key: u64) -> Option<&Value> {
    info.as_map()?.iter().find(|(k, _)| k.as_u64() == Some(key)).map(|(_, v)| v)
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
    let (mut name, mut index, mut document) = (None, None, None);
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
                (23, Some(info)) => {
                    name = preset_name(info).or(name);
                    index = info_field(info, 108).and_then(Value::as_u64).or(index);
                }
                _ => {}
            }
        }
    }
    Ok(document.map(|document| DevicePreset { name: name.unwrap_or_default(), index, document }))
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
        // Per slot: the block's key and its parameter names, for snapshots and controllers.
        let mut slots: HashMap<i64, (String, &[String])> = HashMap::new();
        for (i, placed) in preset.blocks().filter(|p| p.dsp == 0).enumerate() {
            let block = placed.block;
            let sym = syms.get(block.model);
            slots.insert(placed.slot as i64, (format!("block{i}"), sym.map(|s| s.parameters.as_slice()).unwrap_or_default()));
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
        // A controller assignment by its id: the block key and the parameter name it moves.
        let assigned = |id: i64| -> Option<(&str, &str)> {
            let assignment = preset.controllers.iter().find(|a| a.id == id)?;
            let (key, names) = slots.get(&assignment.slot)?;
            Some((key.as_str(), names.get(usize::try_from(assignment.param).ok()?)?.as_str()))
        };
        let mut controller = Map::new();
        for assignment in &preset.controllers {
            let Some((key, name)) = assigned(assignment.id) else { continue };
            let block = controller.entry(key).or_insert_with(|| json!({}));
            block[name] = json!({"@controller": assignment.controller, "@min": param_json(&assignment.min), "@max": param_json(&assignment.max)});
        }
        let mut tone = Map::new();
        tone.insert("dsp0".into(), dsp0.into());
        tone.insert("global".into(), json!({"@current_snapshot": preset.snapshots.current}));
        tone.insert("controller".into(), json!({"dsp0": controller}));
        for (i, snap) in preset.snapshots.list.iter().enumerate() {
            let mut blocks = Map::new();
            for (slot, state) in snap.slots.iter().enumerate() {
                if let Some((key, _)) = slots.get(&(slot as i64)) {
                    blocks.insert(key.clone(), json!(state.enabled));
                }
            }
            let mut values = Map::new();
            for value in &snap.controllers {
                let Some((key, name)) = assigned(value.assignment) else { continue };
                let block = values.entry(key).or_insert_with(|| json!({}));
                block[name] = json!({"@fs_enabled": value.flag, "@value": param_json(&value.value)});
            }
            tone.insert(
                format!("snapshot{i}"),
                json!({
                    "@name": snap.name,
                    "@valid": snap.valid,
                    "@custom_name": snap.custom_name,
                    "@ledcolor": snap.led_color,
                    "@tempo": f64::from(snap.tempo),
                    "blocks": {"dsp0": blocks},
                    "controllers": {"dsp0": values},
                }),
            );
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

    fn capture(name: &str) -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/captures").join(format!("{name}.pcapng"));
        if !path.exists() {
            eprintln!("skipped: no {}", path.display());
            return None;
        }
        Some(path)
    }

    /// Revv DM as HX Edit read it from slot 01C in capture 02, or after the edits of capture 05.
    fn captured(name: &str) -> DevicePreset {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures").join(format!("{name}.doc"));
        DevicePreset { name: "Revv DM".into(), index: Some(2), document: std::fs::read(path).unwrap() }
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
        let Some(path) = capture("02-select-preset-01C") else { return };
        let read = preset_from_capture(&path).unwrap();
        let want = captured("02-select-preset-01C");
        assert_eq!((read.name, read.index, read.document), (want.name, want.index, want.document));
    }

    #[test]
    fn converted_capture_matches_the_imported_hlx() {
        let Some(syms) = helix_sym() else { return };
        let captured = captured("02-select-preset-01C");
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
        let converted = captured("02-select-preset-01C").to_hlx(&syms).unwrap();
        assert_eq!(lanes(&converted, None), lanes(&fixture(), None));
    }

    #[test]
    fn converted_capture_keeps_the_active_snapshot() {
        let Some(syms) = helix_sym() else { return };
        let lead = captured("05-snapshot-switch").to_hlx(&syms).unwrap();
        assert_eq!(lead.current_snapshot(), 2);
    }

    fn tone(preset: &nx_hlx::Preset) -> serde_json::Value {
        serde_json::from_slice::<serde_json::Value>(&preset.to_vec_pretty()).unwrap()["data"]["tone"].clone()
    }

    #[test]
    fn converted_capture_keeps_each_snapshots_block_states() {
        let Some(syms) = helix_sym() else { return };
        let converted = tone(&captured("02-select-preset-01C").to_hlx(&syms).unwrap());
        let want = tone(&fixture());
        for i in 0..3 {
            let snapshot = format!("snapshot{i}");
            assert_eq!(converted[&snapshot]["blocks"], want[&snapshot]["blocks"], "{snapshot}");
        }
    }

    #[test]
    fn converted_capture_keeps_the_snapshot_controller_and_its_values() {
        let Some(syms) = helix_sym() else { return };
        let converted = tone(&captured("02-select-preset-01C").to_hlx(&syms).unwrap());
        let want = tone(&fixture());
        assert_eq!(converted["controller"], want["controller"]);
        for i in 0..3 {
            let snapshot = format!("snapshot{i}");
            assert_eq!(converted[&snapshot]["controllers"], want[&snapshot]["controllers"], "{snapshot}");
        }
    }

    #[test]
    fn converted_capture_keeps_snapshot_settings() {
        let Some(syms) = helix_sym() else { return };
        let converted = tone(&captured("02-select-preset-01C").to_hlx(&syms).unwrap());
        let want = tone(&fixture());
        for i in 0..3 {
            for key in ["@name", "@valid", "@custom_name", "@ledcolor", "@tempo"] {
                assert_eq!(converted[format!("snapshot{i}")][key], want[format!("snapshot{i}")][key], "snapshot{i} {key}");
            }
        }
    }

    #[test]
    fn converted_snapshots_switch_like_the_hlx() {
        let Some(syms) = helix_sym() else { return };
        let mut converted = captured("02-select-preset-01C").to_hlx(&syms).unwrap();
        let mut want = fixture();
        converted.select_snapshot(2);
        want.select_snapshot(2);
        assert_eq!(lanes(&converted, None), lanes(&want, None));
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
        let Some(path) = capture("01-connect") else { return };
        assert!(preset_from_capture(&path).is_err());
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
        let captured = captured("02-select-preset-01C");
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

    #[test]
    fn the_preset_list_is_read_on_the_control_channel_as_hx_edit_reads_it() {
        use nx_session::Channel;
        use nx_session::fake::{FakeDevice, Reply};
        let device = FakeDevice::new();
        device.strict(true);
        device.on_request(|_, _, req| match req {
            Request::ListSetlists => Reply::Ok(Value::Array(vec![Value::Map(vec![(0.into(), "PRESETS\0".into())])])),
            Request::ListPresets { setlist: 0 } => Reply::Ok(Value::Array(
                ["Recto DM", "5150 DM"]
                    .iter()
                    .enumerate()
                    .map(|(i, n)| Value::Map(vec![(i.into(), Value::Map(vec![(109.into(), format!("{n}\0").into())]))]))
                    .collect(),
            )),
            _ => Reply::Ok(Value::Nil),
        });
        let session = nx_session::Session::connect(device.transport()).unwrap();
        let list = preset_list(&session).unwrap();
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Recto DM", "5150 DM"]);
        let asked: Vec<(Channel, u64)> = device.requests().iter().map(|(c, _, r)| (*c, r.opcode())).collect();
        assert_eq!(asked, [(Channel::Control, 254), (Channel::Control, 0), (Channel::Control, 1)]);
    }
}
