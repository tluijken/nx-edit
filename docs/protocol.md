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
| 22 | read current preset document | nil | document (str), see "Preset document" | 02, 05, 06 |
| 23 | current preset info | nil | `107`, `108`, `109` name (NUL-terminated), `117` modified, `83`, `92` snapshot | 02, 05 |
| 24 | read global setting | `118` id | `{118: id, 119: value}` | 05 |
| 30 | set parameter | `98` block, `26` path, `28` param index, `29` true, `119` value | status 0, echoes args | 03 |
| 41 | block on/off | `98` block, `59` enabled | status 0 | 04 |
| 76 | (connect) | nil | map of floats | 06 |
| 78 | select block (editor focus) | `98` block, `26` path | status 0 | 03, 04 |
| 88 | select snapshot | `92` index | status 0 | 05 |

`98` is the slot index in the preset document (input is slot 0) [seen: amp at slot 3, delay at 5].
`28` is the parameter index in the model's parameter order [inferred: Drive is the amp's first
parameter and is 0].

## Notifications on 0x1002 [seen, partly]

`{105: event, 106: data}`. Event 30 follows every set-parameter with the same arguments (`03`).
Notifications need ACK frames or the device stops sending them [map].

## Preset document (op 22)

The result of op 22 is a MessagePack **str** (not bin, although it is not UTF-8) of about 2.8 KB
whose bytes are themselves MessagePack [seen]. Three documents were captured, all of 01C Revv DM:
`02` right after selecting it, `05` after the edits of 03–05, and `06` after reconnecting, which is
byte-identical to `05`. Implemented in `nx-proto::document` (`Document`, typed view `Preset`); the
tests are `crates/nx-proto/tests/document.rs`, checked against `nx-hlx/tests/fixtures/Revv_DM.hlx`.

### Envelope [seen]

```
str    "l6-helix\0"                       fixstr, NUL included
str16  section table, 48 bytes = 12 u32 LE (written as str16 even though str8 would do)
map    the preset, 9 integer keys in the order 7 0 1 3 4 2 5 6 10
```

The table holds byte offsets from the start of the document: the preset map, then the **key byte**
of top-level keys 0 1 3 4 2 5 6 7 10 (in that order, not map order), then the total length twice
[seen: `61 96 904 906 917 1040 1280 1325 62 1331 2801 2801`]. The encoder recomputes it; whether the
device checks it on a write is untested. PROTOCOL.md's 10-word form from older firmware [map] is
rejected rather than guessed at.

The inner MessagePack is **not canonical**: top-level key 2 writes its zeros as int16 (`d1 00 00`)
and uint8 (`cc 00`). A codec that re-encodes in shortest form changes the bytes, so the document is
read into a tree that remembers each value's encoding; unedited documents re-encode byte for byte
and edited integers keep their width when they fit [seen]. Strings carry a trailing NUL. Floats are
f32.

The **preset name is not in the document** [seen]; it comes from op 23 (key 109).

### Top-level keys

| key | content | status |
|---|---|---|
| 7 | `{36: "P33\0", 35: 0x03710000 (BCD 3.71), 37: "v3.70-10-ge773a6f\0"}` | [seen] |
| 0 | DSP 0: `{21: 1, 22: [20 slots]}`; 21 not understood | [seen] |
| 1 | DSP 1: nil on the HX Stomp | [seen] |
| 3 | `{7: 2, 8: [nil ×5]}`; probably footswitches (hlx `@pedalstate` is 2) | raw |
| 4 | controller assignments, array of 10 indexed by controller | [seen] |
| 2 | `{0: [13 × int16 0], 1: [13 × [7 × uint8 0]]}` | raw |
| 5 | global: `16` tempo 120.0 [inferred, = hlx `@tempo`], 45–56 likely Variax (46/47 = −0.1 like `@variax_toneknob/volumeknob`), 30, 134 | raw |
| 6 | cursor `{98: slot, 26: path}`: the block selected in the editor | [seen] |
| 10 | snapshots | [seen] |

