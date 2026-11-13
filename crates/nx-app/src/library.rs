//! Presets kept as `.hlx` files: one folder per source (`mine`, `customtone`, ...) under the
//! library directory next to the imported HX Edit data.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    pub path: PathBuf,
    /// The folder under the library it came from.
    pub folder: String,
    /// The preset's own name, from `data.meta.name`.
    pub name: String,
}

#[derive(Debug, Default)]
pub struct Scan {
    /// Sorted by folder, then name.
    pub entries: Vec<LibraryEntry>,
    /// `.hlx` files that are not presets nx-edit can read.
    pub skipped: Vec<PathBuf>,
}

pub fn default_dir() -> PathBuf {
    let res = nx_catalog::default_res_dir();
    res.parent().map_or_else(|| PathBuf::from("library"), |data| data.join("library"))
}

pub fn scan(dir: &Path) -> Scan {
    let mut scan = Scan::default();
    let Ok(folders) = std::fs::read_dir(dir) else { return scan };
    let listed = |f: &std::fs::DirEntry| f.path().is_dir() && !f.file_name().to_string_lossy().starts_with('.');
    for folder in folders.flatten().filter(listed) {
        let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
        for path in files.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "hlx")) {
            match std::fs::read(&path).ok().and_then(|bytes| nx_hlx::Preset::from_slice(&bytes).ok()) {
                Some(preset) => scan.entries.push(LibraryEntry {
                    folder: folder.file_name().to_string_lossy().into_owned(),
                    name: preset.name().to_owned(),
                    path,
                }),
                None => scan.skipped.push(path),
            }
        }
    }
    scan.entries.sort_by(|a, b| (&a.folder, &a.name).cmp(&(&b.folder, &b.name)));
    scan.skipped.sort();
    scan
}

