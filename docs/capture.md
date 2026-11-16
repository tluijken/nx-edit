# Capturing HX Edit's USB traffic

HX Edit runs in the Windows VM (`~/vms`, started with `~/vms/start.sh`); the HX Stomp is passed
through on its own `qemu-xhci` controller. Every transfer QEMU makes on the pedal's behalf goes
through the host's USB stack, so `usbmon` on the host sees all of it.

## One-time host setup

`usbmon` is not loaded by default. In the NixOS config (`~/.dotfiles/nixos/virtualization.nix`):

```nix
boot.kernelModules = [ "usbmon" ];
```

`/dev/usbmonN` belongs to the `wireshark` group here, so a member of that group captures with
`sg wireshark -c "dumpcap ..."`, which is what `scripts/capture-session.sh` does; without the group
it needs `sudo dumpcap`.

## Making a capture

1. Find the pedal's bus: `lsusb -d 0e41:` (`Bus 003 Device 042` means bus 3, device 42; the device
   number changes on every re-enumeration).
2. Start the capture before the action:
   `sg wireshark -c "dumpcap -i usbmon3 -w local/captures/<label>.pcapng"`
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

## HX Edit drops the connection by itself

In the VM, HX Edit regularly shows "Connection interrupted" and offers Reconnect, after anything
from a few seconds to a minute; users report the same on real Windows machines. It is HX Edit's
own doing [seen, 2026-11-16]: it sends close (kind 0x02) on its channels with nothing unusual
before it, the device answers, and HX Edit's acknowledgements match every byte the device sent.
nx-edit's own session on the same VM stayed up for two minutes with all 396 device transfers
delivered, and neither a VM reboot, a pedal power cycle, restarting HX Edit, stopping other
programs nor signing out of the Line 6 account stopped it.

For captures this means: click Reconnect, then act, and check the timeline afterwards.
`nx-capture --messages` shows the action's request and reply; a close or HELLO between them (the
kind byte is `02`) means the step has to be done again. In session 2 every interruption fell
outside the action.

## Session 2 (2026-11-16)

`scripts/capture-session.sh` lists the steps; the session was run by hand with the same labels.
Device 46 for `07`, 48 for the rest (the pedal re-enumerated after a power cycle and a VM reboot).
All edits were on 03A-03C. `local/capture-session-2.log` has a line per step with what was seen.
What each capture settles is in [protocol.md](protocol.md).

| capture | action | shows |
|---|---|---|
| `07-select-03A` | select 03A | op 20, the notifications of a preset load |
| `08-add-amp` | Revv Gen Red into an empty slot | op 39 insert block |
| `09-change-model` | amp to Cali Rectifire | op 40 change model |
| `10-add-delay` | Simple Delay after the amp | op 78, op 39 |
| `11-move-block` | delay in front of the amp | op 21: the whole document written back |
| `12-toggle-switch` | Trails on | op 30 with `29: false` |
| `13-add-cab-mic` | 4x12 Cali V30, Mic changed twice | op 39, op 30 with an integer |
| `14-remove-block` | remove the delay | op 28 |
| `15-rename-snapshot` | meant as snapshot 1 to "Test", renamed the **preset** | librarian op 6 |
| `16-rename-preset` | 03A to "NX Test" | librarian op 6 |
| `17-save-preset` | save 03A | op 71 |
| `18-export-preset` | export 03A; `18-export-preset.hlx` is the file | librarian ops 255, 4, 254 |
| `19-import-to-03B` | Revv_Home.hlx to 03B with Import | librarian op 8, the 512-byte window |
| `20-copy-paste-03C` | 01A to 03C with Copy and Paste | librarian op 4 then op 8 |
| `22-global-exp-fs-tip` | EXP/FS Tip to FS4 and back | ops 24, 25, global 95 |
| `23-backup` | backup; `23-backup.hxb` is the file | op 85, librarian ops 109 and 4 |
| `24-footswitch-mode` | FS1+FS2 twice, then a single footswitch | global 21 |
| `24-footswitch-mode-long-press` | FS1+FS2 held too long | global 21, a preset switch to 30B |
| `25-rename-snapshot` | snapshot 1 to "TestShot", via right-click on the snapshot at the top | op 89 |
| `26-amp-second-knob` | Cali Rectifire Bass 2.6 to 2.8 | op 30 with `28: 1` |
| `27-snapshot-led-colour` | snapshot colour Red, then Violet | op 98, colours 2 and 9 |

Renaming in HX Edit: a right-click on a preset in the list (or Ctrl+R) renames the preset, also
when a snapshot was meant (`15`, and a first try at `25` kept as
`local/raw/25-rename-preset-again.pcapng`, which renamed 03A to "NX Test 2"). A snapshot is
renamed, recoloured, copied and pasted from a right-click on its name next to the preset name.

Not captured:

- **21, clear a preset**: HX Edit has no such command (Edit > Clear is for blocks and was disabled,
  Del does nothing in the list). `local/raw/21-clear-03C-not-available.pcapng` has the attempt.
- **Tuner Reference** (planned for 22) is not in HX Edit; EXP/FS Tip took its place.

Left for a next session: comparing nx-edit's conversion of `18`'s document with
`18-export-preset.hlx`, field by field (no pedal needed); snapshot copy and paste.
