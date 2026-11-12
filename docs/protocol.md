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
0  u16 BE  sequence                         [inferred: increments per frame]
2  u8      0
3  u8      kind: 0x04 data, 0x08 ack, 0x0c data+ack, 0x10 keep-alive, 0x02 hello
4  u32 LE  acknowledgement, 0x1000 + bytes received  [map; 0x1000 seen right after HELLO]
8  payload (data kinds only)
```

Implemented in `nx-proto::frame`; `every_capture_decodes_cleanly` walks all 1607 transfers.

## Channels [seen]

| host id | device id | service opened | role |
|---|---|---|---|
| `0x1001` | `0x03ef` | 5, later 2 | device info (`"P33Main"` reply) |
| `0x1080` | `0x03ed` | 6 | RPC: all requests below |
| `0x1002` | `0x03f0` | 4 | notifications from the device |

Opening, per channel (`06-reconnect`, order 1001 → 1080 → 1002 → 1001):

1. host HELLO (format 0x28, body `00000002 00010021 00100000`), device answers with its HELLO;
2. host data message with a one-byte body: the service number; device echoes it.

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
Transaction ids count up per request (1009, 1010, ...). Status 0 = done; status 1 = accepted,
completes later [map: announced by notification 20].

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

## Open questions

- Byte order and meaning of the sequence field; how acknowledgements count.
- The preset document format (op 22) and how a document is written back.
- Ops 76, 83 and the remaining fields of op 23.
- The clean-quit sequence (`06` starts after HX Edit had already quit).
