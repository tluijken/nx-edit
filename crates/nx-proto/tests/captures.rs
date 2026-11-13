//! Decodes every capture in `local/captures/`: every transfer must split into frames and every reassembled
//! message must be valid MessagePack. A layering mistake of one byte fails this immediately.

use std::fs::File;
use std::path::PathBuf;

use nx_capture::{Filter, read_transfers};
use nx_proto::{Reassembler, split_transfer};

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

fn captures() -> Option<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(captures_dir()?)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pcapng"))
        .collect();
    files.sort();
    Some(files)
}

#[test]
fn every_capture_decodes_cleanly() {
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..Default::default() };
    let Some(files) = captures() else { return };
    assert!(files.len() >= 6);
    for path in files {
        let transfers = read_transfers(File::open(&path).unwrap(), &filter).unwrap();
        let mut reassembler = Reassembler::default();
        let mut messages = 0;
        for t in &transfers {
            for frame in split_transfer(&t.data).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
                for msg in reassembler.push(&frame).unwrap() {
                    msg.decode().unwrap_or_else(|e| panic!("{}: {e} in {:02x?}", path.display(), msg.body));
                    messages += 1;
                }
            }
        }
        let idle = path.file_name().unwrap().to_string_lossy().starts_with("01-");
        assert!(idle || messages > 0, "{} has no messages", path.display());
    }
}
