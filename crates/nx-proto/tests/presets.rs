//! The preset list on the control channel, against capture 06 where HX Edit read it after starting.

use std::fs::File;
use std::path::PathBuf;

use nx_capture::{Filter, read_transfers};
use nx_proto::{PresetEntry, Reassembler, Request, parse_preset_list, slot_label, split_transfer};
use rmpv::Value;

fn encoded(request: Request, txn: u64) -> String {
    rmpv::decode::read_value(&mut &request.encode(txn)[..]).unwrap().to_string()
}

#[test]
fn listing_requests_match_hx_edit_byte_for_byte() {
    assert_eq!(encoded(Request::ControlStart, 1000), "{102: 1000, 100: 254, 101: {}}");
    assert_eq!(encoded(Request::ListSetlists, 1001), "{102: 1001, 100: 0, 101: nil}");
    assert_eq!(encoded(Request::ListPresets { setlist: 0 }, 1002), "{102: 1002, 100: 1, 101: {107: 0, 101: 2}}");
}

#[test]
fn listing_requests_parse_back() {
    for request in [Request::ControlStart, Request::ListSetlists, Request::ListPresets { setlist: 0 }] {
        let value = rmpv::decode::read_value(&mut &request.encode(7)[..]).unwrap();
        assert_eq!(Request::from_value(&value), Some((7, request)));
    }
}

/// The result (key 104) of the control channel reply to op 1 in capture 06.
/// The captures stay out of git; where `local/captures` is absent the tests that need them pass
/// without checking anything.
fn captures_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/captures");
    if dir.exists() {
        Some(dir)
    } else {
        eprintln!("skipped: no {}", dir.display());
        None
    }
}

fn captured_list() -> Option<Value> {
    let path = captures_dir()?.join("06-reconnect.pcapng");
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let mut reassembler = Reassembler::default();
    let mut list_txn = None;
    for t in read_transfers(File::open(path).unwrap(), &filter).unwrap() {
        for frame in split_transfer(&t.data).unwrap() {
            for msg in reassembler.push(&frame).unwrap() {
                let value = msg.decode().unwrap();
                if msg.from_host && msg.src == 0x1001 {
                    if let Some((txn, Request::ListPresets { .. })) = Request::from_value(&value) {
                        list_txn = Some(txn);
                    }
                    continue;
                }
                let Some(map) = value.as_map() else { continue };
                let field = |k| map.iter().find(|(key, _)| key.as_u64() == Some(k)).map(|(_, v)| v.clone());
                if msg.dst == 0x1001 && field(102).and_then(|v| v.as_u64()) == list_txn && list_txn.is_some() {
                    return Some(field(104).unwrap());
                }
            }
        }
    }
    panic!("no preset list in capture 06");
}

#[test]
fn capture_06_lists_every_preset_of_the_stomp() {
    let Some(list) = captured_list() else { return };
    let list = parse_preset_list(&list).unwrap();
    assert_eq!(list.len(), 126);
    let names: Vec<&str> = list.iter().take(6).map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Recto DM", "5150 DM", "Revv DM", "Mark Lead", "Revv Home", "New Preset"]);
    assert_eq!(list[2], PresetEntry { index: 2, name: "Revv DM".into() });
    assert!(list.iter().enumerate().all(|(i, p)| p.index == i as u64));
}

#[test]
fn a_list_that_is_not_a_preset_list_is_an_error() {
    assert!(parse_preset_list(&Value::Nil).is_err());
    assert!(parse_preset_list(&Value::Array(vec![Value::from(1)])).is_err());
}

#[test]
fn slots_are_named_as_on_the_pedal() {
    assert_eq!(slot_label(0), "01A");
    assert_eq!(slot_label(2), "01C");
    assert_eq!(slot_label(3), "02A");
    assert_eq!(slot_label(125), "42C");
}
