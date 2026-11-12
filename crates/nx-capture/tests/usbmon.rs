use nx_capture::{Filter, TransferKind, read_transfers};

const BULK: u8 = 3;
const INTERRUPT: u8 = 1;

/// One usbmon record (`Documentation/usb/usbmon.rst`); `mmapped` selects the 64-byte header of
/// LINKTYPE_USB_LINUX_MMAPPED over the 48-byte one of LINKTYPE_USB_LINUX.
#[allow(clippy::too_many_arguments)]
fn urb(event: u8, kind: u8, ep: u8, dev: u8, bus: u16, ts: (i64, i32), data: &[u8], mmapped: bool) -> Vec<u8> {
    urb_with(0x1234, 0, event, kind, ep, dev, bus, ts, data, mmapped)
}

#[allow(clippy::too_many_arguments)]
fn urb_with(id: u64, status: i32, event: u8, kind: u8, ep: u8, dev: u8, bus: u16, ts: (i64, i32), data: &[u8], mmapped: bool) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend_from_slice(&id.to_le_bytes());
    r.extend_from_slice(&[event, kind, ep, dev]);
    r.extend_from_slice(&bus.to_le_bytes());
    r.extend_from_slice(&[b'-', if data.is_empty() { b'<' } else { 0 }]);
    r.extend_from_slice(&ts.0.to_le_bytes());
    r.extend_from_slice(&ts.1.to_le_bytes());
    r.extend_from_slice(&status.to_le_bytes());
    r.extend_from_slice(&(data.len() as u32).to_le_bytes());
    r.extend_from_slice(&(data.len() as u32).to_le_bytes());
    r.extend_from_slice(&[0; 8]);
    if mmapped {
        r.extend_from_slice(&[0; 16]);
    }
    r.extend_from_slice(data);
    r
}

fn pcap(linktype: u32, records: &[Vec<u8>]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&0xa1b2c3d4u32.to_le_bytes());
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&0i32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&262144u32.to_le_bytes());
    f.extend_from_slice(&linktype.to_le_bytes());
    for r in records {
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&(r.len() as u32).to_le_bytes());
        f.extend_from_slice(&(r.len() as u32).to_le_bytes());
        f.extend_from_slice(r);
    }
    f
}

fn pcapng(linktype: u16, records: &[Vec<u8>]) -> Vec<u8> {
    fn block(kind: u32, body: &[u8]) -> Vec<u8> {
        let mut body = body.to_vec();
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        let len = (body.len() + 12) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&len.to_le_bytes());
        b.extend_from_slice(&body);
        b.extend_from_slice(&len.to_le_bytes());
        b
    }
    let mut shb = Vec::new();
    shb.extend_from_slice(&0x1a2b3c4du32.to_le_bytes());
    shb.extend_from_slice(&1u16.to_le_bytes());
    shb.extend_from_slice(&0u16.to_le_bytes());
    shb.extend_from_slice(&(-1i64).to_le_bytes());
    let mut idb = Vec::new();
    idb.extend_from_slice(&linktype.to_le_bytes());
    idb.extend_from_slice(&0u16.to_le_bytes());
    idb.extend_from_slice(&262144u32.to_le_bytes());
    let mut f = block(0x0A0D0D0A, &shb);
    f.extend(block(1, &idb));
    for r in records {
        let mut epb = Vec::new();
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&(r.len() as u32).to_le_bytes());
        epb.extend_from_slice(&(r.len() as u32).to_le_bytes());
        epb.extend_from_slice(r);
        f.extend(block(6, &epb));
    }
    f
}

fn all() -> Filter {
    Filter { bus: None, device: None, endpoints: None }
}

#[test]
fn keeps_out_submit_and_in_complete_only() {
    let file = pcap(220, &[
        urb(b'S', BULK, 0x01, 42, 3, (1, 0), &[0xaa, 0xbb], true),
        urb(b'C', BULK, 0x01, 42, 3, (1, 10), &[], true),
        urb(b'S', BULK, 0x81, 42, 3, (1, 20), &[], true),
        urb(b'C', BULK, 0x81, 42, 3, (1, 30), &[0xcc], true),
    ]);
    let transfers = read_transfers(&file[..], &all()).unwrap();
    let got: Vec<(u8, Vec<u8>)> = transfers.iter().map(|t| (t.endpoint, t.data.clone())).collect();
    assert_eq!(got, vec![(0x01, vec![0xaa, 0xbb]), (0x81, vec![0xcc])]);
    assert_eq!(transfers[0].kind, TransferKind::Bulk);
    assert_eq!((transfers[0].bus, transfers[0].device), (3, 42));
}

