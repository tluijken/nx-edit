use std::io::Read;

use pcap_file::DataLink;
use pcap_file::pcap::PcapReader;
use pcap_file::pcapng::{Block, PcapNgReader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub ts_us: u64,
    pub bus: u16,
    pub device: u8,
    /// Endpoint address including the direction bit (0x80 = IN).
    pub endpoint: u8,
    pub kind: TransferKind,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Iso,
    Interrupt,
    Control,
    Bulk,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub bus: Option<u16>,
    pub device: Option<u8>,
    pub endpoints: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("reading capture: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing capture: {0}")]
    Pcap(#[from] pcap_file::PcapError),
    #[error("not a usbmon capture (link type {0})")]
    LinkType(u32),
}

const PCAPNG_MAGIC: [u8; 4] = [0x0a, 0x0d, 0x0d, 0x0a];

pub fn read_transfers(mut r: impl Read, filter: &Filter) -> Result<Vec<Transfer>, CaptureError> {
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes)?;
    let mut transfers = Vec::new();
    if bytes.starts_with(&PCAPNG_MAGIC) {
        let mut reader = PcapNgReader::new(&bytes[..])?;
        let mut linktypes = Vec::new();
        while let Some(block) = reader.next_block() {
            match block? {
                Block::InterfaceDescription(interface) => linktypes.push(u32::from(interface.linktype)),
                Block::EnhancedPacket(packet) => {
                    let linktype = linktypes.get(packet.interface_id as usize).copied().unwrap_or(0);
                    transfers.extend(parse_record(&packet.data, header_len(linktype)?, filter));
                }
                _ => {}
            }
        }
    } else {
        let mut reader = PcapReader::new(&bytes[..])?;
        let header_len = header_len(u32::from(reader.header().datalink))?;
        while let Some(packet) = reader.next_packet() {
            transfers.extend(parse_record(&packet?.data, header_len, filter));
        }
    }
    transfers.sort_by_key(|t| t.ts_us);
    Ok(transfers)
}

fn header_len(linktype: u32) -> Result<usize, CaptureError> {
    match DataLink::from(linktype) {
        DataLink::USB_LINUX => Ok(48),
        DataLink::USB_LINUX_MMAPPED => Ok(64),
        _ => Err(CaptureError::LinkType(linktype)),
    }
}

/// One usbmon record; layout per the kernel's `Documentation/usb/usbmon.rst` (binary API).
/// OUT payloads travel with the submission, IN payloads with the completion; other events carry no
/// data of interest and are skipped.
fn parse_record(rec: &[u8], header_len: usize, filter: &Filter) -> Option<Transfer> {
    if rec.len() < header_len {
        return None;
    }
    let event = rec[8];
    let kind = match rec[9] {
        0 => TransferKind::Iso,
        1 => TransferKind::Interrupt,
        2 => TransferKind::Control,
        _ => TransferKind::Bulk,
    };
    let endpoint = rec[10];
    let device = rec[11];
    let bus = u16::from_le_bytes([rec[12], rec[13]]);
    let wanted_event = if endpoint & 0x80 != 0 { b'C' } else { b'S' };
    if event != wanted_event
        || filter.bus.is_some_and(|b| b != bus)
        || filter.device.is_some_and(|d| d != device)
        || filter.endpoints.as_ref().is_some_and(|eps| !eps.contains(&endpoint))
    {
        return None;
    }
    let ts_sec = i64::from_le_bytes(rec[16..24].try_into().ok()?);
    let ts_usec = i32::from_le_bytes(rec[24..28].try_into().ok()?);
    let len_cap = u32::from_le_bytes(rec[36..40].try_into().ok()?) as usize;
    let data = &rec[header_len..];
    Some(Transfer {
        ts_us: (ts_sec.max(0) as u64) * 1_000_000 + ts_usec.max(0) as u64,
        bus,
        device,
        endpoint,
        kind,
        data: data[..len_cap.min(data.len())].to_vec(),
    })
}
