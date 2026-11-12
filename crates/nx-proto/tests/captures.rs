//! Decodes every capture in `captures/`: every transfer must split into frames and every reassembled
//! message must be valid MessagePack. A layering mistake of one byte fails this immediately.

use std::fs::File;
use std::path::PathBuf;

use nx_capture::{Filter, read_transfers};
use nx_proto::{Reassembler, split_transfer};

fn captures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pcapng"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_capture_decodes_cleanly() {
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let files = captures();
    assert!(files.len() >= 6);
    for path in files {
        let transfers = read_transfers(File::open(&path).unwrap(), &filter).unwrap();
        let mut reassembler = Reassembler::default();
        let mut messages = 0;
        for t in &transfers {
            for frame in split_transfer(&t.data).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
                for msg in reassembler.push(&frame).unwrap() {
                    msg.decode().unwrap_or_else(|e| panic!("{}: {e} in {:02x?}", path.display(), &msg.body));
                    messages += 1;
                }
            }
        }
        let idle = path.file_name().unwrap().to_string_lossy().starts_with("01-");
        assert!(idle || messages > 0, "{} has no messages", path.display());
    }
}