#[test]
fn filters_other_devices() {
    let file = pcap(220, &[
        urb(b'S', BULK, 0x01, 7, 3, (1, 0), &[1], true),
        urb(b'S', BULK, 0x01, 42, 3, (1, 1), &[2], true),
    ]);
    let filter = Filter { device: Some(42), ..all() };
    let transfers = read_transfers(&file[..], &filter).unwrap();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].data, vec![2]);
}

#[test]
fn filters_endpoints() {
    let file = pcap(220, &[
        urb(b'C', INTERRUPT, 0x85, 42, 3, (1, 0), &[9], true),
        urb(b'C', BULK, 0x81, 42, 3, (1, 1), &[8], true),
    ]);
    let filter = Filter { endpoints: Some(vec![0x01, 0x81]), ..all() };
    let transfers = read_transfers(&file[..], &filter).unwrap();
    assert_eq!(transfers.iter().map(|t| t.endpoint).collect::<Vec<_>>(), vec![0x81]);
}

#[test]
fn reads_linktype_220_and_189() {
    for (linktype, mmapped) in [(220, true), (189, false)] {
        let file = pcap(linktype, &[urb(b'S', BULK, 0x01, 42, 3, (1, 0), &[5, 6, 7], mmapped)]);
        let transfers = read_transfers(&file[..], &all()).unwrap();
        assert_eq!(transfers[0].data, vec![5, 6, 7], "linktype {linktype}");
    }
}

#[test]
fn reads_pcapng() {
    let file = pcapng(220, &[urb(b'C', BULK, 0x81, 42, 3, (2, 5), &[1, 2], true)]);
    let transfers = read_transfers(&file[..], &all()).unwrap();
    assert_eq!(transfers[0].data, vec![1, 2]);
    assert_eq!(transfers[0].ts_us, 2_000_005);
}

#[test]
fn preserves_order_by_timestamp() {
    let file = pcap(220, &[
        urb(b'S', BULK, 0x01, 42, 3, (2, 0), &[2], true),
        urb(b'S', BULK, 0x01, 42, 3, (1, 0), &[1], true),
    ]);
    let transfers = read_transfers(&file[..], &all()).unwrap();
    assert_eq!(transfers.iter().map(|t| t.data[0]).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn rejects_other_linktypes() {
    let file = pcap(1, &[vec![0; 60]]);
    assert!(read_transfers(&file[..], &all()).is_err());
}

const ENOENT: i32 = -2;
const EPIPE: i32 = -32;
const EREMOTEIO: i32 = -121;

#[test]
fn drops_cancelled_in_completions() {
    let file = pcap(220, &[
        urb_with(1, ENOENT, b'C', BULK, 0x81, 42, 3, (1, 0), &[], true),
        urb_with(2, 0, b'C', BULK, 0x81, 42, 3, (1, 1), &[7], true),
    ]);
    let transfers = read_transfers(&file[..], &all()).unwrap();
    assert_eq!(transfers.iter().map(|t| t.data.clone()).collect::<Vec<_>>(), vec![vec![7]]);
}

#[test]
fn keeps_short_in_packets() {
    let file = pcap(220, &[urb_with(1, EREMOTEIO, b'C', BULK, 0x81, 42, 3, (1, 0), &[5], true)]);
    assert_eq!(read_transfers(&file[..], &all()).unwrap().len(), 1);
}

#[test]
fn drops_out_submissions_that_failed() {
    let file = pcap(220, &[
        urb_with(1, -115, b'S', BULK, 0x01, 42, 3, (1, 0), &[1], true),
        urb_with(1, EPIPE, b'C', BULK, 0x01, 42, 3, (1, 1), &[], true),
        urb_with(2, -115, b'S', BULK, 0x01, 42, 3, (1, 2), &[2], true),
        urb_with(2, 0, b'C', BULK, 0x01, 42, 3, (1, 3), &[], true),
    ]);
    let transfers = read_transfers(&file[..], &all()).unwrap();
    assert_eq!(transfers.iter().map(|t| t.data.clone()).collect::<Vec<_>>(), vec![vec![2]]);
}

#[test]
fn reused_urb_ids_only_drop_the_failed_submission() {
    let file = pcap(220, &[
        urb_with(7, -115, b'S', BULK, 0x01, 42, 3, (1, 0), &[1], true),
        urb_with(7, EPIPE, b'C', BULK, 0x01, 42, 3, (1, 1), &[], true),
        urb_with(7, -115, b'S', BULK, 0x01, 42, 3, (1, 2), &[2], true),
        urb_with(7, 0, b'C', BULK, 0x01, 42, 3, (1, 3), &[], true),
    ]);
    let transfers = read_transfers(&file[..], &all()).unwrap();
    assert_eq!(transfers.iter().map(|t| t.data.clone()).collect::<Vec<_>>(), vec![vec![2]]);
}
