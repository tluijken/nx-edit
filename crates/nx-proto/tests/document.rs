//! The preset document (RPC opcode 22) of preset 01C "Revv DM", as captured, checked against
//! `crates/nx-hlx/tests/fixtures/Revv_DM.hlx`, the file that was imported into that slot.
//!
//! `02` is the document right after selecting the preset (snapshot 808). `05` is the same preset
//! after capture 03 dragged the amp's Drive to 0.6, 04 toggled the delay on and off, and 05 switched
//! to snapshot Lead. `06` (after a reconnect) is byte-identical to `05`.
//!
//! The captures stay out of git; `tests/fixtures/*.doc` are the documents cut from them, and the
//! tests that read the captures themselves are skipped where `local/captures` is absent.

use std::fs::File;
use std::path::{Path, PathBuf};

use nx_capture::{Filter, read_transfers};
use nx_proto::{Document, ParamValue, RPC_CHANNEL, Reassembler, Request, SlotKind, split_transfer};
use rmpv::Value;

fn capture(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../local/captures/{name}.pcapng"));
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipped: no {}", path.display());
        None
    }
}

/// The result bytes of every opcode-22 reply in a capture.
fn documents(path: &Path) -> Vec<Vec<u8>> {
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let mut reassembler = Reassembler::default();
    let mut asked = Vec::new();
    let mut out = Vec::new();
    for t in read_transfers(File::open(path).unwrap(), &filter).unwrap() {
        for frame in split_transfer(&t.data).unwrap() {
            for msg in reassembler.push(&frame).unwrap() {
                let value = msg.decode().unwrap();
                if msg.from_host && msg.src == RPC_CHANNEL {
                    if let Some((txn, Request::PresetDocument)) = Request::from_value(&value) {
                        asked.push(txn);
                    }
                    continue;
                }
                if msg.from_host || msg.dst != RPC_CHANNEL {
                    continue;
                }
                let Some(reply) = value.as_map() else { continue };
                let field = |k| reply.iter().find(|(key, _)| key.as_u64() == Some(k)).map(|(_, v)| v);
                if field(102).and_then(Value::as_u64).is_some_and(|txn| asked.contains(&txn)) {
                    // The document travels as a MessagePack str, not bin, although it is not UTF-8.
                    match field(104) {
                        Some(Value::String(s)) => out.push(s.as_bytes().to_vec()),
                        other => panic!("{}: document is not a str: {other:?}", path.display()),
                    }
                }
            }
        }
    }
    out
}

fn document(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.doc"))).unwrap()
}

fn float(v: &ParamValue) -> f32 {
    match v {
        ParamValue::Float(f) => *f,
        other => panic!("not a float: {other:?}"),
    }
}

fn floats(values: &[ParamValue]) -> Vec<f32> {
    values.iter().map(float).collect()
}

#[test]
fn the_fixtures_are_the_captured_documents() {
    let (Some(c02), Some(c05), Some(c06)) =
        (capture("02-select-preset-01C"), capture("05-snapshot-switch"), capture("06-reconnect"))
    else {
        return;
    };
    assert_eq!(documents(&c02), vec![document("02-select-preset-01C")]);
    assert_eq!(documents(&c05), vec![document("05-snapshot-switch")]);
    assert_eq!(documents(&c06), vec![document("05-snapshot-switch")]);
}

#[test]
fn the_two_documents() {
    let d02 = document("02-select-preset-01C");
    let d05 = document("05-snapshot-switch");
    assert_eq!(d02.len(), 2801);
    assert!(d02.starts_with(b"\xa9l6-helix\0"));
    assert_ne!(d02, d05);
}

#[test]
fn captured_documents_reencode_byte_for_byte() {
    for name in ["02-select-preset-01C", "05-snapshot-switch"] {
        let bytes = document(name);
        let doc = Document::decode(&bytes).unwrap();
        assert_eq!(doc.encode().unwrap(), bytes, "{name}");
    }
}

#[test]
fn section_table_points_at_top_level_keys() {
    let doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    // map start, keys 0 1 3 4 2 5 6 7 10, then the total length twice.
    assert_eq!(doc.sections().unwrap(), [61, 96, 904, 906, 917, 1040, 1280, 1325, 62, 1331, 2801, 2801]);
}

