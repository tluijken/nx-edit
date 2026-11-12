//! Byte strings below are transfers from `captures/` (session 1), copied verbatim.

use nx_proto::{FrameKind, Message, Reassembler, split_transfer};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

const KEEPALIVE_IN: &str = "08000018ef0301100021001065020000";
const SELECT_PRESET_OUT: &str = "1d0000188010ed0300580004051c0000010006000d0000008366cd03f1641465826b006c02000000";
const HELLO_OUT: &str = "0c0000280110ef03000000020001002100100000";

#[test]
fn parses_a_keepalive() {
    let frames = split_transfer(&hex(KEEPALIVE_IN)).unwrap();
    assert_eq!(frames.len(), 1);
    let f = &frames[0];
    assert_eq!((f.src, f.dst), (0x03ef, 0x1001));
    assert_eq!(f.kind(), Some(FrameKind::KeepAlive));
    assert_eq!(f.seq(), Some(0x21));
    assert_eq!(f.ack(), Some(0x0265));
    assert!(f.payload().unwrap().is_empty());
}

#[test]
fn data_frame_payload_excludes_padding() {
    let f = &split_transfer(&hex(SELECT_PRESET_OUT)).unwrap()[0];
    assert_eq!((f.src, f.dst), (0x1080, 0x03ed));
    assert_eq!(f.kind(), Some(FrameKind::Data));
    assert_eq!(f.payload().unwrap().len(), 21);
}

#[test]
fn two_frames_in_one_transfer() {
    let frames = split_transfer(&hex("08000018ef0301100082001065020000080000 18f00302100076001009020000".replace(' ', "").as_str())).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].src, 0x03f0);
}

#[test]
fn hello_frames_keep_their_raw_body() {
    let f = &split_transfer(&hex(HELLO_OUT)).unwrap()[0];
    assert_eq!(f.format, 0x28);
    assert_eq!(f.kind(), Some(FrameKind::Hello));
    assert_eq!(f.body.len(), 12);
}

#[test]
fn host_frames_encode_back_to_the_captured_bytes() {
    for s in [SELECT_PRESET_OUT, HELLO_OUT, "080000180110ef03002100102f1d0000"] {
        let f = &split_transfer(&hex(s)).unwrap()[0];
        assert_eq!(f.encode(), hex(s), "{s}");
    }
}

#[test]
fn truncated_transfer_is_an_error() {
    assert!(split_transfer(&hex("1d0000188010ed0300580004")).is_err());
}

#[test]
fn reassembles_a_select_preset_request() {
    let f = split_transfer(&hex(SELECT_PRESET_OUT)).unwrap().remove(0);
    let mut r = Reassembler::default();
    let msgs: Vec<Message> = r.push(&f).unwrap();
    assert_eq!(msgs.len(), 1);
    let m = &msgs[0];
    assert!(m.from_host);
    assert_eq!(m.service, 6);
    let body = m.decode().unwrap();
    let map = body.as_map().unwrap();
    let get = |k: u64| map.iter().find(|(key, _)| key.as_u64() == Some(k)).map(|(_, v)| v.clone());
    assert_eq!(get(100).unwrap().as_u64(), Some(20));
    assert_eq!(get(102).unwrap().as_u64(), Some(1009));
}

#[test]
fn reassembles_a_message_split_over_frames() {
    // The device's 2812-byte reply in 02-select-preset-01C starts in a frame whose payload holds
    // only the first 256 bytes; a reassembler fed the first frame alone yields nothing yet.
    let first = "08010018ed038010005c0004e50200000000f100fc0a00008366cd03f3670068da0af1a96c362d68656c697800da00303d00000060000000880300008a0300009503000010040000000500002d0500003e00000033";
    let mut padded = hex(first);
    padded.resize(272, 0);
    let f = split_transfer(&padded).unwrap().remove(0);
    let mut r = Reassembler::default();
    assert!(r.push(&f).unwrap().is_empty());
    assert_eq!(r.pending(f.src, f.dst), Some(2812 + 8));
}
