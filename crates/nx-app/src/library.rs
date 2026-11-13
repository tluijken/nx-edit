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
    for folder in folders.flatten().filter(|f| f.path().is_dir()) {
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
}