### Slots (key 0.22) [seen]

Each slot is `{19: kind, 20: body}`; the slot index is what RPC key `98` addresses (op 30 `98: 3`
is the amp at slot 3, op 41/78 `98: 5` the delay at slot 5), which settles the [inferred] note under
Layer 3. Revv DM's 20 slots are `input, 6 blocks, 2 empty, output, split, 6 empty, cab, send, join`.

| kind | meaning | body |
|---|---|---|
| 0 | input | `{5: route, 7: values}`; route = hlx `@input` |
| 1 | output | `{6: route, 7: values}`; route = hlx `@output` |
| 2 | split | `{14: {5: route, 7: values}, 15: {8: model, 13: before-slot, 10: enabled, 7: values}, 18: true}` |
| 3 | join | `{16: {6: route, 7: values}, 17: {8: model, 13: before-slot, 10: enabled, 7: values}, 18: true}` |
| 6 | block | `{24: {23: false, 25: model, 26: -1}, 9: engine, 10: enabled, 11: values, 12: values}` |
| 8 | empty | nil |

- **values** are `{2: count, 3: params, 4: [...]}`, floats in the model's own units (Drive 0.45,
  frequencies in Hz), integers for menu-like parameters (Aggression 1, Mic 0, SyncSelect1 6),
  booleans for switches.
- **Order** is `Helix.sym`'s parameter order for the model. Checked value by value for every block of
  Revv DM against the hlx (`every_value_matches_the_hlx_by_helix_sym_parameter_name`, needs the real
  `Helix.sym`).
- **count vs params**: Ducked Delay 14/13 and Hall 7/6; the extra last value is `true`, the hlx's
  `@trails: true`. Everything else has count = params. The cab (`HD2_CabMicIr_4x12CaliV30`) carries 7
  values although `Helix.sym` lists an eighth, `IrData`. [seen; that the extra value *is* `@trails`
  is inferred from the hlx]
- **model** is an index into HX Edit's `Helix.sym` (833 entries in HX Edit 3.82): 101 Scream 808 Mono,
  131 Parametric EQ Mono, 545 Revv Gen Red, 77 Ducked Delay Mono, 243 Hall Stereo, 693 CabMicIr
  4x12 Cali V30, 250 Send Mono 1, 257 Split Y, 151 Join [seen, `nx-catalog::ModelSymbols`]. The hlx
  names the shared id (`HD2_ReverbHall`); dropping a `Mono`/`Stereo` suffix maps one to the other.
  The hlx asked for a mono Ducked Delay with `Scale` (a stereo-only parameter); the device dropped it.
- **engine** (key 9): 808/EQ 1, amp 17, delay/reverb 8, cab alone 31, send 25 [seen]. PROTOCOL.md
  lists a cab alone as 15 [map]; our Cab Mic IR model is 31.
- **enabled** (block key 10, junction key 10) is the *current* state, which follows the active
  snapshot: delay and hall are off in `02` (snapshot 808) and on in `05` (Lead) [seen]. The
  snapshot-controlled EQ values in slot 4 likewise hold the active snapshot's values.
- **before-slot** (junction key 13): split 7, join 9, i.e. hlx `@position` + 1, the path-0 slot the
  junction sits in front of [seen for this preset].
- **Paths and positions**: path 0 runs between input and output, path 1 between split and join;
  position counts from the slot after the input or the split. The cab (slot 17) and send (18) are
  hlx path 1 positions 6 and 7 [seen, one preset].
- The join stores B Level −60 where the hlx has −120 [seen]; presumably clamped to the device's
  minimum [inferred].
- Not understood: block key 12 (always an empty values map), 24.23 (always false), 24.26 (always −1),
  junction key 18 (true), split 14's empty values, DSP key 21.

