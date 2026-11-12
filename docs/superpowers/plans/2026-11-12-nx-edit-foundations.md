# nx-edit Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build everything of nx-edit that needs neither the pedal nor captured traffic: the workspace and dev shell, `.hlx` presets, the HX Edit model catalog, the usbmon capture reader, and an app shell that renders a preset in layout B with Scythe's theme.

**Architecture:** Cargo workspace of small crates. Pure data crates (`nx-hlx`, `nx-catalog`, `nx-capture`) have no UI or USB dependency; `nx-app` is an eframe shell that reads a `.hlx` file and the catalog and draws the chain read-only. The protocol crates (`nx-proto`, `nx-usb`, `nx-session`) come in the next plan, once captures exist.

**Tech Stack:** Rust (nixpkgs 1.95, edition 2024), serde + serde_json (`preserve_order`), thiserror, pcap-file, eframe/egui 0.37, Nix flake dev shell.

**Spec:** `docs/superpowers/specs/2026-11-12-hx-stomp-editor-design.md`

## Global Constraints

- Edition 2024; egui/eframe `0.37`, matching Scythe.
- No code taken from tonepush, fretwire or openhx. General crates only.
- Line 6 data is read at runtime from an HX Edit `res` directory. The only Line 6-derived files committed are small test fixtures (the repo is private).
- Dev shell and app must run with `LD_LIBRARY_PATH` unset from the global environment; the flake sets its own.
- No writes to the pedal anywhere in this plan.

## Review Focus

- A `.hlx` written by HX Edit with fields nx-hlx does not model must round-trip unchanged (unknown keys preserved, key order kept). Test in Task 2.
- A real HX Edit `res` directory contains `.models` entries with missing optional keys (`name`, `category`, `load`, `displayType`); the catalog must load all 681 models without failing. Test in Task 3 (fixture with each key missing, plus an ignored real-data test).
- A pcap containing traffic for other USB devices and both URB submit and complete events must yield only the selected device's completed transfers, in order. Test in Task 4.
- A preset whose blocks reference a model id absent from the catalog (newer firmware) must still render, showing the raw id. Test in Task 5.
- Paths: a split whose lower path holds blocks (the cab → send presets) must render two lanes; a preset with an empty lower path renders one lane. Test in Task 5.

---

### Task 1: Workspace and dev shell

**Files:**
- Create: `Cargo.toml` (workspace, `resolver = "3"`, `edition = "2024"`, members `crates/*`)
- Create: `flake.nix` (devShell: cargo, rustc, clippy, rustfmt, pkg-config; runtime libs libxkbcommon, wayland, libGL, vulkan-loader, xorg.libX11/libXcursor/libXrandr/libXi exported via `LD_LIBRARY_PATH` in `shellHook`)
- Create: `.gitignore` (`target/`, `.superpowers/`, `/local/`)
- Create: `crates/nx-hlx/` skeleton (lib with one passing smoke test)

- [ ] **Step 1:** Write the files above; `nx-hlx` has `#[test] fn smoke() {}`.
- [ ] **Step 2:** Run `env -u LD_LIBRARY_PATH nix develop -c cargo test --workspace`. Expected: 1 test passes.
- [ ] **Step 3:** Commit `chore: cargo workspace and nix dev shell`.

### Task 2: nx-hlx, lossless `.hlx` presets

**Files:**
- Create: `crates/nx-hlx/src/lib.rs`, `crates/nx-hlx/src/preset.rs`
- Test: `crates/nx-hlx/tests/roundtrip.rs`, fixtures `crates/nx-hlx/tests/fixtures/*.hlx` (copy `~/vms/presets/*.hlx`)