#[test]
fn rejects_what_is_not_a_document() {
    assert!(Document::decode(b"\xa8l6-helox").is_err());
    let mut bytes = document("02-select-preset-01C");
    bytes.truncate(2000);
    assert!(Document::decode(&bytes).is_err());
}

#[test]
fn firmware_header() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    assert_eq!(preset.firmware.service, "P33");
    assert_eq!(preset.firmware.version, 0x0371_0000);
    assert_eq!(preset.firmware.build, "v3.70-10-ge773a6f");
}

#[test]
fn preset_name_is_not_in_the_document() {
    // The name comes from opcode 23; the only strings here are the header and snapshot names.
    let bytes = document("02-select-preset-01C");
    assert!(!bytes.windows(4).any(|w| w == b"Revv"));
}

#[test]
fn slot_layout_of_revv_dm() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    assert_eq!(preset.dsps.len(), 2);
    assert!(preset.dsps[1].is_none(), "an HX Stomp has one DSP");
    let slots = &preset.dsps[0].as_ref().unwrap().slots;
    let kinds: String = slots
        .iter()
        .map(|s| match s {
            SlotKind::Input(_) => 'I',
            SlotKind::Output(_) => 'O',
            SlotKind::Split(_) => 'S',
            SlotKind::Join(_) => 'J',
            SlotKind::Block(_) => 'B',
            SlotKind::Empty => '.',
            SlotKind::Unknown { .. } => '?',
        })
        .collect();
    assert_eq!(kinds, "IBBBBBB..OS......BBJ");
}

#[test]
fn blocks_with_model_numbers_paths_and_positions() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let placed: Vec<_> = preset.blocks().map(|b| (b.slot, b.block.model, b.path, b.position)).collect();
    // Paths and positions are those of Revv_DM.hlx: the cab and the send sit on path 1.
    assert_eq!(
        placed,
        vec![
            (1, 101, 0, 0),  // Scream 808 (mono)
            (2, 131, 0, 1),  // Parametric EQ (mono)
            (3, 545, 0, 2),  // Revv Gen Red
            (4, 131, 0, 3),  // Parametric EQ (mono)
            (5, 77, 0, 4),   // Ducked Delay (mono)
            (6, 243, 0, 5),  // Hall
            (17, 693, 1, 6), // 4x12 Cali V30 (Cab Mic IR)
            (18, 250, 1, 7), // Send 1 (mono)
        ]
    );
}

#[test]
fn blocks_agree_with_the_imported_hlx() {
    let hlx = nx_hlx::Preset::from_slice(include_bytes!("../../nx-hlx/tests/fixtures/Revv_DM.hlx")).unwrap();
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let placed: Vec<_> = preset.blocks().collect();
    assert_eq!(placed.len(), hlx.blocks().len());
    for b in hlx.blocks() {
        let p = placed
            .iter()
            .find(|p| p.path == b.path as usize && p.position == b.position as usize)
            .unwrap_or_else(|| panic!("{} not in the document", b.key));
        assert_eq!(p.block.enabled, b.enabled, "{}", b.key);
        // Every hlx parameter value appears among the block's values; the order needs Helix.sym, see
        // every_value_matches_the_hlx_by_helix_sym_parameter_name. Scale is not a parameter of the mono
        // Ducked Delay and was dropped by the device.
        for (name, value) in b.params.iter().filter(|(n, _)| !(b.key == "block4" && n == "Scale")) {
            let want = value.as_f64().unwrap_or_else(|| if value.as_bool().unwrap() { 1.0 } else { 0.0 });
            assert!(
                p.block
                    .values
                    .values
                    .iter()
                    .any(|v| v.as_f64() == Some(want) || (v.as_f64().unwrap() - want).abs() < 1e-6),
                "{}.{name} = {want} missing from {:?}",
                b.key,
                p.block.values.values
            );
        }
    }
}

#[test]
fn amp_values_follow_helix_sym_order() {
    let amp = |name| {
        let preset = Document::decode(&document(name)).unwrap().preset().unwrap();
        preset.blocks().find(|b| b.slot == 3).unwrap().block.clone()
    };
    let before = amp("02-select-preset-01C");
    assert_eq!(before.model, 545);
    assert_eq!(before.engine, 17);
    assert!(before.enabled);
    // Drive Bass Mid Treble Presence ChVol Master Resonance Aggression Sag Hum Ripple Bias BiasX
    let v = &before.values.values;
    assert_eq!(v.len(), 14);
    assert_eq!((before.values.count, before.values.params), (14, 14));
    assert_eq!(floats(&v[..8]), [0.45, 0.3, 0.65, 0.68, 0.5, 0.75, 0.4, 0.6]);
    assert_eq!(v[8], ParamValue::Int(1), "Aggression is an integer");
    assert_eq!(floats(&v[9..]), [0.25, 0.5, 0.5, 0.6, 0.5]);
    // Capture 03's last set-parameter was 0.6 (the README's "about 5.5" was HX Edit's 0..10 scale).
    assert_eq!(float(&amp("05-snapshot-switch").values.values[0]), 0.6);
}

