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

fn sent(request: &Request, txn: u64) -> String {
    let decoded = rmpv::decode::read_value(&mut &request.encode(txn)[..]).unwrap();
    assert_eq!(Request::from_value(&decoded), Some((txn, request.clone())), "{decoded}");
    decoded.to_string()
}

#[test]
fn inserting_a_block_is_the_message_hx_edit_sends() {
    let amp = Request::InsertBlock { block: 1, model: 545, engine: 17, cab: None };
    assert_eq!(sent(&amp, 1009), "{102: 1009, 100: 39, 101: {98: 1, 99: {19: 6, 20: {24: {23: false, 25: 545, 26: -1}, 9: 17, 10: true}}}}");
}

#[test]
fn changing_a_model_is_the_message_hx_edit_sends() {
    let rectifire = Request::ChangeModel { block: 1, model: 18, cab: None };
    assert_eq!(sent(&rectifire, 1010), "{102: 1010, 100: 40, 101: {98: 1, 100: {23: false, 25: 18, 26: -1}}}");
    let amp_cab = Request::ChangeModel { block: 2, model: 38, cab: Some(711) };
    assert_eq!(sent(&amp_cab, 1026), "{102: 1026, 100: 40, 101: {98: 2, 100: {23: true, 25: 38, 26: 711}}}");
}

#[test]
fn a_snapshot_colour_is_the_message_hx_edit_sends() {
    let violet = Request::SetSnapshotColor { index: 0, color: 9 };
    assert_eq!(sent(&violet, 1016), "{102: 1016, 100: 98, 101: {92: 0, 66: 9}}");
}

#[test]
fn renaming_a_snapshot_is_what_hx_edit_sends_in_capture_25() {
    let lead = Request::RenameSnapshot { index: 1, name: "LEAD".into() };
    assert_eq!(sent(&lead, 1020), "{102: 1020, 100: 89, 101: {92: 1, 109: \"LEAD\\0\"}}");
    let Some(dir) = captures_dir() else { return };
    let filter = Filter { endpoints: Some(vec![0x01]), ..Default::default() };
    let mut reassembler = Reassembler::default();
    let mut sent = Vec::new();
    for t in read_transfers(File::open(dir.join("25-rename-snapshot.pcapng")).unwrap(), &filter).unwrap() {
        for frame in split_transfer(&t.data).unwrap() {
            for msg in reassembler.push(&frame).unwrap() {
                if msg.from_host && msg.body.len() > 1
                    && let Some((txn, request)) = Request::from_value(&msg.decode().unwrap())
                {
                    assert_eq!(request.encode(txn), msg.body);
                    sent.push(request);
                }
            }
        }
    }
    assert!(sent.contains(&Request::RenameSnapshot { index: 0, name: "TestShot".into() }), "{sent:?}");
}

#[test]
fn removing_a_block_is_the_message_hx_edit_sends() {
    assert_eq!(sent(&Request::RemoveBlock { block: 1 }, 1014), "{102: 1014, 100: 28, 101: {98: 1}}");
}

#[test]
fn writing_the_edit_buffer_carries_the_document_as_a_str() {
    let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/18-export-preset.doc")).unwrap();
    let request = Request::WriteDocument { document: document.clone() };
    let bytes = request.encode(1013);
    let decoded = rmpv::decode::read_value(&mut &bytes[..]).unwrap();
    let args = decoded.as_map().unwrap().iter().find(|(k, _)| k.as_u64() == Some(101)).unwrap().1.clone();
    let carried = args.as_map().unwrap().iter().find(|(k, _)| k.as_u64() == Some(110)).unwrap().1.clone();
    let rmpv::Value::String(s) = carried else { panic!("not a str") };
    assert_eq!(s.as_bytes(), &document[..]);
    let at = bytes.windows(document.len()).position(|w| w == &document[..]).unwrap();
    assert_eq!(bytes[at - 3], 0xda, "a str16, as HX Edit writes a document of this size");
    assert_eq!(Request::from_value(&decoded), Some((1013, request)));
}

#[test]
fn importing_a_file_to_a_place_is_what_hx_edit_sends_in_capture_19() {
    let Some(dir) = captures_dir() else { return };
    let filter = Filter { endpoints: Some(vec![0x01]), ..Default::default() };
    let mut reassembler = Reassembler::default();
    let mut sent = Vec::new();
    for t in read_transfers(File::open(dir.join("19-import-to-03B.pcapng")).unwrap(), &filter).unwrap() {
        for frame in split_transfer(&t.data).unwrap() {
            for msg in reassembler.push(&frame).unwrap() {
                if msg.from_host && msg.service == 2 && msg.body.len() > 1 {
                    let (txn, request) = Request::from_value(&msg.decode().unwrap()).unwrap();
                    assert_eq!(request.encode(txn), msg.body, "{request:?}");
                    sent.push(request);
                }
            }
        }
    }
    let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/19-import-to-03B.doc")).unwrap();
    let write = Request::WritePreset { setlist: 0, index: 7, name: "Revv Home".into(), document };
    let at = sent.iter().position(|r| *r == write).expect("op 8 with Revv Home's document");
    assert_eq!(sent[at - 1], Request::ControlBegin);
    assert_eq!(sent[at + 1], Request::ControlStart);
}
