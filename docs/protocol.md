# HX Stomp editor protocol

What nx-edit knows about the protocol on interface `MI_00`, from its own captures
(`local/captures/`, kept out of git; HX Edit 3.82, HX Stomp firmware 3.71.0; session 1 is `01`-`06`,
session 2 of 2026-11-16 is `07`-`24`, see [capture.md](capture.md)).
tonepush's PROTOCOL.md was used as a map; every statement here is checked against our captures.
Tags: **[seen]** observed in our captures and covered by a test; **[inferred]** consistent with
our captures but not proven; **[map]** from PROTOCOL.md, not yet seen by us.

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
8  payload (data kinds only)
```

The device's frames carry at most 256 payload bytes. HX Edit's carry up to 496: in `19` it sends a
2.9 KB message as 496 + 16, 496 + 16, ... so that each pair fills the 512-byte window exactly (see
"Acknowledgements") [seen].

The HELLO body (format 0x28) is laid out the same way for its first four bytes (seq 0, kind 0x02),
followed by two words; see "Session".

Implemented in `nx-proto::frame`; `every_capture_decodes_cleanly` walks every transfer of every
capture, session 1 (1607 transfers; the counts below are session 1's) and session 2 alike.

## Channels [seen]

| host id | device id | service opened | role |
|---|---|---|---|
| `0x1001` | `0x03ef` | 5, later 2 | device info (`"P33Main"` reply), then the librarian |
| `0x1080` | `0x03ed` | 6 | RPC: the editing requests below |
| `0x1002` | `0x03f0` | 4 | notifications from the device |

The device ids are fixed; the host ids are HX Edit's choice and can change on every connect
[seen]: session 1, `07`, `25` and later use the ids above, while the connects in between used
`0x1003` (service 6), `0x1004` (service 2) and `0x1005` (service 4), within one run of HX Edit as
well. The device accepts either; the service number, not the host id, says what a channel is for.

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
| 20 | select preset | `107` setlist, `108` index | status 1 | 02, 07 |
| 21 | write the current preset document | `110` document (str, as op 22 returns it) | status 1; notification 20, then 21 | 11 |
| 22 | read current preset document | nil | document (str), see "Preset document" | 02, 05, 06 |
| 23 | current preset info | nil | `107`, `108`, `109` name (NUL-terminated), `117` modified, `83`, `92` snapshot | 02, 05 |
| 24 | read global setting | `118` id | `{118: id, 119: value}` | 05, 22 |
| 25 | write global setting | `118` id, `119` value | status 0, nil; notification 22 | 22 |
| 28 | remove block | `98` slot | status 0, nil; notification 49 subtype 14 | 14 |
| 30 | set parameter | `98` block, `26` sub-block (not the path, see below), `28` param index, `29` true, `119` value | status 0, echoes args | 03, 12, 13 |
| 39 | insert block into an empty slot | `98` slot, `99` slot body `{19: 6, 20: {24: {23: false, 25: model, 26: -1}, 9: engine, 10: true}}` | `{13: slot, 24: the full slot}`, values at the model's defaults; notification 49 subtype 10 | 08, 10, 13 |
| 40 | change a block's model | `98` slot, `100` `{23: false, 25: model, 26: -1}` | as op 39; notifications 49 subtypes 47 and 10, 39 | 09 |
| 41 | block on/off | `98` block, `59` enabled | status 0 | 04 |
| 71 | save the current preset to a place | `107` setlist, `108` index, `109` name | status 0, nil; notifications 5 and 6 | 17 |
| 76 | (connect) | nil | map of floats | 06 |
| 78 | select block (editor focus) | `98` block, `26` sub-block | status 0 | 03, 04, 10, 13, 14 |
| 85 | read all global settings | `{}` | one MessagePack blob of `{id: value}` maps | 23 |
| 88 | select snapshot | `92` index | status 0 | 05 |
| 89 | rename snapshot | `92` index, `109` name (NUL-terminated) | status 0, nil; notification 43 | 25 |
| 90 | copy the current snapshot (on the pedal) | `{}` | status 0, nil; notification 44 `{92: 0, 93: 0}` | 28 |
| 91 | paste into the current snapshot | `{}` | status 0, nil; notification 44 `{92: 1, 93: 1}`, then 42, 23, 46 as for a switch | 28 |
| 98 | set snapshot LED colour | `92` index, `66` colour | status 0, nil; notification 48 | 27 |

Copying a snapshot to another (`28`) happens on the pedal: op 90 copies the current snapshot, HX
Edit selects the target with op 88, and op 91 pastes into it. The target takes the source's name as
well; the document read afterwards has "TestShot" twice [seen].

Moving a block (`11`) has no op of its own: HX Edit edits the document and writes the whole of it
back with op 21 [seen]. Status 1 arrives while the 2.4 KB document is still being sent; notification
20 for the transaction follows, then notification 21 (nil) [seen]. The document it writes is in HX
Edit's own form, as with op 8 (see "A document as HX Edit writes it"): compared with the one the
pedal returned after the reconnect in `12`, it has key 5's extra keys, 13-entry snapshot lists and
nil slot-state flags, and the junctions' before-slot is 1 and 9 where the pedal stores 0 [seen].
`Request::WriteDocument` encodes `11`'s document to the very bytes HX Edit sent. nx-edit moves a
block by swapping two slots of the pedal's own document (`Document::swap_slots`, which also moves
the snapshot slot states, controller assignments and cursor) and writing that back; whether the
pedal accepts its own form there has not been tried.

The trails switch of a delay (`12`) is op 30 with `29: false, 28: 0` on the delay's slot, value
`true`/`false` [seen]. `28` is the parameter index in the model's order: turning the Cali
Rectifire's Bass (its second parameter) in `26` sends `28: 1`, values 0.26-0.28, which HX Edit shows
as 2.6-2.8 [seen]. `29: true` addresses the model's parameters, `29: false` apparently the values
after them: a Simple Delay holds 7 values for 6 parameters, the 7th being trails (see "count vs
params") [inferred]; nx-edit's own switch of a Ducked Delay's trails this way was accepted and
echoed [seen, `30`]. A list parameter is an integer: the cab's Mic is `28: 0`, set to 9 and then 8
in `13`, and the export holds `Mic: 8` [seen]. A switch is a boolean: Tempo Sync `28: 11, 119:
true` [seen, `30`].

New models in session 2: 18 Cali Rectifire (engine 17), 80 Simple Delay (engine 8, 7 values for 6
parameters), 693 Cab Mic IR 4x12 Cali V30 (engine 31) [seen].

### The librarian channel (service 2) [seen]

Stored presets, as opposed to the one being edited, go over the service 2 channel with the same
request format and its own transaction ids. Deferred operations there complete with notification 1
`{102: txn, 103: status, 104: result}`, not 20. Export, import, copy and backup are bracketed by
op 255 and op 254 (a rename is not), and after op 8 HX Edit reads the preset list again with op 1.
At connect HX Edit sends 254, 0, 1, 112 and 13 here, the ops the Layer 3 introduction lists.

| op | meaning | args | reply | capture |
|---|---|---|---|---|
| 0 | setlists | nil | `[{0: "PRESETS\0"}]` | 12 |
| 1 | preset list | `107` setlist, `101: 2` | `[{index: {109: name, 123: false, 124: false, 125: 0}}]` | 19, 20 |
| 4 | read a stored preset document | `107`, `108`, `101: 2` | document (str) | 18, 20, 23 |
| 6 | rename a stored preset | `107`, `108`, `109` name | status 1; notification 6, then 1 | 15, 16 |
| 8 | write a preset to a place | `107`, `108`, `109` name, `123`, `124`, `125` (as op 1 lists them), `110` document | status 1; notification 6, then 1 | 19, 20 |
| 109 | not understood | `64` id, `106` bool, sometimes `105` number | status 0, nil | 23 |
| 254 | end of a librarian operation | `{}` | status 0 | 18, 19, 20, 23 |
| 255 | start of a librarian operation | `{}` | status 0; took 1.07 s in `18`, 2 ms elsewhere | 18, 19, 20, 23 |

- **Export** (`18`) is op 255, op 4 on the place, op 254; HX Edit builds the `.hlx` from the
  document itself [seen].
- **Import to a place** (`19`) is op 255, op 8 with the file's preset, op 254, op 1. The device
  answers status 1 and sends notification 6 with the new name before the 2.9 KB message has fully
  arrived; notification 1 follows the last frame [seen].
- **Copy and paste** (`20`) reads the source with op 4 and writes it to the target with op 8; the
  pedal does not copy internally [seen].
- **Backup** (`23`) is op 85 (globals), op 255, 1116 × op 109 (three per id, 372 ids), 126 × op 4
  (every place, 0-125), op 254. The `.hxb` HX Edit wrote is 10.8 KB for all of that, so it is
  compressed [inferred].
- **Rename** (`15`, `16`) is op 6. `15` was meant as a snapshot rename but renamed the preset, to
  "Test"; renaming a snapshot is RPC op 89 (`25`).

Saving the preset being edited (`17`) is op 71 on the RPC channel, not a librarian write: the
device stores its own edit buffer under the given name, so nothing but the place and name travels.

`98` is the slot index in the preset document (input is slot 0) [seen: amp at slot 3, delay at 5].

`26` is not the path [seen, live session 2026-11-13]: the slot number already places a block on
path A or B, and the pedal reports its own selection of the path B cab (slot 17) as `{98: 17, 26: 0}`.
Sending `{98: 17, 26: 1}` made the pedal select a second cab that the single cab block does not have
(its screen showed None), and op 30 on it failed with status 255 `{111: -3}`. It is probably the
sub-block of a dual cab or an amp+cab [inferred]; nx-edit sends 0 until a capture shows HX Edit
sending anything else.
`28` is the parameter index in the model's parameter order [seen: Drive, the amp's first parameter,
is 0 in `03`; Bass, its second, is 1 in `26`].

## Notifications on 0x1002 [seen, partly]

`{105: event, 106: data}`; most events wrap their own data as `{82, 68, 121: subtype, 106: data}`.
Notifications need ACK frames or the device stops sending them [map].

| event | data | when | source |
|---|---|---|---|
| 1 | `{102: txn, 103: status, 104: result}` | a deferred librarian operation (service 2) completed | 15, 16, 19, 20 |
| 4 | `{107: setlist, 108: index}` | the pedal has switched preset; HX Edit then reads op 23 and op 22 | 02, 07, 24 |
| 5 | `{107, 108, 116: u32}` | a preset was saved to a place (op 71); 116 not understood | 17 |
| 6 | `{107, 108, 109: name}` | a place's name changed: rename, save, import, paste | 15, 16, 17, 19, 20 |
| 8 | `{107, 108}` | a preset switch is under way, before 4 | 02, 07, 24 |
| 20 | `{102: txn, 103: status, 104: result}` | a deferred operation completed | 02, 11 |
| 21 | nil | after notification 20 for an op 21 document write | 11 |
| 22 | `{118: id, 119: value}` | a global setting changed (see "Global settings") | 02, live-05, 07, 22, 24 |
| 23 | `{23: 0}` | after a snapshot switch; also six times while footswitches were pressed in snapshot mode; twice during a preset load | 05, live-06, 07 |
| 25 | nil | during a preset load, after each 23 | 07, 24 |
| 27 | subtype 23 `{98: 0, 51: 1}` | during a preset load; not understood | 07, 24 |
| 30 | `{98, 29, 26, 28, 119}` as op 30 | a parameter changed, by op 30 (an echo) or by a knob on the pedal | 03, live-05, 12, 13 |
| 39 | `{98: slot, 26: sub}` | a block was selected, by op 78 or on the pedal, or after a model change | 04, live-05, 09 |
| 42 | `{92: index}` | a snapshot switch began | 05 |
| 43 | `{92: index, 109: name}` | a snapshot was renamed (op 89) | 25 |
| 44 | `{92, 93}` | a snapshot was copied (op 90: `0, 0`) or pasted (op 91: `1, 1`, the target being snapshot 1); what 92 and 93 each mean is not settled | 28 |
| 48 | `{92: index, 66: colour}` | a snapshot's LED colour was set (op 98) | 27 |
| 46 | `{92: index}` | a snapshot finished loading; HX Edit then reads op 23 and op 22 | 05 |
| 49 | subtype 17 `{98, 59: enabled}`, 10 `{98}`, 47 nil, 14 `{98}`, 11 `{56: 0, 64: 257}` | a block was switched on or off (17), a model was set by insert or change (10, after 47 on a change), a block was removed (14); 11 during a preset load, not understood | 04, live-05, 07-10, 14, 30 |

nx-edit reads the preset again after 4, 46 and 49, and applies 30 to the knob in place.

### Global settings [seen, session 2]

Read with op 24, written with op 25, reported by notification 22; all take `{118: id, 119: value}`.

| id | meaning | values | source |
|---|---|---|---|
| 16 | tempo | 120 | 07, 24 |
| 19 | not identified | became true while the pedal was handled by hand, 1.4 s before its block selection moved along the chain [inferred: the pedal's edit view] | live-05, 30 |
| 21 | footswitch mode | 2 after FS1+FS2 (FS1+FS2 then steps through presets); 3 again after a single footswitch picked a snapshot | 24 |
| 28 | current preset index | 6 on selecting 03A, 88 and 3 on footswitch preset changes | 07, 24, 30 |
| 95 | Device Settings > EXP/FS Tip | bool: false EXP 1, true FS4 | 22 |
| 96-99 | read together with 95 when HX Edit opens its settings: true, 0, 10, 11 | not identified | 22 |

HX Edit's View settings (block name labels) stay in the application and send nothing [seen, `22`].
Holding FS1+FS2 too long in `24-footswitch-mode-long-press` set 21 to 2, then switched to preset 88
(30B), after which 21 read 3.

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
device checks it on a write is untested. `Document::decode` rejects a document whose table differs
from the one the encoder would write, so an unedited round trip never changes it, and the encoder
keeps the length forms the magic and the table were read with. PROTOCOL.md's 10-word form from older firmware [map] is
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
| 6 | cursor `{98: slot, 26: sub-block}`: the block selected in the editor | [seen] |
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
| 6 | block | `{24: {23: amp+cab, 25: model, 26: cab model or -1}, 9: engine, 10: enabled, 11: values, 12: the cab's values}` |
| 7 | looper | `{8: model, 9: 22, 10: enabled, 7: values}` [seen, `31`]; `nx-proto` reads it as a block |
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
- An Amp+Cab block (`31`): `24.23` is true, `24.26` the cab's model (711, 4x10 Tweed P10R, with the
  Tweed Blues Nrm amp) and key 12 the cab's 7 values; a lone amp or other block has false, -1 and an
  empty key 12, as every block in session 1 had [seen].
