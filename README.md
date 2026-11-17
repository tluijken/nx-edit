# nx-edit

A native Linux editor for the Line 6 HX Stomp, built as a replacement for HX Edit on NixOS. It
speaks the pedal's USB protocol directly. Line 6 never published that protocol; it was recovered
from USB captures of HX Edit and static analysis of its Windows binary, with tonepush's protocol
notes as a starting map (see [docs/protocol.md](docs/protocol.md)).

nx-edit exists because Line 6 offers no HX Edit for Linux. The protocol was studied only as far as
an independently written editor needs to work with the pedal, mostly by observing HX Edit's USB
traffic, as the EU Software Directive (2009/24/EC, articles 5(3) and 6) allows for interoperability.
nx-edit contains no code or data of Line 6; the model data it shows is imported from the user's own
HX Edit installation.

![nx-edit](docs/nx-edit-design.png)

Tested against an HX Stomp on firmware 3.71, with HX Edit 3.82 as the reference, on NixOS and in a
Windows 11 VM.

## What it does

- Reads the current preset from the pedal and draws its signal chain, including path B splits and
  joins.
- Turns knobs, picks list values such as a cab's mic by name, flips switches and a delay's or
  reverb's trails, switches blocks on and off and changes snapshots on the pedal. It follows edits
  made on the pedal itself, and undo and redo work for knob turns. A snapshot's footswitch LED colour
  is picked from the dot beside it. Save (Ctrl+S) stores the edits in
  the preset's own place on the pedal, and nx-edit asks before a preset switch would drop them.
- Edits the pedal's chain on path A: click an empty slot and pick a model to add a block, pick a
  model with a block selected to change it, remove a block from its context menu, or drag it to
  another position.
- Lists the pedal's presets and follows preset switches.
- Keeps a library of `.hlx` files beside the pedal's list. You can drop files in, rename them and
  move them to the trash. Library presets can be edited offline and saved (Ctrl+S); nx-edit asks
  before it drops unsaved edits.