/// Copies a preset file into the library's `mine` folder, never over an existing file.
pub fn import(library: &Path, file: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    nx_hlx::Preset::from_slice(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    let stem = file.file_stem().map_or("preset".into(), |s| s.to_string_lossy().into_owned());
    let target = free_path(&library.join("mine"), &stem);
    std::fs::create_dir_all(library.join("mine")).map_err(|e| e.to_string())?;
    std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

/// Moves a preset into the library's `.trash` folder, which is not listed; nothing is deleted.
pub fn trash(library: &Path, file: &Path) -> Result<PathBuf, String> {
    let bin = library.join(".trash");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let stem = file.file_stem().map_or("preset".into(), |s| s.to_string_lossy().into_owned());
    let target = free_path(&bin, &stem);
    std::fs::rename(file, &target).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(target)
}

/// Renames the preset inside the file; the file keeps its name.
pub fn rename(file: &Path, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a preset needs a name".into());
    }
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut preset = nx_hlx::Preset::from_slice(&bytes).map_err(|e| e.to_string())?;
    preset.set_name(name);
    std::fs::write(file, preset.to_vec_pretty()).map_err(|e| format!("{}: {e}", file.display()))
}

/// Keeps a CustomTone download in the library's `customtone` folder, under the name the site gave
/// it with path separators replaced. Only HX Stomp presets are kept.
pub fn save_download(library: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let preset = nx_hlx::Preset::from_slice(bytes).map_err(|e| format!("{file_name}: not a preset ({e})"))?;
    if preset.device() != nx_hlx::HX_STOMP_DEVICE {
        return Err(format!("{file_name}: a preset for another device ({})", preset.device()));
    }
    let safe: String = file_name.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    let stem = safe.strip_suffix(".hlx").unwrap_or(&safe);
    let folder = library.join("customtone");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let target = free_path(&folder, stem);
    std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

/// `dir/stem.hlx`, or `dir/stem 2.hlx` and so on when that exists.
fn free_path(dir: &Path, stem: &str) -> PathBuf {
    (1..)
        .map(|n| if n == 1 { dir.join(format!("{stem}.hlx")) } else { dir.join(format!("{stem} {n}.hlx")) })
        .find(|p| !p.exists())
        .expect("some number is free")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-hlx/tests/fixtures").join(name)).unwrap()
    }

    fn library_with(test: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nx-library-{}-{test}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        for (path, bytes) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        dir
    }

    #[test]
    fn presets_are_found_per_folder_with_the_name_inside_the_file() {
        let dir = library_with("folders", &[
            ("mine/Revv_DM.hlx", fixture("Revv_DM.hlx")),
            ("mine/Mark_Lead.hlx", fixture("Mark_Lead.hlx")),
            ("customtone/4088879.hlx", fixture("5150_DM.hlx")),
        ]);
        let scan = scan(&dir);
        let found: Vec<(&str, &str)> = scan.entries.iter().map(|e| (e.folder.as_str(), e.name.as_str())).collect();
        assert_eq!(found, [("customtone", "5150 DM"), ("mine", "Mark Lead"), ("mine", "Revv DM")]);
        assert!(scan.skipped.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_that_are_not_presets_are_skipped_and_counted() {
        let dir = library_with("skipped", &[
            ("mine/Revv_DM.hlx", fixture("Revv_DM.hlx")),
            ("mine/broken.hlx", b"not json".to_vec()),
            ("mine/notes.txt", b"setlist".to_vec()),
        ]);
        let scan = scan(&dir);
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.skipped, [dir.join("mine/broken.hlx")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_library_is_empty() {
        let scan = scan(Path::new("/nonexistent/nx-edit/library"));
        assert!(scan.entries.is_empty() && scan.skipped.is_empty());
    }

    #[test]
    fn a_dropped_preset_is_copied_into_mine_without_overwriting() {
        let dir = library_with("import", &[("mine/Revv_DM.hlx", fixture("Revv_DM.hlx"))]);
        let outside = dir.join("Revv_DM.hlx");
        std::fs::write(&outside, fixture("Revv_DM.hlx")).unwrap();
        let copied = import(&dir, &outside).unwrap();
        assert_eq!(copied, dir.join("mine/Revv_DM 2.hlx"));
        assert_eq!(scan(&dir).entries.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_preset_is_not_imported() {
        let dir = library_with("import-bad", &[]);
        std::fs::create_dir_all(&dir).unwrap();
        let outside = dir.join("notes.hlx");
        std::fs::write(&outside, b"not json").unwrap();
        assert!(import(&dir, &outside).is_err());
        assert!(!dir.join("mine/notes.hlx").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_deleted_preset_goes_to_the_trash_which_is_not_listed() {
        let dir = library_with("trash", &[("mine/Revv_DM.hlx", fixture("Revv_DM.hlx"))]);
        let kept = trash(&dir, &dir.join("mine/Revv_DM.hlx")).unwrap();
        assert!(kept.starts_with(dir.join(".trash")) && kept.exists());
        assert!(scan(&dir).entries.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn renaming_changes_the_name_inside_the_file() {
        let dir = library_with("rename", &[("mine/Revv_DM.hlx", fixture("Revv_DM.hlx"))]);
        rename(&dir.join("mine/Revv_DM.hlx"), "  Revv Live ").unwrap();
        assert_eq!(scan(&dir).entries[0].name, "Revv Live");
        assert!(rename(&dir.join("mine/Revv_DM.hlx"), "   ").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_download_is_kept_in_customtone_under_a_safe_name() {
        let dir = library_with("download", &[]);
        let saved = save_download(&dir, "../Revv/DM.hlx", &fixture("Revv_DM.hlx")).unwrap();
        assert_eq!(saved, dir.join("customtone/.._Revv_DM.hlx"));
        let again = save_download(&dir, "../Revv/DM.hlx", &fixture("Revv_DM.hlx")).unwrap();
        assert_eq!(again, dir.join("customtone/.._Revv_DM 2.hlx"));
        assert_eq!(scan(&dir).entries.iter().filter(|e| e.folder == "customtone").count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_download_that_is_not_an_hx_stomp_preset_is_refused() {
        let dir = library_with("download-bad", &[]);
        assert!(save_download(&dir, "page.hlx", b"<html>Registration required</html>").is_err());
        let mut helix: serde_json::Value = serde_json::from_slice(&fixture("Revv_DM.hlx")).unwrap();
        helix["data"]["device"] = serde_json::json!(2162689);
        assert!(save_download(&dir, "helix.hlx", &serde_json::to_vec(&helix).unwrap()).is_err());
        assert!(!dir.join("customtone").exists() || scan(&dir).entries.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
