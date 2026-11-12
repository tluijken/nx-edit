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
   `sudo nix shell nixpkgs#wireshark-cli -c dumpcap -i usbmon3 -w captures/<label>.pcapng`
3. Do exactly one action in HX Edit (select a preset, turn one knob, save, ...), wait a second,
   stop the capture with Ctrl-C.
4. Write `captures/<label>.md`: what was clicked, from which state, what the pedal showed.
5. Inspect: `cargo run -p nx-capture -- captures/<label>.pcapng --device 42 --ep 0x01,0x81`.

The editor channel is interface `MI_00`: bulk OUT `0x01`, bulk IN `0x81`. Audio (`0x03`, `0x83`),
MIDI (`0x02`, `0x82`) and HID traffic can be filtered out with `--ep`.

## Rules

- One action per capture, labelled. Mixed captures cost more time than they save.
- Never capture while signing in to a Line 6 account in the same file as device traffic; account
  traffic is captured separately (HTTPS proxy in the VM) and tokens are scrubbed before committing.
- The repo is private; captures contain Line 6 data and stay out of public repos.