- Not understood: junction key 18 (true), split 14's empty values, DSP key 21.

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
- **12**, the LED colour, numbers HX Edit's menu from Auto: Red is 2 and Violet 9 [seen, op 98 in
  `27`], which puts Auto 0, White 1, Dark Orange 3, Light Orange 4, Yellow 5, Green 6, Turquoise 7,
  Blue 8, Pink 10, Off 11 [inferred from the menu order];
- **3** is one `[flag, enabled]` per slot. `enabled` matches the hlx snapshot `blocks` for every block
  of all three snapshots (the Hall is off in 808, the 808 off in 33, delay and hall on in Lead).
  `flag` is false everywhere. Non-block slots read: input false, empty slots and split true, output
  and join false — not interpreted;
- **2** is 64 `[flag, assignment, value]`; unused entries have assignment 64 and value nil. Used
  entries hold the snapshot's value for that assignment: 808 and 33 `Level 3, MidFreq 400, MidGain −1`,
  Lead `5, 800, 2`, as in the hlx. `flag` is false (hlx `@fs_enabled` is false) [inferred];
- **1** is not understood.

### Compared with HX Edit's export [seen, `18`]

`18-export-preset.hlx` is HX Edit's export of 03A (Cali Rectifire, 4x12 Cali V30 cab, no path B),
and the document HX Edit read for it (op 4) is in the same capture. Converted with
`DevicePreset::to_hlx`, every key nx-edit writes is also in the export, with the same value. Two
differences were found and fixed (the fixture is `nx-proto/tests/fixtures/18-export-preset.doc`):