- Browses CustomTone, signs in to line6.com and downloads tones into the library.
- Puts a library or CustomTone preset on a place of the pedal ("Assign to a place on the pedal…"
  in the library's context menu), written as HX Edit writes an import. Amps with a cab, dual cabs,
  returns, FX loops and other blocks whose engine number has not been captured are refused by name.

## Not yet

- Exporting the pedal's preset as an `.hlx` that HX Edit accepts: the conversion writes 60 of the
  132 values HX Edit's export holds ([docs/protocol.md](docs/protocol.md), "Compared with HX Edit's
  export").
- Firmware updates; those stay with HX Edit.

## Download

Builds for Linux (x86_64), Windows (x86_64) and macOS (Apple Silicon and Intel) are attached to
each [release](https://github.com/tluijken/nx-edit/releases). They are not signed:

- Windows SmartScreen and macOS Gatekeeper warn on first start. On macOS, clear the quarantine flag
  with `xattr -d com.apple.quarantine nx-edit`.
- On Linux, nx-edit loads the X11 or Wayland libraries and OpenGL of the desktop it runs on, and
  needs access to the pedal's USB device (see below).
- On Windows, it uses the USB driver HX Edit installed (see "Windows").

The macOS builds are compiled but have not been tried against a pedal yet.

## Installing on Linux

nx-edit needs access to the pedal's USB device. The distribution packages install a udev rule,
`/usr/lib/udev/rules.d/70-nx-edit.rules`, that gives the logged-in user access to Line 6 devices
(vendor `0e41`, HX Stomp `0e41:4246`); reconnect the pedal after installing so it applies. For the
tarball and the Flatpak, add the rule yourself:

```sh
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="0e41", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/70-nx-edit.rules
sudo udevadm control --reload-rules
```

and reconnect the pedal. `--import-hxedit` needs 7-Zip (`7z`) to unpack the HX Edit installer; the
packages recommend it rather than require it, as an installed HX Edit folder needs no 7-Zip.

Releases after v0.1.0 carry a `.deb` and an `.rpm` for x86_64 beside the tarball. On Debian, Ubuntu
and their derivatives:

```sh
sudo apt install ./nx-edit_0.1.1-1_amd64.deb
```

On Fedora, openSUSE and other RPM distributions:

```sh
sudo dnf install ./nx-edit-0.1.1-1.x86_64.rpm
```

The AUR, COPR and Flathub packages below are prepared in [packaging/](packaging) but not published
yet.

On Arch Linux, from the AUR, with `makepkg` or an AUR helper:

```sh
yay -S nx-edit-bin
```

On Fedora, from COPR, built from source:

```sh
sudo dnf copr enable tluijken/nx-edit
sudo dnf install nx-edit
```

As a Flatpak, from Flathub:

```sh
flatpak install flathub io.github.tluijken.NxEdit
flatpak run io.github.tluijken.NxEdit --import-hxedit ~/Downloads/"HX Edit v3.82 Installer.exe"
```

The Flatpak includes 7-Zip. Its sandbox reads the installer only from the Downloads folder, and
keeps the library in `~/.var/app/io.github.tluijken.NxEdit/data/nx-edit`.

With the tarball, put `nx-edit` anywhere on your `PATH`; tarballs after v0.1.0 also hold the udev
rule, the desktop entry and the icon. On Nix, see "Running"; on NixOS, add the rule with
`services.udev.extraRules`.

## Running

```sh
nix run ~/projects/prive/nx-edit
```

for Nix. To install it, add the flake as an input and use `packages.x86_64-linux.default`; the
package includes a desktop entry and an icon.

Block and parameter names come from HX Edit's own model data, which nx-edit does not ship. Import
it once, either from the official Windows installer, which nx-edit unpacks with 7-Zip without
running it, or from an installed HX Edit, by passing its folder (the one that holds `res`) or the
`res` folder itself:

```sh
nx-edit --import-hxedit "HX Edit v3.82 Installer.exe"
nx-edit --import-hxedit "<HX Edit folder>"
```

The data lands in `$XDG_DATA_HOME/nx-edit/hxedit-res` and the preset library in
`$XDG_DATA_HOME/nx-edit/library` (`%APPDATA%\nx-edit\hxedit-res` and `%APPDATA%\nx-edit\library` on
Windows).

```
nx-edit [preset.hlx]                      open the editor, optionally with a library preset
nx-edit --from-device                     connect to the pedal and open its current preset
nx-edit --from-capture <capture.pcapng>   show the preset found in a USB capture
nx-edit --import-hxedit <installer.exe | HX Edit folder>   import HX Edit's model data
```

Your user needs access to the pedal's USB device (vendor `0e41`, HX Stomp `0e41:4246`), for example
through a udev rule (see "Installing on Linux"). While the pedal is passed through to a VM, nx-edit
cannot reach it.

### Windows

```sh
nix build .#windows
```

cross-compiles `nx-edit.exe` with MinGW and leaves `result/nx-edit-windows-x86_64.zip`. Unzip it
anywhere; there is no installer. The Line 6 password is kept in the Windows Credential Manager.
nx-edit uses the USB driver HX Edit installed and does not install one itself; HX Edit must be
closed while nx-edit talks to the pedal.

## Development

```sh
nix develop -c cargo test --workspace
nix develop -c cargo run -p nx-app
```

The dev shell sets `LD_LIBRARY_PATH` for the libraries eframe loads at runtime. UI tests run
headless with `egui_kittest`.

The `windows` dev shell builds for `x86_64-pc-windows-gnu` and runs the tests under Wine:

```sh
WINEPREFIX=$PWD/target/wine WINEDLLOVERRIDES="mscoree=;mshtml=" nix develop .#windows -c cargo test --workspace
```

The overrides skip Wine's Mono and Gecko prompts, which otherwise stall the first run of a new
prefix.

| crate | role |
|---|---|
| `nx-app` | the egui application (`nx-edit` binary) |
| `nx-usb` | USB transport to the pedal |
| `nx-proto` | frames, messages and the preset document |
| `nx-session` | session handling: sequence numbers, acknowledgements, keep-alives |
| `nx-device` | reading the pedal's presets and editing them live, and converting them to `.hlx` |
| `nx-hlx` | reading and editing `.hlx` preset files |
| `nx-catalog` | model and parameter catalog from HX Edit's data |
| `nx-customtone` | line6.com sign-in and the CustomTone listing |
| `nx-capture` | decoding `usbmon` captures of HX Edit |

## Documentation

- [docs/protocol.md](docs/protocol.md): the USB protocol, marked as seen or inferred.
- [docs/capture.md](docs/capture.md): how captures are made, and the next session.
- [docs/reverse-engineering.md](docs/reverse-engineering.md): static analysis of HX Edit.
- [docs/customtone.md](docs/customtone.md): how CustomTone's pages and downloads work.

## License

MIT, see [LICENSE](LICENSE). This project is not affiliated with or endorsed by Line 6 or Yamaha. HX Stomp, HX Edit and
CustomTone are their trademarks.
