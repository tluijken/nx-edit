//! [`nx_session::Transport`] over `nusb` for the HX Stomp's editor interface.
//!
//! Never resets the device: a USB reset takes an HX Stomp off the bus and it does not come back
//! without a physical replug [map].

use std::io;
use std::time::Duration;

use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::{Endpoint, Interface, MaybeFuture};
use nx_session::Transport;

pub const VENDOR_ID: u16 = 0x0e41;
pub const HX_STOMP: u16 = 0x4246;
pub const INTERFACE: u8 = 0;
pub const EP_OUT: u8 = 0x01;
pub const EP_IN: u8 = 0x81;

/// Reads kept queued on the IN endpoint, so the device always has somewhere to put what it sends;
/// a device whose IN queue fills stops draining OUT too [map].
const READS_IN_FLIGHT: usize = 4;
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

pub fn is_hx_stomp(vendor_id: u16, product_id: u16) -> bool {
    (vendor_id, product_id) == (VENDOR_ID, HX_STOMP)
}

/// IN transfer size: a multiple of the packet size, as the host controller requires, with room for
/// several frames (the largest frame seen is 272 bytes). A short packet ends the transfer early.
pub fn read_len(max_packet_size: usize) -> usize {
    let packet = max_packet_size.max(1);
    (4096usize.div_ceil(packet)) * packet
}

pub struct UsbTransport {
    _interface: Interface,
    out: Endpoint<Bulk, Out>,
    input: Endpoint<Bulk, In>,
    read_len: usize,
}

impl UsbTransport {
    /// Opens the first HX Stomp found and claims its editor interface. Fails with `Busy` while HX
    /// Edit holds it.
    pub fn open() -> io::Result<Self> {
        let info = nusb::list_devices()
            .wait()?
            .find(|d| is_hx_stomp(d.vendor_id(), d.product_id()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no HX Stomp connected"))?;
        let device = info.open().wait()?;
        let interface = device.claim_interface(INTERFACE).wait()?;
        Self::from_interface(interface)
    }

    pub fn from_interface(interface: Interface) -> io::Result<Self> {
        let out = interface.endpoint::<Bulk, Out>(EP_OUT)?;
        let mut input = interface.endpoint::<Bulk, In>(EP_IN)?;
        let read_len = read_len(input.max_packet_size());
        while input.pending() < READS_IN_FLIGHT {
            input.submit(Buffer::new(read_len));
        }
        Ok(Self { _interface: interface, out, input, read_len })
    }
}

impl Transport for UsbTransport {
    fn write(&mut self, transfer: &[u8]) -> io::Result<()> {
        let done = self.out.transfer_blocking(Buffer::from(transfer), WRITE_TIMEOUT);
        done.status.map_err(io::Error::from)
    }

    fn read(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        let Some(done) = self.input.wait_next_complete(timeout) else { return Ok(None) };
        self.input.submit(Buffer::new(self.read_len));
        done.status.map_err(io::Error::from)?;
        Ok(Some(done.buffer[..done.actual_len].to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_the_hx_stomp() {
        assert!(is_hx_stomp(0x0e41, 0x4246));
        assert!(!is_hx_stomp(0x0e41, 0x4253), "HX Stomp XL is not tested");
        assert!(!is_hx_stomp(0x1234, 0x4246));
    }

    #[test]
    fn reads_are_whole_packets_with_room_for_several_frames() {
        assert_eq!(read_len(512), 4096);
        assert_eq!(read_len(64), 4096);
        assert_eq!(read_len(1000) % 1000, 0);
        assert!(read_len(1000) >= 4096);
    }

    #[test]
    fn never_resets_the_device() {
        let needle = concat!(".re", "set(");
        assert!(!include_str!("lib.rs").contains(needle));
    }
}
