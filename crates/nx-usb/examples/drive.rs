//! First write to a real HX Stomp: on preset "Revv DM", turn the amp's Drive up a little with the
//! requests HX Edit sent in capture 03, read it back, wait, and turn it back. Nothing is saved.
//!
//! cargo run -p nx-usb --example drive -- <log dir> [seconds to hold]

mod common;

use std::fs::{self, File};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use common::Logged;
use nx_proto::{Document, ParamValue, Request};
use nx_session::Session;
use nx_usb::UsbTransport;
use rmpv::Value;

const PRESET: &str = "Revv DM";
/// Slot 3 on path 0 holds the amp in Revv DM; 545 is Revv Gen Red in Helix.sym.
const AMP_SLOT: u64 = 3;
const REVV_GEN_RED: u32 = 545;
const DRIVE: u64 = 0;
const STEP: f32 = 0.05;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn drive(session: &Session) -> Result<f32> {
    let bytes = match session.request(Request::PresetDocument)? {
        Value::String(s) => s.as_bytes().to_vec(),
        other => return Err(format!("op 22 reply is not a document: {other}").into()),
    };
    let preset = Document::decode(&bytes)?.preset()?;
    let amp = preset.blocks().find(|b| b.slot as u64 == AMP_SLOT && b.path == 0).ok_or("no block in the amp slot")?;
    if amp.block.model != REVV_GEN_RED {
        return Err(format!("slot {AMP_SLOT} holds model {}, not Revv Gen Red", amp.block.model).into());
    }
    match amp.block.values.values.get(DRIVE as usize) {
        Some(ParamValue::Float(v)) => Ok(*v),
        other => Err(format!("Drive is {other:?}").into()),
    }
}

fn set_drive(session: &Session, value: f32) -> Result<()> {
    let request = Request::SetParam { block: AMP_SLOT, path: 0, param: DRIVE, value: Value::F32(value) };
    let reply = session.request(request)?;
    println!("  set Drive {value:.2}: reply {reply}");
    Ok(())
}

fn main() -> Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "local/live/drive".into()));
    let hold: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    fs::create_dir_all(&dir)?;
    let transport = Logged::new(UsbTransport::open()?, File::create(dir.join("transfers.log"))?);
    let session = Session::connect(transport)?;

    let info = session.request(Request::PresetInfo)?;
    let name = info
        .as_map()
        .and_then(|m| m.iter().find(|(k, _)| k.as_u64() == Some(109)))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.trim_end_matches('\0').to_owned())
        .unwrap_or_default();
    if name != PRESET {
        return Err(format!("preset on the pedal is {name:?}, not {PRESET:?}; nothing written").into());
    }

    let original = drive(&session)?;
    let raised = (original + STEP).min(1.0);
    println!("{PRESET}: Drive {:.1} -> {:.1}", original * 10.0, raised * 10.0);

    session.request(Request::SelectBlock { block: AMP_SLOT, path: 0 })?;
    set_drive(&session, raised)?;
    let outcome = drive(&session).and_then(|now| {
        println!("  read back {:.3}", now);
        if (now - raised).abs() > 1e-4 { Err(format!("read back {now}, expected {raised}").into()) } else { Ok(()) }
    });
    if outcome.is_ok() {
        println!("  holding {hold} s: check the Stomp");
        thread::sleep(Duration::from_secs(hold));
    }

    set_drive(&session, original)?;
    let restored = drive(&session)?;
    println!("  restored, read back {restored:.3}");
    outcome?;
    if (restored - original).abs() > 1e-4 {
        return Err(format!("restore read back {restored}, expected {original}").into());
    }
    session.close()?;
    println!("closed");
    Ok(())
}
