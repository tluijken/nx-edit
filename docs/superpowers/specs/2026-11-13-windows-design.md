# nx-edit on Windows

Date: 2026-11-13. Owner: Thomas. Status: design, awaiting approval.

## Intent

nx-edit runs on NixOS only. Thomas wants the same editor on Windows, for two reasons: the private
nx-edit plugin for Reaper and Scythe (separate design) has to run there too, and Scythe itself is
getting a Windows port. He has no Windows machine; the Windows 11 VM in `~/vms` (quickemu, HX Stomp
passed through on its own `qemu-xhci` controller, HX Edit 3.82 installed) is where it is tested.

Success: in the VM, the Windows build of nx-edit connects to the HX Stomp, shows and edits its
current preset live, browses the library and CustomTone, and keeps the Line 6 password in the
Windows credential store, all as it does on NixOS.

## Goals

1. A Windows x86_64 build of the `nx-edit` binary, built from NixOS (no toolchain in the VM needed
   to build; the VM only runs it).
2. Every feature that works on Linux works on Windows: live editing over USB, the library, offline
   editing and saving, CustomTone browsing, sign-in and download.
3. Model data comes from the HX Edit installation on the same machine when there is one, so a
   Windows user does not need the installer and 7-Zip.
4. A way to get a build into the VM and run the tests there that need Windows (USB, credential
   store, paths), repeatable by Claude without Thomas clicking.

## Non-goals

- An installer or code signing. A zip with the `.exe` is enough for now (Scythe's installer is a
  separate project and nx-edit can reuse it later).
- macOS.
- Running nx-edit and HX Edit against the pedal at the same time. Only one program can hold the
  editor interface; that is the same on Linux.
- Windows on ARM.

## Anti-goals

- Anything that breaks or slows the Linux build, or makes the Nix package depend on a Windows
  toolchain.
- Installing or replacing USB drivers on Thomas's behalf. nx-edit uses the driver HX Edit already
  installed; if it is missing, nx-edit says so.
- Touching the pedal from the VM without Thomas present (same rule as on Linux).

## What is platform-specific today

| Area | Linux today | Windows |
|---|---|---|
| USB (`nx-usb`, `nusb` 0.2) | usbfs; claims interface `MI_00` | `nusb` supports Windows through WinUSB. HX Edit's installer put a WinUSB driver on the pedal (`~/vms/PLAN.md`); whether it is bound to `MI_00` itself is to be checked in Device Manager |
| Data folders (`nx_catalog::default_res_dir`, library) | `$XDG_DATA_HOME/nx-edit/...` | `%APPDATA%\nx-edit\...` (`hxedit-res`, `library`, `line6-user`) |
| Password (`account.rs`, `keyring` 4 with the Secret Service store) | GNOME keyring | Windows Credential Manager (`keyring`'s Windows store) |
| Model data import (`--import-hxedit`, runs `7z`) | unpacks the installer | read `res` straight from the installed HX Edit when present; the installer import stays as a fallback and needs `7z` on `PATH` |
| GUI (eframe 0.37) | Wayland/X11 | Win32 + wgpu or glow; works upstream |
| Packaging | Nix flake, desktop entry | zip with `nx-edit.exe` and the fonts already embedded |

## Approaches

1. **Cross-compile from NixOS with MinGW (`x86_64-pc-windows-gnu`)**, as a second output of the
   flake. Reproducible, needs nothing in the VM, and the VM only runs the result. Risk: crates that
   need MSVC-only pieces; none of nx-edit's dependencies is known to.
2. **Cross-compile for MSVC with `cargo-xwin`.** Closer to how Windows software is usually built,
   but downloads the Microsoft SDK at build time, which is not reproducible in Nix and has its own
   licence terms.
3. **Build inside the VM** with rustup and the MSVC build tools. Simplest toolchain-wise, but the
   VM becomes a build machine to maintain, and builds are slow there.

**Recommended: 1.** It matches how nx-edit is already built (Nix), keeps the VM disposable, and
the dependency set is plain Rust plus `nusb`, `eframe`, `keyring` and `reqwest`-like networking,
all of which build for the GNU target. If a dependency turns out to need MSVC, fall back to 3 for
that one build, not 2.

## The how, as far as decided

- Platform differences live behind small functions, not `cfg` scattered through the UI:
  `nx_catalog::default_res_dir()` and the library root get a Windows branch; `account.rs` picks
  the keyring store per target; the importer gains "use the installed HX Edit" with the
  installer as fallback.
- Where HX Edit keeps its `res` folder on Windows is read from the installation itself (to be
  confirmed in the VM; do not hard-code a guess).
- Tests that are platform-independent keep running on Linux. Tests that need Windows run in the VM
  through SSH (host port 22220 already works): copy the zip in, run `nx-edit --version`-style
  smoke checks and the test binary built for Windows.
- The USB path is first verified read-only in the VM, with Thomas present, exactly as the first
  Linux contact was: connect, read the current preset, close.

## Open questions for the builder

- Whether HX Edit's WinUSB driver covers `MI_00` alone or the whole device, and whether `nusb`
  can open it while the Line 6 audio driver holds the audio interfaces. If not, nx-edit cannot
  use the pedal on Windows without a driver change, and Thomas decides what to do.
- Whether the CustomTone HTTP client and its TLS build for the GNU target without extra system
  libraries (prefer `rustls`).

## Left to the builder

Crate layout of the Windows branches, how the flake exposes the Windows build, the exact smoke
tests in the VM, and the zip layout.