#[test]
fn delay_and_reverb_carry_one_value_more_than_they_count() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let block = |slot| preset.blocks().find(|b| b.slot == slot).unwrap().block.clone();
    let delay = block(5);
    assert_eq!((delay.values.count, delay.values.params, delay.engine), (14, 13, 8));
    // Helix.sym lists 13 parameters for HD2_DelayDuckedDelayMono; the 14th is `@trails` (true).
    assert_eq!(delay.values.values[13], ParamValue::Bool(true));
    assert_eq!(delay.values.values[10..13], [ParamValue::Bool(false), ParamValue::Bool(false), ParamValue::Int(6)]);
    let hall = block(6);
    assert_eq!((hall.values.count, hall.values.params, hall.engine), (7, 6, 8));
    assert_eq!(floats(&hall.values.values[..6]), [0.6, 0.05, 200.0, 5000.0, 0.25, 0.0]);
    assert_eq!(hall.values.values[6], ParamValue::Bool(true));
    let cab = block(17);
    // Helix.sym lists IrData as an eighth parameter; the document does not carry it.
    assert_eq!((cab.values.count, cab.values.params, cab.engine), (7, 7, 31));
    assert_eq!(cab.values.values[0], ParamValue::Int(0), "Mic");
    assert_eq!(floats(&cab.values.values[1..]), [0.4, 1.5, 0.0, 110.0, 8500.0, 0.0]);
    let send = block(18);
    assert_eq!(send.engine, 25);
    assert_eq!(floats(&send.values.values), [0.0, -120.0]);
}

#[test]
fn input_output_split_and_join() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let slots = &preset.dsps[0].as_ref().unwrap().slots;
    let SlotKind::Input(input) = &slots[0] else { panic!() };
    // hlx inputA: @input 1, noiseGate true, threshold -45, decay 0.5
    assert_eq!(input.route, 1);
    assert_eq!(input.values.values[0], ParamValue::Bool(true));
    assert_eq!(floats(&input.values.values[1..]), [-45.0, 0.5]);
    let SlotKind::Output(output) = &slots[9] else { panic!() };
    assert_eq!(output.route, 1);
    assert_eq!(floats(&output.values.values), [0.5, 0.0], "pan, gain");
    let SlotKind::Split(split) = &slots[10] else { panic!() };
    assert_eq!(split.model, 257, "HD2_AppDSPFlowSplitY");
    assert!(split.enabled);
    // hlx split @position 6: the split sits before slot 7 of path 0.
    assert_eq!(split.before_slot, 7);
    assert_eq!(split.values.values, [ParamValue::Float(0.5), ParamValue::Float(0.5), ParamValue::Bool(false)]);
    assert_eq!(split.endpoint.route, 0, "hlx inputB @input 0");
    let SlotKind::Join(join) = &slots[19] else { panic!() };
    assert_eq!(join.model, 151, "HD2_AppDSPFlowJoin");
    assert_eq!(join.before_slot, 9, "hlx join @position 8");
    assert_eq!(join.endpoint.route, 0, "hlx outputB @output 0");
    // A Level, A Pan, B Level, B Pan, B Polarity, Level. The hlx has B Level -120; the device
    // stores -60, presumably its minimum.
    assert_eq!(
        join.values.values,
        [0.0, 0.5, -60.0, 0.5]
            .map(ParamValue::Float)
            .into_iter()
            .chain([ParamValue::Bool(false), ParamValue::Float(0.0)])
            .collect::<Vec<_>>()
    );
}