- **Floats.** HX Edit rounds to six significant digits, as printf's `%g` does: the pedal's f32
  `0.68000007` is exported as `0.680`, `19.899999` as `19.90` (it pads trailing zeros, which JSON
  readers ignore). nx-edit had written the f32 widened to f64 (`0.6800000667572021`).
- **Junctions without a path B.** Split and join both hold `before_slot: 0`; HX Edit exports split
  0 and join 8, where nx-edit had written `before_slot - 1 = -1`. With a real split (Revv DM: 7 and
  9) both write `before_slot - 1`. So 0 means "not placed" [inferred], and the join's default is
  after path A's last position.

What the export has and nx-edit does not write yet (72 of the export's 132 values):

| hlx | export | document |
|---|---|---|
| `inputA` | `@model HelixStomp_AppDSPFlowInput`, `@input 1`, `noiseGate false`, `threshold -48.0`, `decay 0.10` | input slot: route 1, values `[false, -48.0, 0.1]` |
| `inputB` | same model, `@input 0`, `decay 0.50` | the split's endpoint, route 0 |
| `outputA` | `@model HelixStomp_AppDSPFlowOutputMain`, `@output 1`, `pan 0.50`, `gain 0.0` | output slot: route 1, values `[0.5, 0.0]` |
| `outputB` | `@model HelixStomp_AppDSPFlowOutputSend`, `@output 0`, `Type true`, `pan`, `gain` | the join's endpoint |
| `split` | `BalanceA 0.50`, `BalanceB 0.50`, `bypass false`, `@no_snapshot_bypass false` | split values `[0.5, 0.5, false]` |
| `join` | `A Level 0.0`, `A Pan 0.50`, `B Level 0.0`, `B Pan 0.50`, `B Polarity false`, `Level 3.0`, `@no_snapshot_bypass false` | join values `[0.0, 0.5, 0.0, 0.5, false, 3.0]` |
| blocks | `@type` (amp 1, cab 2), `@no_snapshot_bypass false`, amp `@bypassvolume 1.0` | not located; engine is 17 and 31 |
| `global` | `@tempo 120.0`, `@pedalstate 2`, `@topology0 "A"`, `@topology1 0`, `@cursor_dsp/path/position 0`, `@cursor_group ""`, `@DtSelect 2`, `@PowercabMode 0`, `@PowercabSelect 2`, `@PowercabVoicing 0`, `@guitarinputZ 0`, `@guitarpad 0`, `@model "@global_params"` | tempo is key 5.16, pedal state 3.7, cursor key 6; the rest not located |
| snapshots | `@pedalstate 0` each | not located |
| `variax` | `@variax_model 0`, `magmode true`, `toneknob`/`volumeknob -0.10`, six string tunings 0, `customtuning false`, `lockctrls 0` | key 5, 45-56 [inferred, see "Top-level keys"] |
| `meta` | `application "HX Edit"`, `appversion`, `build_sha` (= document key 7.37), `modifieddate`; top-level `meta.original/pbn/premium 0` | the application's own |

The document holds the endpoint and junction values in the order noiseGate, threshold, decay
(input), pan, gain (output), BalanceA, BalanceB, bypass (split) and A Level, A Pan, B Level, B Pan,
B Polarity, Level (join), presumably `Helix.sym`'s order as for blocks [inferred from this one
preset; the export lists its keys alphabetically]. `dsp1` is `{}` and there is no `footswitch` or `controller` key
in the export.

