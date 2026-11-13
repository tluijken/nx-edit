//! Read-only first contact with a real HX Stomp: handshake, preset info (op 23), preset document
//! (op 22), close. Every transfer is logged so a failure can be compared with the captures.
//!
//! cargo run -p nx-usb --example probe -- <log dir>

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nx_proto::{Document, Request};
use nx_session::{Session, Transport};
use nx_usb::UsbTransport;

struct Logged<T> {
    inner: T,
    log: File,
    start: Instant,
}

impl<T> Logged<T> {
    fn line(&mut self, dir: &str, data: &[u8]) {
        let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
        _ = writeln!(self.log, "{:10.6} {dir} {hex}", self.start.elapsed().as_secs_f64());
    }
}

impl<T: Transport> Transport for Logged<T> {
    fn write(&mut self, transfer: &[u8]) -> io::Result<()> {
        self.line(">", transfer);
        self.inner.write(transfer)
    }

    fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        let got = self.inner.read(timeout)?;
        if let Some(data) = &got {
            self.line("<", data);
        }
        Ok(got)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "local/live".into()));
    fs::create_dir_all(&dir)?;
    let transport = Logged { inner: UsbTransport::open()?, log: File::create(dir.join("transfers.log"))?, start: Instant::now() };
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
