# Capturing HX Edit's USB traffic

HX Edit runs in the Windows VM (`~/vms`, started with `~/vms/start.sh`); the HX Stomp is passed
through on its own `qemu-xhci` controller. Every transfer QEMU makes on the pedal's behalf goes
through the host's USB stack, so `usbmon` on the host sees all of it.

## One-time host setup

`usbmon` is not loaded by default. In the NixOS config (`~/.dotfiles/nixos/virtualization.nix`):

```nix
boot.kernelModules = [ "usbmon" ];
```

Capturing needs root (`/dev/usbmonN` is root-only), so captures run with `sudo dumpcap`.

## Making a capture

1. Find the pedal's bus: `lsusb -d 0e41:` (`Bus 003 Device 042` means bus 3, device 42; the device
   number changes on every re-enumeration).
2. Start the capture before the action:
   `sudo nix shell nixpkgs#wireshark-cli -c dumpcap -i usbmon3 -w local/captures/<label>.pcapng`
3. Do exactly one action in HX Edit (select a preset, turn one knob, save, ...), wait a second,
   stop the capture with Ctrl-C.
4. Write `local/captures/<label>.md`: what was clicked, from which state, what the pedal showed.
5. Inspect: `cargo run -p nx-capture -- local/captures/<label>.pcapng --device 42 --ep 0x01,0x81`.

The editor channel is interface `MI_00`: bulk OUT `0x01`, bulk IN `0x81`. Audio (`0x03`, `0x83`),
MIDI (`0x02`, `0x82`) and HID traffic can be filtered out with `--ep`.

## Rules

- One action per capture, labelled. Mixed captures cost more time than they save.
- Never capture while signing in to a Line 6 account in the same file as device traffic; account
  traffic is captured separately (HTTPS proxy in the VM) and tokens are scrubbed before committing.
- Captures contain Line 6 data and stay in `local/captures/`, which git ignores. Tests that read
  them are skipped where that folder is absent; the preset documents the tests need are cut from
  them into `crates/nx-proto/tests/fixtures/`.

## Session 2 (planned)

`scripts/capture-session.sh` leads through the next session: preset editing, chain editing,
switch and list parameters, renaming, saving, export, import to a place (the CustomTone "assign"
flow), copy and clear, a global setting, backup, and a footswitch mode check. Each step prints
the action, captures it, keeps the pedal's traffic in `local/captures/<label>.pcapng` and logs what was
seen in `local/capture-session-2.log`. All edits happen on 03A–03C, never on the presets in use.
Before step 19, copy `Revv_Home.hlx` into the VM.

What nx-edit waits on from this session:

- **Step 17, save**: how HX Edit writes the edited preset back; nx-edit's live edits are not saved
  on the pedal until this is known.
- **Step 18, export**: copy the exported `.hlx` out of the VM next to the capture. nx-edit's
  conversion of the pedal's preset already has the chain, parameters, block states, snapshots
  (names, block states, controlled values, LED colour, tempo) and controller assignments. Still
  missing, to compare against the export: per block `@type`, `@stereo`, `@trails`,
  `@bypassvolume` and `@no_snapshot_bypass`; `inputA/B` and `outputA/B`; the rest of
  `tone.global`; `variax`, `footswitch` and `dsp1`; `meta` beyond the name; and the snapshots'
  `@pedalstate`.
- **Step 19, import to 03B**: what HX Edit sends for a preset file. The "Assign to a place"
  dialog (branch `feat/assign`) has its Replace button disabled until this is in.