### Editing [seen]

`Document::set_param`, `set_enabled`, `set_cursor`, `set_current_snapshot` and `set_snapshot_name`
edit the tree in place. Applying to `02` what captures 03–05 did (Drive 0.6, the Lead EQ values,
delay and hall on, cursor on slot 5, current snapshot 2) and encoding gives `05` byte for byte.

A document is written with op 21 (the edit buffer) or librarian op 8 (a place), both as a str under
key `110` with the same envelope and section table as op 22 returns [seen, `11`, `19`, `20`]. HX
Edit's own documents always carry a correct table, so whether the device checks it is still
untested.

### A document as HX Edit writes it [seen, `19` and `30`]

`19` holds the 2880-byte document HX Edit sent with op 8 for `Revv_Home.hlx`, and `30` the
2762-byte document the pedal returned for that place (op 22). The pedal accepts HX Edit's form and
normalises it when it stores the preset. Compared key by key, HX Edit's form differs in:

- `7.37`: HX Edit's own build (`v3.60-56-g73ae345`), not the firmware's;
- key 5 also holds 111-116 (1.0 each), 380 and 381 (2), 252 and 253 (0), which the pedal drops;
- each snapshot's key 1 has 13 entries instead of 11, and the first element of every slot state
  (`3.N.0`) is nil instead of false;
