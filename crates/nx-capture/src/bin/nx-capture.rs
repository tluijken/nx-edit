use std::fs::File;
use std::process::ExitCode;

use nx_capture::{Filter, read_transfers};

const USAGE: &str = "usage: nx-capture <file.pcapng> [--bus N] [--device N] [--ep 0x01,0x81] [--messages]";

fn parse_u8(s: &str) -> Option<u8> {
    match s.strip_prefix("0x") {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

fn parse_args(args: &[String]) -> Option<(String, Filter, bool)> {
    let mut path = None;
    let mut filter = Filter::default();
    let mut messages = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--bus" => filter.bus = Some(it.next()?.parse().ok()?),
            "--device" => filter.device = Some(it.next()?.parse().ok()?),
            "--messages" => messages = true,
            "--ep" => filter.endpoints = Some(it.next()?.split(',').map(parse_u8).collect::<Option<_>>()?),
            other if path.is_none() && !other.starts_with("--") => path = Some(other.to_owned()),
            _ => return None,
        }
    }
    Some((path?, filter, messages))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((path, filter, messages)) = parse_args(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let transfers = match File::open(&path).map_err(Into::into).and_then(|f| read_transfers(f, &filter)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let start = transfers.first().map_or(0, |t| t.ts_us);
    if messages {
        print_messages(&transfers, start);
        return ExitCode::SUCCESS;
    }
    for t in &transfers {
        let dir = if t.endpoint & 0x80 != 0 { "IN " } else { "OUT" };
        let hex: String = t.data.iter().map(|b| format!("{b:02x}")).collect();
        println!("{:>10.6}  {dir}  ep{:02x}  {:>5}  {hex}", (t.ts_us - start) as f64 / 1e6, t.endpoint, t.data.len());
    }
    ExitCode::SUCCESS
}

/// One line per reassembled editor message: time, direction, host-side channel, service, body.
fn print_messages(transfers: &[nx_capture::Transfer], start: u64) {
    let mut reassembler = nx_proto::Reassembler::default();
    for t in transfers {
        let Ok(frames) = nx_proto::split_transfer(&t.data) else { continue };
        for frame in frames {
            for msg in reassembler.push(&frame).unwrap_or_default() {
                let channel = if msg.from_host { msg.src } else { msg.dst };
                let dir = if msg.from_host { "->" } else { "<-" };
                let body = msg.decode().map_or_else(|e| format!("<{e}>"), |v| v.to_string());
                println!("{:>10.6}  {dir} {channel:04x} s{:<3} {body}", (t.ts_us - start) as f64 / 1e6, msg.service);
            }
        }
    }
}