#[test]
fn snapshots_names_and_block_states() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let snaps = &preset.snapshots;
    assert_eq!(snaps.current, 0);
    let names: Vec<_> = snaps.list.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["808", "33", "Lead"]);
    for s in &snaps.list {
        assert!(s.valid);
        assert!(s.custom_name);
        assert_eq!(s.tempo, 120.0);
        assert_eq!(s.led_color, 0);
        assert_eq!(s.slots.len(), 20);
    }
    let on = |snap: usize, slot: usize| snaps.list[snap].slots[slot].enabled;
    // Revv_DM.hlx snapshot blocks: block0..5 at slots 1..6, block6/7 (cab, send) at 17/18.
    let hlx = [
        [true, false, true, true, false, false, true, true],
        [false, true, true, true, false, false, true, true],
        [true, false, true, true, true, true, true, true],
    ];
    let block_slots = [1, 2, 3, 4, 5, 6, 17, 18];
    for (snap, states) in hlx.iter().enumerate() {
        for (slot, want) in block_slots.iter().zip(states) {
            assert_eq!(on(snap, *slot), *want, "snapshot {snap} slot {slot}");
        }
    }
    assert!(!on(0, 6), "the Hall is off in 808");
}

#[test]
fn snapshot_states_of_non_block_slots_are_unexplained() {
    // Input, output and join read false, the split and empty slots true. Raw, not interpreted.
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let states: String = preset.snapshots.list[0].slots.iter().map(|s| if s.enabled { '1' } else { '0' }).collect();
    assert_eq!(states, "01011001101111111110");
    assert!(preset.snapshots.list.iter().flat_map(|s| &s.slots).all(|s| !s.flag));
}

#[test]
fn snapshot_controller_values() {
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let assignments = &preset.controllers;
    assert_eq!(assignments.len(), 3);
    // All three are snapshot assignments (controller 9) on the second EQ (slot 4):
    // Level (param 11), MidFreq (3), MidGain (5), as in the hlx's controller section.
    let by_id = |id: i64| assignments.iter().find(|a| a.id == id).unwrap();
    for (id, param, min, max) in [(0, 11, -60.0, 12.0), (1, 3, 50.0, 20000.0), (2, 5, -15.0, 15.0)] {
        let a = by_id(id);
        assert_eq!((a.controller, a.slot, a.param), (9, 4, param), "assignment {id}");
        assert_eq!((float(&a.min), float(&a.max)), (min, max), "assignment {id}");
    }
    let values = |snap: usize| -> Vec<(i64, f32)> {
        preset.snapshots.list[snap].controllers.iter().map(|c| (c.assignment, float(&c.value))).collect()
    };
    assert_eq!(values(0), [(0, 3.0), (1, 400.0), (2, -1.0)]);
    assert_eq!(values(1), [(0, 3.0), (1, 400.0), (2, -1.0)]);
    assert_eq!(values(2), [(0, 5.0), (1, 800.0), (2, 2.0)]);
}

#[test]
fn current_state_follows_the_active_snapshot() {
    let p02 = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let p05 = Document::decode(&document("05-snapshot-switch")).unwrap().preset().unwrap();
    let block = |p: &nx_proto::Preset, slot| p.blocks().find(|b| b.slot == slot).unwrap().block.clone();
    assert_eq!(p05.snapshots.current, 2);
    assert!(!block(&p02, 5).enabled && !block(&p02, 6).enabled);
    assert!(block(&p05, 5).enabled && block(&p05, 6).enabled, "Lead turns delay and hall on");
    // The snapshot-controlled EQ values are the active snapshot's.
    let eq = |p| {
        let v = block(p, 4).values.values;
        [float(&v[3]), float(&v[5]), float(&v[11])]
    };
    assert_eq!(eq(&p02), [400.0, -1.0, 3.0]);
    assert_eq!(eq(&p05), [800.0, 2.0, 5.0]);
}

#[test]
fn cursor_is_the_selected_block() {
    let p02 = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let p05 = Document::decode(&document("05-snapshot-switch")).unwrap().preset().unwrap();
    assert_eq!((p02.cursor.slot, p02.cursor.path), (1, 0));
    // Capture 04 selected the delay with opcode 78 {98: 5, 26: 0}.
    assert_eq!((p05.cursor.slot, p05.cursor.path), (5, 0));
}