- the junctions' `before-slot` is 7 and 9 for an unused path B, where the pedal stores 0;
- controller assignments come in another order.

Every other value is the same. `nx_device::import::hlx_document` writes `Revv_Home.hlx` as `19`
byte for byte. How it maps the file [seen, `19`, unless marked]:

- slots: input 0 (`inputA`), path A blocks at `1 + @position`, output 9 (`outputA`), split 10,
  path B blocks at `11 + @position` [inferred], join 19, the rest empty (`{19: 8, 20: nil}`);
- values follow `Helix.sym`'s order up to the last parameter the file holds: a cab's sym lists
  `IrData` after Level, which neither the file nor the document has, and the input stops
  after `decay`. A delay's or reverb's `@trails` comes after them, with key 3 one less than key 2;
- a block is `{24: {23: false, 25: model, 26: -1}, 9: engine, 10: @enabled, 11: values, 12: empty}`;
  a looper uses the pedal's own slot kind 7 form [inferred];
- split and join hold path B's endpoint (`@input`/`@output` of `inputB`/`outputB`, values left out),
  model, `@position + 1`, `@enabled` and values, and `18: false`;
- controller assignments are numbered by block key and then parameter name, alphabetically, and
  sit under their controller (`@controller`) in key 4; key 1 and key 5 both hold the slot. Each
  snapshot's key 2 holds `[@fs_enabled, id, @value]` at position `id`;
