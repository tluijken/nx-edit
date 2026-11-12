//! Reads Linux usbmon captures (pcap or pcapng, as written by `dumpcap -i usbmonN`) into the USB
//! transfers one device exchanged. Used to study HX Edit's traffic and to build protocol fixtures.

mod usbmon;

pub use usbmon::{CaptureError, Filter, Transfer, TransferKind, read_transfers};
