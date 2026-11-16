//! Every RPC request HX Edit sent in `local/captures/` must parse into a `Request` and encode back to the
//! exact bytes HX Edit wrote.

use std::fs::File;
use std::path::PathBuf;

use nx_capture::{Filter, read_transfers};
use nx_proto::{RPC_CHANNEL, Reassembler, Request, split_transfer};

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

fn host_rpc_bodies() -> Option<Vec<(String, Vec<u8>)>> {
    let dir = captures_dir()?;
    let filter = Filter { endpoints: Some(vec![0x01]), ..Default::default() };
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pcapng"))
        .collect();
    files.sort();
    for path in files {
        let mut reassembler = Reassembler::default();
        for t in read_transfers(File::open(&path).unwrap(), &filter).unwrap() {
            for frame in split_transfer(&t.data).unwrap() {
                for msg in reassembler.push(&frame).unwrap() {
                    if msg.src == RPC_CHANNEL && msg.body.len() > 1 {
                        out.push((path.file_name().unwrap().to_string_lossy().into_owned(), msg.body));
                    }
                }
            }
        }
    }
    Some(out)
}

#[test]
fn captured_requests_encode_byte_for_byte() {
    let Some(bodies) = host_rpc_bodies() else { return };
    assert!(bodies.len() > 20);
    let mut known = 0;
    for (file, body) in &bodies {
        let value = rmpv::decode::read_value(&mut &body[..]).unwrap();
        let (txn, request) = Request::from_value(&value).unwrap_or_else(|| panic!("{file}: not a request: {value}"));
        if matches!(request, Request::Other { .. }) {
            continue;
        }
        known += 1;
        assert_eq!(&request.encode(txn), body, "{file}: {value}");
    }
    assert!(known > 20, "{known}");
}

#[test]
fn set_param_builds_the_message_hx_edit_sends() {
    let request = Request::SetParam { block: 3, path: 0, param: 0, value: rmpv::Value::F32(0.46) };
    let decoded = rmpv::decode::read_value(&mut &request.encode(1014)[..]).unwrap();
    assert_eq!(decoded.to_string(), "{102: 1014, 100: 30, 101: {98: 3, 29: true, 26: 0, 28: 0, 119: 0.46}}");
}

#[test]
fn select_preset_and_snapshot() {
    let preset = rmpv::decode::read_value(&mut &Request::SelectPreset { setlist: 0, index: 2 }.encode(1009)[..]).unwrap();
    assert_eq!(preset.to_string(), "{102: 1009, 100: 20, 101: {107: 0, 108: 2}}");
    let snap = rmpv::decode::read_value(&mut &Request::SelectSnapshot { index: 2 }.encode(1035)[..]).unwrap();
    assert_eq!(snap.to_string(), "{102: 1035, 100: 88, 101: {92: 2}}");
}

/// HX Edit saving 03A as "NX Test" in capture 17, the message body as it went out.
const SAVE_03A: [u8; 23] = [
    0x83, 0x66, 0xcd, 0x03, 0xf1, 0x64, 0x47, 0x65, 0x83, 0x6b, 0x00, 0x6c, 0x06, 0x6d, 0xa8, b'N', b'X', b' ', b'T', b'e', b's', b't', 0x00,
];

#[test]
fn save_preset_is_the_message_hx_edit_sends() {
    let request = Request::SavePreset { setlist: 0, index: 6, name: "NX Test".into() };
    assert_eq!(request.encode(1009), SAVE_03A);
    let value = rmpv::decode::read_value(&mut &SAVE_03A[..]).unwrap();
    assert_eq!(Request::from_value(&value), Some((1009, request)));
}

#[test]
fn set_extra_is_the_trails_switch_hx_edit_sends() {
    let request = Request::SetExtra { block: 1, path: 0, index: 0, value: rmpv::Value::Boolean(true) };
    let decoded = rmpv::decode::read_value(&mut &request.encode(1010)[..]).unwrap();
    assert_eq!(decoded.to_string(), "{102: 1010, 100: 30, 101: {98: 1, 29: false, 26: 0, 28: 0, 119: true}}");
    assert_eq!(Request::from_value(&decoded), Some((1010, request)));
}

#[test]
fn set_param_carries_integers_and_booleans_as_they_are() {
    let mic = Request::SetParam { block: 3, path: 0, param: 0, value: rmpv::Value::from(9) };
    let decoded = rmpv::decode::read_value(&mut &mic.encode(1011)[..]).unwrap();
    assert_eq!(decoded.to_string(), "{102: 1011, 100: 30, 101: {98: 3, 29: true, 26: 0, 28: 0, 119: 9}}");
    assert_eq!(Request::from_value(&decoded), Some((1011, mic)));
}
