//! One-file backup of everything stored on this computer (projects, imported products,
//! download logs), for moving to another computer, and restoring such a backup.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Marker inside every backup, so other zip files are refused on restore.
const MARK: &str = "buscraft-backup.txt";
/// Safety copies made before a restore (not included in backups).
pub const SAFETY_DIR: &str = "backups";

fn files(dir: &Path, rel: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if path.is_dir() {
            if rel.is_empty() && name == SAFETY_DIR {
                continue;
            }
            files(&path, &r, out);
        } else {
            out.push((path, r));
        }
    }
}

/// Write all files of the data folder into one zip. Returns the number of files.
pub fn create(data: &Path, out: &Path) -> Result<usize, String> {
    let mut list = vec![];
    files(data, "", &mut list);
    let f = std::fs::File::create(out).map_err(|e| format!("Could not create {}: {e}", out.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let err = |e: &dyn std::fmt::Display| format!("Could not write the backup: {e}");
    z.start_file(MARK, opt).map_err(|e| err(&e))?;
    let secs = crate::model::now_secs();
    let info = format!("BusCraft backup\nCreated: {}\nFiles: {}\n", crate::model::fmt_time(secs), list.len());
    z.write_all(info.as_bytes()).map_err(|e| err(&e))?;
    for (path, name) in &list {
        let data = std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        z.start_file(name.as_str(), opt).map_err(|e| err(&e))?;
        z.write_all(&data).map_err(|e| err(&e))?;
    }
    z.finish().map_err(|e| err(&e))?;
    Ok(list.len())
}

/// Check that a file is a BusCraft backup; returns its description.
pub fn describe(file: &Path) -> Result<String, String> {
    let f = std::fs::File::open(file).map_err(|e| format!("Could not open {}: {e}", file.display()))?;
    let mut z = zip::ZipArchive::new(f).map_err(|_| "This file is not a BusCraft backup.".to_string())?;
    let mut m = z.by_name(MARK).map_err(|_| "This file is not a BusCraft backup.".to_string())?;
    let mut s = String::new();
    let _ = m.read_to_string(&mut s);
    Ok(s.lines().skip(1).collect::<Vec<_>>().join(" · "))
}

/// Replace the data folder's files with the backup's. The current state is first saved as
/// a safety copy in `backups/`. Returns (files restored, safety copy).
pub fn restore(file: &Path, data: &Path) -> Result<(usize, Option<PathBuf>), String> {
    describe(file)?;
    let mut safety = None;
    if data.join("projects.json").exists() {
        let dir = data.join(SAFETY_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        let stamp = crate::model::fmt_time(crate::model::now_secs()).replace([' ', ':'], "-");
        let p = dir.join(format!("before-restore-{stamp}.zip"));
        create(data, &p)?;
        safety = Some(p);
    }
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut n = 0;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        if e.is_dir() {
            continue;
        }
        // Only safe relative names (no "..", no absolute paths)
        let Some(rel) = e.enclosed_name() else { continue };
        if rel == Path::new(MARK) {
            continue;
        }
        let target = data.join(&rel);
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("Could not create {}: {e}", p.display()))?;
        }
        let mut buf = vec![];
        e.read_to_end(&mut buf).map_err(|e| format!("The backup is damaged: {e}"))?;
        std::fs::write(&target, buf).map_err(|e| format!("Could not write {}: {e}", target.display()))?;
        n += 1;
    }
    Ok((n, safety))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_and_restore_round_trip() {
        let base = std::env::temp_dir().join(format!("buscraft-backup-test-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(a.join("library").join("M-0001")).unwrap();
        std::fs::write(a.join("projects.json"), "{\"projects\":[]}").unwrap();
        std::fs::write(a.join("library").join("M-0001").join("app.json"), "x".repeat(5000)).unwrap();
        std::fs::create_dir_all(a.join(SAFETY_DIR)).unwrap();
        std::fs::write(a.join(SAFETY_DIR).join("old.zip"), "skip me").unwrap();
        let zip = base.join("backup.zip");
        assert_eq!(create(&a, &zip).unwrap(), 2);
        assert!(describe(&zip).unwrap().contains("Files: 2"));
        // Restoring over existing data keeps a safety copy
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("projects.json"), "old").unwrap();
        let (n, safety) = restore(&zip, &b).unwrap();
        assert_eq!(n, 2);
        assert!(safety.unwrap().exists());
        assert_eq!(std::fs::read_to_string(b.join("projects.json")).unwrap(), "{\"projects\":[]}");
        assert_eq!(std::fs::read_to_string(b.join("library").join("M-0001").join("app.json")).unwrap().len(), 5000);
        // Other files are refused
        std::fs::write(base.join("other.zip"), "not a zip").unwrap();
        assert!(restore(&base.join("other.zip"), &b).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