- a snapshot's slot states are nil and the block's state in that snapshot; input, output and join
  write false, the split and empty slots true;
- key 5: `@tempo` under 16, the variax values under 45-56 in the order model, tone knob, volume
  knob, lock controls, custom tuning, the six string tunings, mag mode [inferred from defaults
  only], then the constants listed above;
- key 7: the pedal's own service and version (`P33`, `0x03710000`, although the file says 3.70) and
  the file's `build_sha`;
- key 2 is thirteen int16 zeros and thirteen arrays of seven uint8 zeros, the only values written
  in a non-shortest form.

An amp with a cab (`@cab`) and a dual cab are refused: no `.hlx` here holds one, so the file side is
not known.

### Writing a place (librarian op 8) [seen, `19`]

HX Edit's import sends op 255 `{}`, op 8, op 254 `{}` and then reads the list again with op 1. Op 8
is `{107: setlist, 108: place, 109: name + NUL, 123: false, 124: false, 125: 0, 110: document}`, the
document a str as with op 21. nx-edit sends the same; whether the pedal reloads its edit buffer
when the place written is the one it holds is not captured, so nx-edit selects that place again.

A block's engine (slot key 9) is in none of HX Edit's data files. Op 39 (insert) carries it, op 40
(change model) does not, and the pedal's reply to both holds the whole slot with it. `31` changed one
block through every category; the engine follows the model's `category` in HX Edit's `*.models`
[seen for one or two models per category, inferred for the rest]:

| model `category` | blocks | engine |
|---|---|---|
| 3, 4, 6, 7, 8, 11, 14, 17 | distortion, dynamics, filter, pitch/synth, modulation, wah, EQ, volume/pan | 1 |
| 9, 10 | delay, reverb | 8 |
| 1, 13 | amp, preamp | 17; an amp with a cab (`24.23` true) 33 |
| 19 | cab (Mic IR), single and the 2x12 tried | 31 |
| 16 | IR | 19 |
| 15 | looper | 22 |
| 12 | send/return | per model: Send 1 25, Send 2 26 |

