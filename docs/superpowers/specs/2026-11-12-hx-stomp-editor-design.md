# nx-edit: a native NixOS editor for the Line 6 HX Stomp

Date: 2026-11-12. Owner: Thomas. Status: design, awaiting approval.

## Intent

Thomas owns an HX Stomp and runs NixOS. Line 6's HX Edit exists only for Windows and macOS, so today
every edit goes through a Windows VM (`~/vms`, quickemu) with USB passthrough. That works but is
fragile. He wants his own native editor that does everything HX Edit does, looks like his DAW Scythe,
and contains no protocol code he did not write himself.

He plays technical death metal in Bb. Day to day he uses the editor at home to fine-tune presets (EQ,
gain, delay and reverb); on stage he only plays the pedal. The editor is for his own use.

Later the editor should be usable from inside Scythe (`~/projects/prive/scythe`) as a plugin or
panel. That is not part of this project, but the architecture must not block it.

## Goals

In order of priority; the first item is what makes the editor worth using.

1. **Live preset editing.** Connect, list presets, select one, show its signal chain, edit block
   parameters with immediate audible feedback on the pedal, swap a block's model from a model browser,
   bypass blocks, save. The editor follows changes made on the pedal itself.
2. **Preset management.** Rename, copy, paste, reorder, clear; import and export `.hlx`; full backup and
   restore of the pedal.
3. **Snapshots, footswitches, controllers.** Switch, rename and edit snapshots; footswitch and expression
   assignments as HX Edit offers them on an HX Stomp.
4. **Signal routing.** Splits, merges, paths and output selection (Main L/R, Send), moving blocks.
5. **Impulse responses.** Upload, list, clear.
6. **Global settings** that HX Edit exposes.
7. **Line 6 account and firmware, like HX Edit.** Sign in to the Line 6 account, check for and download
   firmware. Credentials stored in the desktop keyring (Secret Service), never in plain files.
   **Flashing is deferred** (decided 2026-11-12): Thomas does not want to repeat the 3.71 → 3.82 update
   now, so there is no update to capture. Flashing is built only when Line 6 releases a new firmware
   that can be captured end to end (see Firmware safety). Until then the pedal stays on 3.71.
