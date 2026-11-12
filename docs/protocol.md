# HX Stomp editor protocol

What nx-edit knows about the protocol on interface `MI_00`, from its own captures (`captures/`,
session 1: HX Edit 3.82, HX Stomp firmware 3.71.0). tonepush's PROTOCOL.md was used as a map; every
statement here is checked against our captures. Tags: **[seen]** observed in our captures and
covered by a test; **[inferred]** consistent with our captures but not proven; **[map]** from
PROTOCOL.md, not yet seen by us.

## Layer 1: transfers and frames [seen]

Bulk OUT `0x01` / IN `0x81`, 512-byte packets. A transfer holds one or more frames, each padded to a
multiple of 4 bytes (`06-reconnect` has two keep-alives in one 32-byte IN transfer).

```
0  u16 LE  body length (bytes after this 8-byte header)
2  u8      0
3  u8      header format: 0x18 channel frame, 0x28 HELLO
4  u16 LE  source endpoint id
6  u16 LE  destination endpoint id
8  body
```

Channel frame body (format 0x18):

```
0  u8      0
1  u8      sequence, per channel and direction, wraps 0xff -> 0x00   [seen]
2  u8      0
3  u8      kind: 0x04 data, 0x08 ack, 0x0c data+ack, 0x10 keep-alive, 0x02 close  [seen]
4  u32 LE  acknowledgement: receiver's base + payload bytes received  [seen]
8  payload (data kinds only; at most 256 bytes per frame)
```

The HELLO body (format 0x28) is laid out the same way for its first four bytes (seq 0, kind 0x02),
followed by two words; see "Session".

Implemented in `nx-proto::frame`; `every_capture_decodes_cleanly` walks all 1607 transfers.

## Channels [seen]

| host id | device id | service opened | role |
|---|---|---|---|
| `0x1001` | `0x03ef` | 5, later 2 | device info (`"P33Main"` reply) |
| `0x1080` | `0x03ed` | 6 | RPC: all requests below |
| `0x1002` | `0x03f0` | 4 | notifications from the device |

Opening, per channel (`06-reconnect`, order 1001 → 1080 → 1002 → 1001):

1. host HELLO (format 0x28, body `00000002 00010021 00100000`), device answers with its HELLO
   (`00000002 00010001 00020000`);
2. host data message with a one-byte body: the service number; device echoes it.

Before that, at 25.4 s, HX Edit probes `0x1001` with service 5 (the `"P33Main"` reply) and closes it
again about a second later; see "Session".

Idle channels exchange keep-alives (kind 0x10) roughly once a second in both directions.

## Layer 2: messages [seen]

The payload stream of each direction of each channel carries messages, possibly split over frames:

```
0  u16 LE  originator: 1 host, 0 device
2  u16 LE  service (device replies sometimes carry junk here, e.g. 241 on large replies)
4  u32 LE  MessagePack body length
8  MessagePack body
```

Implemented in `nx-proto::message`. Every message in every capture decodes as MessagePack.

## Layer 3: RPC on 0x1080 [seen]

Request `{102: txn, 100: opcode, 101: args}`; reply `{102: txn, 103: status, 104: result}`.
Transaction ids start at 1000 at connect and count up per request, separately for `0x1080` and
`0x1001` (`06`: both channels use 1000, 1001, ...). `0x1001` carries the same request format once
opened with service 2 (ops 254, 0, 1, 13, 112 at connect). Status 0 = done; status 1 = accepted,
completes later with notification 20 `{102: txn, 103: status, 104: result}` [seen, `02`].

| op | meaning | args | reply | capture |
|---|---|---|---|---|
| 20 | select preset | `107` setlist, `108` index | status 1 | 02 |
| 22 | read current preset document | nil | binary document, starts `"l6-helix"` | 02, 05 |
| 23 | current preset info | nil | `107`, `108`, `109` name (NUL-terminated), `117` modified, `83`, `92` snapshot | 02, 05 |
| 24 | read global setting | `118` id | `{118: id, 119: value}` | 05 |
| 30 | set parameter | `98` block, `26` path, `28` param index, `29` true, `119` value | status 0, echoes args | 03 |
| 41 | block on/off | `98` block, `59` enabled | status 0 | 04 |
| 76 | (connect) | nil | map of floats | 06 |
| 78 | select block (editor focus) | `98` block, `26` path | status 0 | 03, 04 |
| 88 | select snapshot | `92` index | status 0 | 05 |

`98` is the block slot counted from 1 [inferred: amp at preset position 2 is 3, delay at 4 is 5].
`28` is the parameter index in the model's parameter order [inferred: Drive is the amp's first
parameter and is 0].

## Notifications on 0x1002 [seen, partly]

`{105: event, 106: data}`. Event 30 follows every set-parameter with the same arguments (`03`).
Notifications need ACK frames or the device stops sending them [map].

## Session