In `19`, `@type` 0 (Scream 808, EQ) had engine 1, 1 (amp) 17, 2 (cab) 31 and 7 (delay, reverb) 8, which
fits the table. Not seen: returns and FX loops, the legacy cabs of `cab.models`, gates, stereo
variants (none could be picked on the HX Stomp).

The setters never change what the device wrote: `set_param` keeps a value's type and encoding (an
integer stays an integer of the same width, an f32 stays f32) and rejects a value that type cannot
hold exactly, a boolean for a number or the other way round, with `DocError::Value`. The other
setters check against the document's own counts: the current snapshot must be one of its snapshots,
the cursor one of its slots on path 0 or 1. A snapshot name must be printable ASCII (0x20-0x7e, so no
NUL) of at most 15 bytes [inferred: neither HX Edit's resource files nor the captured documents state
the limit, and the captures only hold names of up to four characters].

Note: capture 03 ends with Drive **0.6** (`SetParam 119: 0.6`, and the document agrees), not about
0.55 as `local/captures/README.md` estimates.

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

### Acknowledgements [seen]

The acknowledgement field is the sender's base plus the payload bytes it has received on that
channel. The base is the last word of the sender's own HELLO: 0x1000 for the host, 0x200 for the
device. Every frame carries the current value, keep-alives included. In `06` the host's first data
frame on each channel carries exactly 0x1000; the device's echo of a 9-byte service open carries
0x209; the host's ACK of a 32-byte reply carries 0x1020. Across the 1600 channel frames in the
captures the host never acknowledges more than it has received; it often lags while frames are in
flight.

The base doubles as the receive window: a side may send payload up to the limit its peer last
acknowledged. That explains the device stopping after 4096 bytes of unacknowledged notifications
[map], and the device's 0x200 means the host may have at most 512 bytes in flight. `19` shows HX
Edit keeping to it [seen]: it sends 512 payload bytes (a 496-byte and a 16-byte frame), waits for
the device's ACK, which raises the limit by exactly 0x200, and only then sends the next 512.
`nx-session` enforces the same window.

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
accepted; a fresh session after a quit is not what the map warns about. The fake device
(`nx_session::fake`) can play the hostile case (`FakeDevice::strict`: a HELLO on an open channel
wedges it), and the session tests run against it.

### Requests and deferred operations

One request in flight per channel; `0x1080` and `0x1001` run independently, as in `06` [seen].
Replies are matched by transaction id. A status 1
reply means accepted: the operation completes with notification 20 carrying the same transaction
id [seen, `02`: status 1 at 16.7391 s, notification 20 at 16.7396 s, next request only after both].
Until it arrives `nx-session` sends no request on any channel [map: racing a running commit wedges
the device; HX Edit waits]. Notification 20 is matched against the one deferred operation in
flight, which also resolves the ambiguity of the two channels' overlapping transaction ids. A
notification 20 that overtakes the status 1 reply (they travel on different channels) is kept with
the request in flight that has its transaction id, and the status 1 reply then completes it at once.

**A timeout leaves the device's state unknown** [rule]. A reply that arrives after its request
was given up on may be status 1, after which the device commits while the host believes nothing
is running; a deferred operation whose notification 20 never came may still be running; a request
whose bytes were only partly sent leaves a half message in the device. Guessing which is the case
is what the spec forbids, so after any reply or deferred timeout the session is *poisoned*: the
timed-out request reports `Timeout`, everything else pending and every later request fails at once
with `Error::Poisoned`, unsent request bytes are dropped, and no request goes out on any channel
again. The session keeps acknowledging and keep-aliving so the device is not left with unread
data. The caller ends the session and connects again. The same holds when the device announces a message larger than
1 MiB (the largest seen, the preset document, is about 3 KB): the stream is out of step, so the
half message is discarded and the session poisoned rather than buffered without bound. A transfer
that does not split into frames is handled up to the break, and then the session is poisoned:
whatever followed is lost (during the handshake this fails the connect; stale transfers from an
earlier client are drained unread before the first HELLO). Opening a
channel (HELLO) starts its message streams afresh.