**Interfaces:**
- Produces:
  - `pub struct Preset { raw: serde_json::Value }` with `Preset::from_slice(&[u8]) -> Result<Preset, HlxError>`, `Preset::to_vec_pretty(&self) -> Vec<u8>`
  - `Preset::name(&self) -> &str`, `device(&self) -> u32`, `device_version(&self) -> u32`
  - `Preset::blocks(&self) -> Vec<Block>` where `pub struct Block { pub key: String, pub model: String, pub path: u8, pub position: u8, pub enabled: bool, pub params: Vec<(String, serde_json::Value)> }` (params = non-`@` keys in file order), sorted by `(position, path)`
  - `Preset::split(&self) -> Option<SplitInfo>` with `pub struct SplitInfo { pub model: String, pub position: u8 }`
  - `Preset::snapshot_names(&self) -> Vec<String>`
  - `pub const HX_STOMP_DEVICE: u32 = 2162694;`
  - `pub enum HlxError { Json(serde_json::Error), Shape(&'static str) }` (thiserror)

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn every_fixture_round_trips_byte_for_byte_after_normalising() // parse → to_vec_pretty → parse == first parse (Value equality) AND key order of data.tone.dsp0 unchanged
#[test] fn unknown_keys_survive() // insert "data.tone.dsp0.block0.ZZ_unknown": 1 → round trip keeps it
#[test] fn revv_dm_blocks() // Revv_DM.hlx: blocks()[2].model == "HD2_AmpRevvGenRed"; cab and HD2_SendMono1 have path 1; split().unwrap().position == 6
#[test] fn snapshot_names_of_revv_dm() // == ["808", "33", "Lead"]
#[test] fn device_is_hx_stomp() // device() == HX_STOMP_DEVICE, device_version() == 0x0370_0000
```

- [ ] **Step 2:** `cargo test -p nx-hlx`. Expected: compile failure / FAIL.
- [ ] **Step 3:** Implement with `serde_json` feature `preserve_order`; accessors walk `raw` and return `HlxError::Shape` when `data.tone.dsp0` is missing.
- [ ] **Step 4:** `cargo test -p nx-hlx`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(hlx): lossless .hlx presets with block and snapshot accessors`.

### Task 3: nx-catalog, HX Edit model data

**Files:**
- Create: `crates/nx-catalog/src/{lib.rs,models.rs,categories.rs,import.rs}`
- Test: `crates/nx-catalog/tests/catalog.rs`, fixtures `crates/nx-catalog/tests/fixtures/res/{amp.models,distortion.models,HX_ModelCatalog.json}` (hand-trimmed: 2 amps, 2 distortions, one entry missing each optional key)

**Interfaces:**
- Produces:
  - `pub struct Catalog` with `Catalog::load(res_dir: &Path) -> Result<Catalog, CatalogError>` (reads every `*.models` file plus `HX_ModelCatalog.json`)
  - `Catalog::model(&self, id: &str) -> Option<&ModelDef>`; `Catalog::categories(&self) -> &[Category]`
  - `pub struct ModelDef { pub id: String, pub name: Option<String>, pub category: Option<u32>, pub load: Option<f32>, pub params: Vec<ParamDef> }`
  - `pub struct ParamDef { pub id: String, pub name: String, pub value_type: u8, pub display_type: Option<String>, pub min: f64, pub max: f64, pub default: serde_json::Value }`
  - `pub struct Category { pub id: u32, pub name: String, pub color: u32, pub model_ids: Vec<String> }` (models from direct `models` and all `subcategories[].models`, order kept; color parsed from `"0xf5901e"`)
  - `pub fn default_res_dir() -> PathBuf` = `$XDG_DATA_HOME/nx-edit/hxedit-res` (fallback `~/.local/share/...`)
  - `pub fn import_installer(installer: &Path, dest: &Path) -> Result<(), CatalogError>`: runs `7z x` into a temp dir, copies `HX Edit/res/*.models`, `HX_ModelCatalog.json`, `HelixControls.json` into `dest`
- Consumes: nothing.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn loads_fixture_models() // model("HD2_AmpRevvGenRed").unwrap().params contains id "Drive", min 0.0, max 1.0
#[test] fn missing_optional_keys_are_none() // fixture entry without name/category/load → None, still loaded
#[test] fn categories_flatten_subcategories() // categories()[1].name == "Distortion", color == 0xf5901e, model_ids contains "HD2_DistKinkyBoost"
#[test] #[ignore] fn real_res_dir_loads_all_models() // NX_HXEDIT_RES=...; >= 681 models; model("HD2_AmpPVPanama").is_some()
```

- [ ] **Step 2:** `cargo test -p nx-catalog`. Expected: FAIL.
- [ ] **Step 3:** Implement with serde structs (`#[serde(rename = "symbolicID")]`, optional fields `Option`); unknown keys ignored.
- [ ] **Step 4:** `cargo test -p nx-catalog` PASS; then `NX_HXEDIT_RES=<extracted res> cargo test -p nx-catalog -- --ignored` PASS.
- [ ] **Step 5:** Commit `feat(catalog): load HX Edit model definitions and categories`.

### Task 4: nx-capture, usbmon capture reader