#[test]
fn editing_02_like_captures_03_to_05_reproduces_05() {
    let mut doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    doc.set_param(3, 0, ParamValue::Float(0.6)).unwrap();
    doc.set_param(4, 3, ParamValue::Float(800.0)).unwrap();
    doc.set_param(4, 5, ParamValue::Float(2.0)).unwrap();
    doc.set_param(4, 11, ParamValue::Float(5.0)).unwrap();
    doc.set_enabled(5, true).unwrap();
    doc.set_enabled(6, true).unwrap();
    doc.set_cursor(5, 0).unwrap();
    doc.set_current_snapshot(2).unwrap();
    assert_eq!(doc.encode().unwrap(), document("05-snapshot-switch"));
}

#[test]
fn edits_reject_slots_that_have_no_such_value() {
    let mut doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    assert!(doc.set_param(7, 0, ParamValue::Float(0.0)).is_err(), "slot 7 is empty");
    assert!(doc.set_param(3, 14, ParamValue::Float(0.0)).is_err(), "the amp has 14 values");
    assert!(doc.set_enabled(40, true).is_err());
}

#[test]
fn growing_a_value_moves_the_section_table() {
    // A longer name shifts every later section; the table must follow.
    let bytes = document("02-select-preset-01C");
    let mut doc = Document::decode(&bytes).unwrap();
    doc.set_snapshot_name(0, "Lead Clean").unwrap();
    let grown = doc.encode().unwrap();
    let again = Document::decode(&grown).unwrap();
    assert_eq!(again.preset().unwrap().snapshots.list[0].name, "Lead Clean");
    let delta = grown.len() as u32 - bytes.len() as u32;
    let before = Document::decode(&bytes).unwrap();
    for (old, new) in before.sections().unwrap().iter().zip(again.sections().unwrap()) {
        // Snapshots (key 10) come last, so only the two trailing totals move.
        assert!(new == *old || new == *old + delta);
    }
    assert_eq!(again.sections().unwrap()[10], grown.len() as u32);
}

/// HX Edit's `Helix.sym`, which is Line 6's and not in the repo; `None` (test skipped) without it.
fn helix_sym() -> Option<nx_catalog::ModelSymbols> {
    let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
    let path = dir.join("Helix.sym");
    if !path.exists() {
        eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
        return None;
    }
    Some(nx_catalog::ModelSymbols::load(&path).unwrap())
}

#[test]
fn every_value_matches_the_hlx_by_helix_sym_parameter_name() {
    let Some(syms) = helix_sym() else { return };
    let hlx = nx_hlx::Preset::from_slice(include_bytes!("../../nx-hlx/tests/fixtures/Revv_DM.hlx")).unwrap();
    let preset = Document::decode(&document("02-select-preset-01C")).unwrap().preset().unwrap();
    let mut unknown = Vec::new();
    for b in hlx.blocks() {
        let placed = preset.blocks().find(|p| p.path == b.path as usize && p.position == b.position as usize).unwrap();
        let sym = syms.get(placed.block.model).unwrap();
        assert_eq!(nx_catalog::shared_id(&sym.symbol), b.model, "{}", b.key);
        for (name, value) in &b.params {
            let Some(i) = sym.parameters.iter().position(|p| p == name) else {
                unknown.push(format!("{}.{name}", b.key));
                continue;
            };
            let got = placed.block.values.values[i].as_f64().unwrap();
            let want = value.as_f64().unwrap_or_else(|| if value.as_bool().unwrap() { 1.0 } else { 0.0 });
            assert!((got - want).abs() < 1e-6, "{}.{name}: document {got}, hlx {want}", b.key);
        }
        // Values past the named parameters: `@trails` on the delay and the hall, nothing elsewhere.
        let extra = placed.block.values.values.len() as i64 - placed.block.values.params;
        assert_eq!(extra, i64::from(b.model == "HD2_DelayDuckedDelay" || b.model == "HD2_ReverbHall"), "{}", b.key);
        assert!(placed.block.values.params as usize <= sym.parameters.len(), "{}", b.key);
    }
    // The generator wrote Scale, which only the stereo Ducked Delay has; the device dropped it.
    assert_eq!(unknown, ["block4.Scale"]);
}

#[test]
fn set_param_keeps_the_stored_type_and_encoding() {
    let bytes = document("02-select-preset-01C");
    let mut doc = Document::decode(&bytes).unwrap();
    // Drive is an f32 and Aggression an integer: each keeps its type whichever kind of number arrives.
    doc.set_param(3, 0, ParamValue::Int(1)).unwrap();
    doc.set_param(3, 8, ParamValue::Float(2.0)).unwrap();
    let amp = doc.preset().unwrap().blocks().find(|b| b.slot == 3).unwrap().block.values.values.clone();
    assert_eq!(amp[0], ParamValue::Float(1.0));
    assert_eq!(amp[8], ParamValue::Int(2));
    assert_eq!(doc.encode().unwrap().len(), bytes.len(), "no value changed its width");
}