### Ending a session [seen, partly]

HX Edit's quit in `06` (0-11 s): close `0x1001`, device answers; about 0.5 s later close `0x1002`,
device answers; `0x1080` is left open (the device kept sending keep-alives on it until reads
stopped). The next connect worked, re-opening `0x1080` with a HELLO. `Session::close` does the same
after waiting for a running request or deferred operation (up to its timeout, which poisons the
session) and acknowledging what arrived; if control does not answer its close, events is still
closed. The map reports that closing every channel kept an HX Stomp's next session from opening;
our capture shows HX Edit closing two of three and the device fine afterwards. Dropping a `Session`
runs the same close, bounded by the sum of the reply, deferred and two handshake timeouts; a
session thread still busy after that is left to finish on its own. A handshake that fails closes
whatever control and events channels the device had answered HELLO on, the same way, and is not
retried.

## Open questions

- Whether the device checks the section table of a document written with op 21 or op 8.
- Whether op 21 takes the pedal's own document form, or only HX Edit's; the safe first try is
  writing back the unchanged document the pedal just returned.
- The engine of returns, FX loops, legacy cabs (`cab.models`) and gates; see "A document as HX Edit
  writes it". Op 33 `{102: 5}`, which HX Edit sends after an Amp+Cab or looper model change, and
  notifications 31 `{98, 70, 79}` and 34 that come with it (`31`).
- Op 109 in a backup (three calls per id for 372 ids, keys 64, 105, 106), the `116` word of
  notification 5, list keys 123-125, and globals 96-99.
- Why op 255 took a second before the export in `18` and two milliseconds everywhere else.
- Preset document: top-level keys 2 and 3, most of 5, snapshot key 1, snapshot-list keys 7 and 13,
  block keys 12, 24.23, 24.26, junction key 18, and the first element of slot states and controller
  values. Captures that change footswitches, MIDI, Command Center, tempo and snapshot colour would
  pin these down.
- Whether `before-slot` and path positions hold for presets with other split positions, and what
  the 10-word section table looks like on this device.
- Cab engine class 31 versus PROTOCOL.md's 15: a different cab type, or firmware-specific.
- Op 30 carries the parameter index under `28` (settled by `26`), while a controller assignment in
  the document keeps it under `6.29` with `6.28: 0`. Whether `6.28` is the assignment's counterpart
  of op 30's `29` (and `6.29` then just the index) is not known.
- Byte 0 of the channel frame body has always been 0: the high byte of a 16-bit sequence that never
  got past 0xff, or something else? (Sequences wrap at 0xff, so probably not a counter.)
- The middle word of HELLO (`00010021` host, `00010001` device): version and capabilities?
- Why HX Edit closes `0x1001` and `0x1002` at quit but not `0x1080`.
- What the device does if the host closes `0x1080`, or if the device closes a channel first.
- Whether non-deferred requests on other channels may run while a deferred operation is pending
  (HX Edit's captures never try; `nx-session` holds them).
- Status values other than 0 and 1.
- Ops 76, 83 and the remaining fields of op 23.
- Footswitches stopped switching presets after the second live knob session (2026-11-13,
  `live-06-knobs`), until the pedal was power-cycled. nx-edit had sent only op 78 and op 30
  (Drive, cab Distance), all accepted, and closed control and events as HX Edit does. Seen in the
  same captures, not understood: notification 22 `{118: 19, 119: true}` (global setting 19 became
  true; in the first session six seconds before nx-edit wrote anything) and notification 23
  `{23: 0}`, six times while the pedal was being handled. Capture 05 shows 23 after every snapshot
  switch, so the pedal was most likely in snapshot footswitch mode (FS1+FS2 cycles the modes), in
  which footswitches choose snapshots, not presets; the power cycle restored the default mode.
  Session 2 (`24`) confirms that FS1+FS2 changes the footswitch mode, global 21, and that a single
  footswitch then picks snapshots; global 19 has not been seen again.