**Files:**
- Create: `crates/nx-capture/src/{lib.rs,usbmon.rs}`, `crates/nx-capture/src/bin/nx-capture.rs`
- Create: `docs/capture.md` (how to capture: `boot.kernelModules = [ "usbmon" ]`, `sudo dumpcap -i usbmon3 -w file.pcapng`, VM setup, one action per capture, scrub rules)
- Test: `crates/nx-capture/tests/usbmon.rs` (builds a pcap in memory)

**Interfaces:**
- Produces:
  - `pub struct Transfer { pub ts_us: u64, pub bus: u16, pub device: u8, pub endpoint: u8 /* incl. 0x80 dir bit */, pub kind: TransferKind, pub data: Vec<u8> }`, `pub enum TransferKind { Bulk, Interrupt, Control, Iso }`
  - `pub fn read_transfers(r: impl Read, filter: &Filter) -> Result<Vec<Transfer>, CaptureError>`; `pub struct Filter { pub bus: Option<u16>, pub device: Option<u8>, pub endpoints: Option<Vec<u8>> }`
  - Handles pcap and pcapng (`pcap-file`), linktypes 189 (`USB_LINUX`, 48-byte header) and 220 (`USB_LINUX_MMAPPED`, 64-byte header). Keeps OUT data from submit events (`'S'`) and IN data from complete events (`'C'`), skips the rest.
  - CLI: `nx-capture <file> [--bus N] [--device N] [--ep 0x01,0x81]` prints `ts  dir  ep  len  hex` per transfer.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn keeps_out_submit_and_in_complete_only()
#[test] fn filters_other_devices()        // device 7 traffic dropped when filter.device = Some(42)
#[test] fn reads_linktype_220_and_189()
#[test] fn preserves_order_by_timestamp()
```
(Build records with a test helper `fn urb(event: u8, kind: u8, ep: u8, dev: u8, bus: u16, data: &[u8]) -> Vec<u8>` following the usbmon header layout in Linux `Documentation/usb/usbmon.rst`.)

- [ ] **Step 2:** `cargo test -p nx-capture`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p nx-capture`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(capture): read usbmon captures into USB transfers`.

### Task 5: nx-app shell, layout B, read-only

**Files:**
- Create: `crates/nx-app/src/{main.rs,theme.rs,layout.rs,view.rs}`
- `theme.rs`: copy of Scythe `crates/scythe-app/src/theme.rs` colours (BG, PANEL, RAISED, RAISED_HI, LANE, LANE_SELECTED, TEXT, TEXT_MID, TEXT_DIM, SWITCH_OFF, YELLOW, ORANGE, GREEN, PINK, ACCENT) and its egui style setup; header comment names the source file.
- Test: `crates/nx-app/src/layout.rs` unit tests

**Interfaces:**
- Consumes: `nx_hlx::{Preset, Block}`, `nx_catalog::Catalog`
- Produces:
  - `pub struct Lane { pub path: u8, pub tiles: Vec<Tile> }`, `pub struct Tile { pub block_key: String, pub label: String, pub category_color: Option<u32>, pub enabled: bool }`
  - `pub fn lanes(preset: &Preset, catalog: Option<&Catalog>) -> Vec<Lane>`: lane 0 always; lane 1 only when a block has path 1. Label = catalog model name, else the raw model id.
  - App: `nx-edit [preset.hlx]` opens a window: left category list (from catalog, or empty), centre lanes with tiles and output labels (`Main L/R`, and `Send` when an `HD2_Send*` block exists), bottom knob row for the selected tile showing param name and value (0–1 values shown ×10 with one decimal). Read-only; no device code. Catalog from `default_res_dir()` if present.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn revv_dm_has_two_lanes()          // lanes(..)[1].tiles labels == ["4x12 Cali V30"-or-id, "HD2_SendMono1"-or-name]
#[test] fn revv_home_has_one_lane()
#[test] fn unknown_model_shows_raw_id()     // catalog without the model → label == model id
```

- [ ] **Step 2:** `cargo test -p nx-app`. Expected: FAIL.
- [ ] **Step 3:** Implement `layout.rs`, then the eframe view.
- [ ] **Step 4:** `cargo test -p nx-app` PASS; `nix develop -c cargo run -p nx-app -- crates/nx-hlx/tests/fixtures/Revv_DM.hlx` opens a window showing two lanes (verify with a screenshot).
- [ ] **Step 5:** Commit `feat(app): read-only preset view in layout B with Scythe theme`.
