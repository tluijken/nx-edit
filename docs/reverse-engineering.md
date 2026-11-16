# Reverse engineering HX Edit

Static inspection of HX Edit 3.82 for Windows, taken from the installer
(`HX Edit v3.82 Installer.exe`, NSIS, extracted with 7-Zip; nothing is installed or run). It is
limited to what interoperability with the pedal needs; the protocol itself comes from USB captures
(see [protocol.md](protocol.md)).

## Method

The import table and strings of `HX Edit.exe` were read with REA's PE tools. No code of HX Edit is
reproduced here or in nx-edit.

## Findings

- `HX Edit.exe` is 32-bit x86 PE, 13 MB.
- Imports: no WinUSB. Device I/O goes through `Interposer.dll` (Line 6, installed under
  `ProgramData\Line 6\Interposer`), SETUPAPI enumerates devices, WININET does HTTP, ADVAPI32
  `CryptGenRandom` is used (probably for the account sign-in).
- libusb is linked in (its URL string is present); the interposer is Line 6's layer around it.
- Device-specific updaters are named in the binary: `P33Updater` (P33 is the HX Stomp; its device
  info reply starts `"P33Main"`), `P34Updater`, `P36Updater`, `F15Updater`, plus
  `DeviceUpdateReadyForExternalUpdate` and `UpdateUpdaterApp`. Firmware updates stay with HX Edit.
- The Line 6 web API is reached under a versioned REST path, `/api/rest/v%d`.

## Open

- Whether HX Edit validates the preset document's section table before writing a preset; a capture
  of a write with a deliberately wrong table would tell, without looking at its code.