8. **The rest of "everything HX Edit does"** after the above: favorites, user default block settings,
   block presets, Command Center as far as it applies to an HX Stomp, Global EQ, and anything else
   HX Edit 3.82 offers for this device. Each is its own later milestone. (Builder's call on order.)

Success: Thomas edits his presets at home with nx-edit instead of HX Edit, and the Windows VM is only
kept as a fallback.

## Non-goals

- Other Line 6 devices (Helix, HX Effects, HX Stomp XL, POD Go). The code may be structured so they
  could be added, but they are not tested or supported.
- Stage use. The editor does not need to be robust for live performance; it is not connected on stage.
- Windows or macOS builds.
- Scythe integration (later project; see Architecture for what keeps it possible).
- Redistributing Line 6 data. Model names, parameter ranges and artwork come from Thomas's own HX Edit
  installation at runtime and are never committed.

## Anti-goals (a failure even if it technically works)

- **Bricking the pedal or leaving it unrecoverable.** Any flash operation must have a tested recovery
  path before it is enabled (see Firmware safety).
- **Locking the pedal up** so it needs its power pulled. Requests must follow the sequencing, timeouts
  and acknowledgements HX Edit uses; when unsure, stop and reconnect instead of guessing.
- **Silent data loss.** A preset or setlist must never be overwritten without an existing backup of what
  it replaces.
- **Third-party protocol code.** No code from tonepush, fretwire, openhx or similar projects. Thomas does
  not trust those libraries.
- **Leaking credentials.** No Line 6 password in files, logs, captures committed to git, or crash output.

## Constraints

- **Platform:** NixOS (nixpkgs 26.05), x86-64, Hyprland/Wayland. Rust from nixpkgs (1.95 at time of
  writing). The dev shell must not inherit the global `LD_LIBRARY_PATH` from `/etc/set-environment`
  (it points at pipewire-jack and breaks binaries from a newer nixpkgs); use `env -u LD_LIBRARY_PATH`.
- **UI:** egui/eframe 0.37 and edition 2024, matching Scythe.
- **Dependencies:** widely used general crates are fine (USB transport such as `nusb`, egui, serde, a
  MessagePack codec, a keyring crate, an HTTP client). Anything that speaks the HX protocol is written in
  this repo.
- **Reference material:** tonepush's `PROTOCOL.md` (MIT, prose only) may be read as a map. Every claim
  taken from it is verified against our own captures or the decompiled HX Edit before code depends on it.
  No code is copied from it.
- **Device access:** udev rule in `~/.dotfiles/nixos/virtualization.nix` (`70-line6.rules`) already
  grants the logged-in user access to `0e41:*` and the bootloader `1fc9:000c`.
- **Exclusive interface:** HX Edit and nx-edit both claim the vendor interface; only one may run at a
  time. When the pedal is passed to the VM, nx-edit cannot see it, and vice versa.
- **Firmware target:** the pedal runs 3.71.0 and stays there for this project; the protocol targets
  3.71 as driven by HX Edit 3.82. HX Edit 3.82 does work with it: on 2026-11-12 it connected, listed the
  presets and imported five `.hlx` presets, while offering the update.
- **Firmware history:** the pedal runs 3.71.0. An update to 3.82 through HX Edit was aborted in DFU mode
  on 2026-11-12; the pedal recovered by itself. HX Edit 3.82 is installed in the VM.

## What is known about the device

Observed on 2026-11-12 (host `lsusb -v`, VM PnP, captured `.hlx` files):

- USB `0e41:4246`, serial `4294967295`, high speed. Interfaces:
  - `MI_00` vendor-specific, bulk EP `0x01` OUT / `0x81` IN, 512 bytes. Windows binds WinUSB. This is
    the editor channel.
  - `MI_01..03` USB Audio (iso EP `0x03` OUT / `0x83` IN), `MI_04` USB MIDI (bulk EP `0x02`/`0x82`),
    `MI_05` HID keyboard.
- During a firmware update the pedal re-enumerates as an NXP bootloader `1fc9:000c` ("LPC"); Windows
  names it "DFU" and binds WinUSB. In normal mode it returns as `0e41:4246`.
- `.hlx` presets are JSON (`schema: L6Preset`, `version: 6`), HX Stomp device id `2162694`, firmware
  encoded as `0xMMmm0000` in `device_version` (3.70 = `0x03700000`). Model ids look like
  `HD2_AmpRevvGenRed`; snapshots store per-block bypass and controller values; controller id 9 is the
  snapshot controller. A generator that writes valid Stomp presets exists in `~/vms/presets/make_presets.py`.
- Per tonepush's write-up (unverified): the editor protocol uses channels, MessagePack RPC and a preset
  document with an internal offset table; HX Edit's model catalog ships as JSON in its resources.

## Approaches considered

**A. Own Rust workspace; protocol from USB captures (chosen).** HX Edit runs in the VM; the host records
the passed-through traffic with `usbmon`. Message formats are derived from captures, using `PROTOCOL.md`
as a map. Fast to iterate because every claim can be checked against the wire.

**B. Decompile HX Edit (used where needed).** HX Edit is a large native C++ app (likely JUCE). Deriving
message formats from decompiled code is slower than reading them off the wire, but the code shows what
captures cannot: checksums, validation, error handling, timeouts and ordering. Thomas wants B wherever
captures leave a question open, and it is required for firmware flashing and the Line 6 login/download
flow, which are undocumented and (for the network part) TLS-encrypted.

**C. Fork tonepush and audit it.** Fastest, rejected: Thomas does not trust that code.

Decision: **A, with B wherever captures leave a gap, and B always for firmware and account flows.**

## Architecture

Decided:

- New repo `~/projects/prive/nx-edit`, Cargo workspace.
- The protocol side is a library with no UI dependency, so Scythe can later use it directly. The app is
  a thin egui shell over it.
- Scythe's `crates/scythe-app/src/theme.rs` is copied into nx-edit for now; when Scythe integration
  starts it becomes a shared crate.

Proposed crate split (builder may adjust):

| crate | role |
|---|---|
| `nx-proto` | pure codec: framing, channels, messages, preset document. No I/O. |
| `nx-usb` | transport over the `MI_00` bulk endpoints. |
| `nx-session` | connection lifecycle, request/response sequencing, event stream from the pedal. |
| `nx-catalog` | reads model and parameter definitions from a local HX Edit installation or installer. |
| `nx-hlx` | `.hlx` import/export. |
| `nx-account` | Line 6 sign-in, firmware lookup and download; credentials in the keyring. |
| `nx-firmware` | firmware file handling and the flash sequence, including the bootloader. |
| `nx-app` | eframe application. |
| `nx-capture` (tool) | turns usbmon captures into readable message logs; used for reverse engineering and test fixtures. |

## UI

Decided (mockup B on 2026-11-12): HX Edit's layout in Scythe's colours.

- Top bar: preset name, active snapshot, undo, save.
- Left: model browser by category; clicking a model replaces the selected block and is heard on the
  pedal immediately.
- Centre: the signal path as HX Edit draws it, paths stacked, splits and merges visible, output
  endpoints (Main L/R, Send) at the end.
- Bottom: knobs of the selected block, sent live while turning, values in HX Edit's units
  (0–10, dB, Hz, ms).
- The editor mirrors changes made on the pedal (knobs, snapshot, preset).

Proposed: the visual vocabulary (block tiles, toggles, knobs) follows Scythe's guitar rig window
(`docs/screenshots/guitar-rig.png` in the Scythe repo).

## Reverse-engineering workflow

1. **Capture setup on the host.** Load `usbmon` (NixOS `boot.kernelModules`) and give the user read
   access to the capture device, or capture with `sudo tshark`. Keep the pedal passed to the VM on its
   own `qemu-xhci` controller (see `~/vms/windows-11.conf` and `~/vms/watch-line6.sh`).
2. **One action per capture.** Connect, select preset, turn one knob, swap a model, save, rename, IR
   upload, and so on, each labelled. Captures and notes live in the repo; captures must not contain
   credentials.
3. **Decoder first.** `nx-capture` decodes captures into messages; decoded captures become test fixtures
   for `nx-proto` (encode/decode round trips byte for byte).
4. **Decompile where needed.** HX Edit and its bundled libraries are analysed with Ghidra (available in
   Scythe's devbox; the REA MCP server needs repairing first). Focus: checksums and validation, error
   paths, the firmware update sequence, the account/download flow.
5. **Network flows.** The Line 6 login and firmware download are observed from inside the VM through an
   intercepting proxy, with Thomas's own account. Tokens and passwords are scrubbed from anything stored.

## Firmware safety

Flashing is out of scope until a new Line 6 firmware exists to capture (see Goals). Checking for updates
and downloading firmware, which happen before any flashing, are in scope and must never trigger an
update on the pedal. When flashing is taken up later, it stays disabled until all of these hold:

- The full HX Edit update sequence (normal mode → bootloader `1fc9:000c` → flash → reboot → rebuild) is
  captured end to end at least once and the code reproduces it exactly.
- Only unmodified firmware files obtained from Line 6 are flashed; their integrity is checked the same
  way HX Edit checks it (to be found in the decompiled code).
- Recovery is documented and tried: hold the right page button while powering on to enter update mode,
  then flash with Line 6 Central or HX Edit in the VM. The VM stays available as the fallback.
- The editor refuses to flash on battery power, warns the user not to disconnect, and keeps the pedal
  claimed through the bootloader re-enumeration.
- A dry run mode shows every step without writing.

## Testing

- `nx-proto`: byte-exact round trips on captured fixtures.
- `nx-session`: replay of captured sessions against a fake transport, including timeouts and the pedal's
  own events.
- `nx-hlx`: the presets in `~/vms/presets` and HX Edit exports import and export losslessly.
- Hardware tests only with the pedal attached and an explicit flag; read-only first.
- Write tests use only the last bank of the pedal as scratch space (decided by Thomas, 2026-11-12). His
  own presets live in 01A–02B and must never be written by tests.
- Firmware: dry run on every build; real flashing only by hand, with Thomas present.

## Calls made while writing this up (Thomas may override)

- **Captures are made in short scripted sessions:** Thomas clicks through HX Edit in the VM following a
  written script; the builder records and labels. The builder can also drive the VM's keyboard and take
  screenshots, but not reliably click.
- **The repo stays local and private;** captures and fixtures are committed. They contain Line 6 data
  (model ids, preset contents), so the repo must not be published as is.
- **`nx-session` is runtime-agnostic:** it owns a thread and talks over channels, so it works inside
  Scythe later (Scythe uses tokio) without depending on it.

## Left to the builder

- Exact crate boundaries, error types, async or threads.
- Which general-purpose crates to use (USB, MessagePack, HTTP, keyring).
- The order of capture sessions within each phase.
- How the editor resolves the HX Edit data location (installed in the VM, or extracted from the
  installer in `~/Downloads`).
- Whether 3.71 and newer firmware need separate code paths; Thomas may update the pedal to 3.82 once
  the editor's read path works.
