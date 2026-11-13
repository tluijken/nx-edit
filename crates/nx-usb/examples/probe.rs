//! Read-only first contact with a real HX Stomp: handshake, preset info (op 23), preset document
//! (op 22), close. Every transfer is logged so a failure can be compared with the captures.
//!
//! cargo run -p nx-usb --example probe -- <log dir>

mod common;

use std::fs::{self, File};
use std::path::PathBuf;

use common::Logged;
use nx_proto::{Document, Request};
use nx_session::Session;
use nx_usb::UsbTransport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "local/live".into()));
    fs::create_dir_all(&dir)?;
    let transport = Logged::new(UsbTransport::open()?, File::create(dir.join("transfers.log"))?);
    println!("interface claimed, handshake");

    let session = Session::connect(transport)?;
    println!("device info: {:?}", String::from_utf8_lossy(session.device_info()));

    let info = session.request(Request::PresetInfo)?;
    println!("preset info: {info}");

    let doc = session.request(Request::PresetDocument)?;
    let bytes = match &doc {
        rmpv::Value::String(s) => s.as_bytes().to_vec(),
        rmpv::Value::Binary(b) => b.clone(),
        other => return Err(format!("op 22 reply is not a document: {other}").into()),
    };
    fs::write(dir.join("document.bin"), &bytes)?;
    println!("document: {} bytes", bytes.len());
    match Document::decode(&bytes).and_then(|d| d.preset()) {
        Ok(p) => println!("decoded: {} blocks, snapshots {:?}", p.blocks().count(), p.snapshots.list.iter().map(|s| &s.name).collect::<Vec<_>>()),
        Err(e) => println!("decode failed: {e}"),
    }

    session.close()?;
    println!("closed");
    Ok(())
}
