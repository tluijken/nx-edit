# Reverse engineering HX Edit

Static analysis of HX Edit 3.82 for Windows, taken from the installer
(`HX Edit v3.82 Installer.exe`, NSIS, extracted with 7-Zip; nothing is installed or run).

## Tooling

- REA 6.8 (`rea-agents` from Scythe's `node_modules`) with Ghidra 12.1.2 and OpenJDK 25 from
  Scythe's devbox. The MCP registration in `~/.claude.json` points there and sets
  `REA_GHIDRA_STARTUP_TIMEOUT_MS=3600000`: the default 330 s is too short for `HX Edit.exe`.
- Ghidra analysis snapshot: `local/re/hxedit.snapshot` (git-ignored, with the binary it belongs to,
  `local/re/HXEdit-3.82.exe`, sha256 `f7faac49…6cab66`).

## Findings

- `HX Edit.exe` is 32-bit x86 PE, 13 MB.
- Imports (`rea inspect-pe-imports`): no WinUSB. Device I/O goes through `Interposer.dll` (Line 6,
  installed under `ProgramData\Line 6\Interposer`), SETUPAPI enumerates devices, WININET does HTTP,
  ADVAPI32 `CryptGenRandom` is used (probably for the account sign-in).
- libusb is linked in (its URL string is present); the interposer is Line 6's layer around it.
- Device-specific updaters are named in the binary: `P33Updater` (P33 is the HX Stomp; its device
  info reply starts `"P33Main"`), `P34Updater`, `P36Updater`, `F15Updater`, plus
  `DeviceUpdateReadyForExternalUpdate` and `UpdateUpdaterApp`.
- The Line 6 web API is reached under a versioned REST path, `/api/rest/v%d`.

## Next

- Find the code that builds requests for `/api/rest/v%d` (sign-in, update check, download) before
  capturing the network side in the VM.
- Locate `P33Updater` and the bootloader (`1fc9:000c`) handling for the deferred firmware work.
- Check whether HX Edit validates the preset document's section table before writing a preset.