How `nx-session` drives the interface. Evidence is `06-reconnect` (HX Edit's full connect) unless
stated otherwise. Counts are over all frames of all six captures; `crates/nx-session/tests/replay.rs`
checks the session against HX Edit's connect byte for byte.

### Sequence numbers [seen]

Each side keeps its own one-byte counter per channel. HELLO carries 0; the first frame after it
carries 2 (on both sides, every channel, every open); every further frame of any kind (data, ack,
keep-alive, close) adds 1, wrapping from 0xff to 0x00 (`04`, `05`). The two sides' counters are
independent: they often run in step because exchanges are one-to-one, but drift apart on idle
channels (`03`: host 0x56 vs device 0x5c on `0x1002`). Nothing suggests the receiver checks them.

### Acknowledgements [seen; window meaning inferred]

The acknowledgement field is the sender's base plus the payload bytes it has received on that
channel. The base is the last word of the sender's own HELLO: 0x1000 for the host, 0x200 for the
device. Every frame carries the current value, keep-alives included. In `06` the host's first data
frame on each channel carries exactly 0x1000; the device's echo of a 9-byte service open carries
0x209; the host's ACK of a 32-byte reply carries 0x1020. Across the 1600 channel frames in the
captures the host never acknowledges more than it has received; it often lags while frames are in
flight.

[inferred] The base doubles as the receive window: a side may send payload up to the limit its peer
last acknowledged. That explains the device stopping after 4096 bytes of unacknowledged
notifications [map], and the device's 0x200 means the host may have at most 512 bytes in flight.
`nx-session` enforces that window; no capture has a host message large enough to show what HX
Edit does.

Kind bits: 0x04 data, 0x08 ack, 0x0c data that also carries a new acknowledgement. HX Edit sends
0x0c exactly when the acknowledgement changed since its previous frame on that channel and 0x04
otherwise (37 of 37 plain data frames carry no news, 11 of 11 0x0c frames do) [seen]. Keep-alives
never carried news (609 of 609) [seen]; the device does not count them as acknowledgements of
notifications [map], so `nx-session` sends an ACK instead of a keep-alive when one is owed.

When the host acknowledges [seen]:

- every frame of a multi-frame message except the last at once, one ACK per 256-byte frame
  (`02`, `05`, `06`), which keeps the device's view of the window open;
- a completed message either on the next request, if that follows within about 10 ms, or with a
  bare ACK after about 5-15 ms (Windows timer granularity is a likely cause);
- notifications on `0x1002` with bare ACKs, one ACK covering several notifications that arrived
  together (`02`: four notifications, one ACK).

`nx-session` does the same: partial messages at once, complete ones after `ack_delay` (10 ms)
unless a request carries it first. The device answers a bare ACK with its own about 100-150 ms
later [seen]; the host does not answer device ACKs [seen].

### Keep-alives [seen]

A channel on which the host has sent nothing for about 1.05 s gets a keep-alive (kind 0x10, current
acknowledgement); the timer is per channel and restarts with every frame sent (`06`: 1080's
keep-alive follows its last ACK by 1.047 s). The device does the same at about 1.00 s. Neither
side answers the other's keep-alives. `nx-session` uses 1 s. A device stops answering a channel
after four unanswered probes of its own [map]; keeping our own timer running covers that.

### Handshake [seen]

HX Edit's connect, frame for frame, which `nx-session` reproduces byte for byte (the replay test
checks every transfer):

1. HELLO `0x1001`; service 5; ACK; about 1 s later close (kind 0x02) `0x1001`, device answers with
   its own close.
2. HELLO `0x1080`; service 6.
3. HELLO `0x1002`; ACK `0x1080`; service 4.
4. HELLO `0x1001`; ACK `0x1002`; service 2; ACK `0x1001`.

After a close a channel's counters restart from HELLO. The service 5 reply is a 24-byte message
whose body starts with 5 and contains `"P33Main"`. Service 2 is only opened after the probe, as
[map] warns. `nx-session` drains stale transfers before step 1 (bounded) [map: a client that
exits without closing leaves a backlog], reports any timeout or a non-HELLO answer to HELLO as an
error, and never repeats the handshake or sends HELLO on a channel that is open [map: a second
HELLO on an open channel can wedge the device until its power is pulled]. Note that HX Edit itself
sent HELLO on `0x1080` in `06` although the previous session never closed it, and the device
accepted; a fresh session after a quit is not what the map warns about.

### Requests and deferred operations

One request in flight per channel; `0x1080` and `0x1001` run independently, as in `06` [seen].
Replies are matched by transaction id; a reply whose request timed out is dropped. A status 1
reply means accepted: the operation completes with notification 20 carrying the same transaction
id [seen, `02`: status 1 at 16.7391 s, notification 20 at 16.7396 s, next request only after both].
Until it arrives `nx-session` sends no request on any channel [map: racing a running commit wedges
the device; HX Edit waits]. Notification 20 is matched against the one deferred operation in
flight, which also resolves the ambiguity of the two channels' overlapping transaction ids.

### Ending a session [seen, partly]

HX Edit's quit in `06` (0-11 s): close `0x1001`, device answers; about 0.5 s later close `0x1002`,
device answers; `0x1080` is left open (the device kept sending keep-alives on it until reads
stopped). The next connect worked. `Session::close` does the same after acknowledging what
arrived. The map reports that closing every channel kept an HX Stomp's next session from opening;
our capture shows HX Edit closing two of three and the device fine afterwards. Dropping a `Session`
only acknowledges and stops, without closing.

## Open questions

- Byte 0 of the channel frame body has always been 0: the high byte of a 16-bit sequence that never
  got past 0xff, or something else? (Sequences wrap at 0xff, so probably not a counter.)
- The middle word of HELLO (`00010021` host, `00010001` device): version and capabilities?
- Is the HELLO base really a window? A host write larger than 512 bytes (preset upload) would show
  whether HX Edit waits for device ACKs; `nx-session` assumes it must.
- Why HX Edit closes `0x1001` and `0x1002` at quit but not `0x1080`.
- What the device does if the host closes `0x1080`, or if the device closes a channel first.
- Whether non-deferred requests on other channels may run while a deferred operation is pending
  (HX Edit's captures never try; `nx-session` holds them).
- Status values other than 0 and 1.
- The preset document format (op 22) and how a document is written back.
- Ops 76, 83 and the remaining fields of op 23.
- The clean-quit sequence (`06` starts after HX Edit had already quit).