#[test]
fn set_param_rejects_values_the_stored_type_cannot_hold() {
    let mut doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    let before = doc.clone();
    assert!(doc.set_param(3, 8, ParamValue::Float(1.5)).is_err(), "Aggression is an integer");
    assert!(doc.set_param(3, 8, ParamValue::Int(300)).is_err(), "Aggression is a fixint");
    assert!(doc.set_param(3, 0, ParamValue::Bool(true)).is_err(), "Drive is a number");
    assert!(doc.set_param(5, 13, ParamValue::Float(1.0)).is_err(), "trails is a boolean");
    assert!(doc.set_param(5, 13, ParamValue::Int(1)).is_err(), "trails is a boolean");
    assert!(doc.set_param(3, 0, ParamValue::Other(nx_proto::Item::Nil)).is_err());
    assert!(doc.set_param(3, 0, ParamValue::Int((1 << 40) + 1)).is_err(), "not exact as f32");
    assert_eq!(doc, before, "a rejected edit changes nothing");
    doc.set_param(5, 13, ParamValue::Bool(false)).unwrap();
}

fn table_word(bytes: &[u8], i: usize) -> u32 {
    // fixstr magic (10 bytes), then the str16 head of the table (3 bytes).
    let at = 13 + i * 4;
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn set_table_word(bytes: &mut [u8], at: usize, i: usize, value: u32) {
    bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn a_section_table_that_does_not_match_the_document_is_rejected() {
    let mut bytes = document("02-select-preset-01C");
    let word = table_word(&bytes, 3);
    set_table_word(&mut bytes, 13, 3, word + 1);
    assert!(matches!(Document::decode(&bytes), Err(nx_proto::DocError::TableMismatch { .. })));
}

#[test]
fn the_magic_keeps_its_encoding() {
    // The same document with the magic written as str8 instead of fixstr: one byte longer, so every
    // offset in the table moves by one.
    let captured = document("02-select-preset-01C");
    let mut bytes = vec![0xd9, 9];
    bytes.extend_from_slice(&captured[1..]);
    for i in 0..12 {
        let word = table_word(&captured, i);
        set_table_word(&mut bytes, 14, i, word + 1);
    }
    let doc = Document::decode(&bytes).unwrap();
    assert!(doc.encode().unwrap() == bytes, "the magic went back to fixstr");
}

#[test]
fn sections_are_the_table_encode_writes() {
    let mut doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    doc.set_snapshot_name(0, "Lead Clean").unwrap();
    let encoded = doc.encode().unwrap();
    let written: Vec<u32> = (0..12).map(|i| table_word(&encoded, i)).collect();
    assert_eq!(doc.sections().unwrap(), written);
}

#[test]
fn edits_stay_within_the_documents_own_counts() {
    let mut doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    let before = doc.clone();
    assert!(doc.set_current_snapshot(3).is_err(), "Revv DM has three snapshots");
    assert!(doc.set_cursor(20, 0).is_err(), "twenty slots");
    assert!(doc.set_cursor(5, 2).is_err(), "paths 0 and 1");
    assert!(doc.set_snapshot_name(3, "x").is_err());
    assert_eq!(doc, before);
    doc.set_current_snapshot(2).unwrap();
    doc.set_cursor(19, 1).unwrap();
}

#[test]
fn snapshot_names_are_short_printable_ascii() {
    let mut doc = Document::decode(&document("02-select-preset-01C")).unwrap();
    let before = doc.clone();
    assert!(doc.set_snapshot_name(0, "a\0b").is_err());
    assert!(doc.set_snapshot_name(0, "Crunch\u{e9}").is_err());
    assert!(doc.set_snapshot_name(0, "tab\there").is_err());
    assert!(doc.set_snapshot_name(0, "sixteen chars ok").is_err());
    assert_eq!(doc, before);
    doc.set_snapshot_name(0, "fifteen chars!!").unwrap();
    doc.set_snapshot_name(1, "").unwrap();
    let names: Vec<_> = doc.preset().unwrap().snapshots.list.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["fifteen chars!!", "", "Lead"]);
}