### Controller assignments (key 4) [seen]

An array indexed by controller number; Revv DM has nil except index 9 (the snapshot controller, as
in the hlx), an array of three `{0: id, 1: {...}}`:

| field | meaning |
|---|---|
| `0` | assignment id, which snapshot controller values refer to |
| `1.0` | controller (9) |
| `1.2`, `1.3` | min, max (−60/12 Level, 50/20000 MidFreq, −15/15 MidGain) |
| `1.5` | slot (4, the second EQ) [seen, matches PROTOCOL.md] |
| `1.6.29` | parameter index (11 Level, 3 MidFreq, 5 MidGain) |
| `1.1` (4), `1.4` (0), `1.6.28` (0), `1.6.41` (false), `1.7` (0), `1.13` (false) | not understood |

### Snapshots (key 10) [seen]

`{6: current, 7: 0, 8: 3 (count), 9: 20 (slot count), 10: [snapshots], 13: [20 × true]}`. Current is
0 in `02` and 2 in `05` (op 88 `92: 2`). Keys 7 and 13 are not understood.

Each snapshot is `{0: valid, 1: [11 × [13, false, [7 × 0]]], 2: [64 controller values], 3: [20 slot
states], 4: name, 14: custom name, 5: tempo, 12: LED colour}`:

- names `"808\0"`, `"33\0"`, `"Lead\0"`; 14 true and 12 = 0 match the hlx `@custom_name` and
  `@ledcolor` [inferred from one preset; PROTOCOL.md reads 14 differently, as "renamed on the
  device", which does not fit our imported preset];
- **3** is one `[flag, enabled]` per slot. `enabled` matches the hlx snapshot `blocks` for every block
  of all three snapshots (the Hall is off in 808, the 808 off in 33, delay and hall on in Lead).
  `flag` is false everywhere. Non-block slots read: input false, empty slots and split true, output
  and join false — not interpreted;
- **2** is 64 `[flag, assignment, value]`; unused entries have assignment 64 and value nil. Used
  entries hold the snapshot's value for that assignment: 808 and 33 `Level 3, MidFreq 400, MidGain −1`,
  Lead `5, 800, 2`, as in the hlx. `flag` is false (hlx `@fs_enabled` is false) [inferred];
- **1** is not understood.

### Editing [seen]

`Document::set_param`, `set_enabled`, `set_cursor`, `set_current_snapshot` and `set_snapshot_name`
edit the tree in place. Applying to `02` what captures 03–05 did (Drive 0.6, the Lead EQ values,
delay and hall on, cursor on slot 5, current snapshot 2) and encoding gives `05` byte for byte. How a
document is *written* to the device has not been captured.

Note: capture 03 ends with Drive **0.6** (`SetParam 119: 0.6`, and the document agrees), not about
0.55 as `captures/README.md` estimates.

## Open questions

- Byte order and meaning of the sequence field; how acknowledgements count.
- How a preset document is written back (opcode, whether the section table is checked).
- Preset document: top-level keys 2 and 3, most of 5, snapshot key 1, snapshot-list keys 7 and 13,
  block keys 12, 24.23, 24.26, junction key 18, and the first element of slot states and controller
  values. Captures that change footswitches, MIDI, Command Center, tempo and snapshot colour would
  pin these down.
- Whether `before-slot` and path positions hold for presets with other split positions, and what
  the 10-word section table looks like on this device.
- Cab engine class 31 versus PROTOCOL.md's 15: a different cab type, or firmware-specific.
- Op 30 sends the parameter index under `28` (with `29: true`), while a controller assignment in the
  document keeps it under `6.29` (with `6.28: 0`). Drive is index 0, so capture 03 cannot tell which
  key carries the index; a capture that edits a parameter other than the first would.
- Ops 76, 83 and the remaining fields of op 23.
- The clean-quit sequence (`06` starts after HX Edit had already quit).
