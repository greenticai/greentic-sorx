//! Filesystem and `.gtpack` zip helpers shared by the test-bundle driver.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(super) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| format!("failed to encode JSON for {}: {err}", path.display()))?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

pub(super) fn extract_zip_to_dir(pack_path: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_dir_all(target)
            .map_err(|err| format!("failed to clean {}: {err}", target.display()))?;
    }
    fs::create_dir_all(target)
        .map_err(|err| format!("failed to create {}: {err}", target.display()))?;
    let file = fs::File::open(pack_path)
        .map_err(|err| format!("failed to open {}: {err}", pack_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| format!("failed to read zip {}: {err}", pack_path.display()))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("failed to read zip entry {index}: {err}"))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            return Err(format!("unsafe zip entry path `{}`", entry.name()));
        };
        let out_path = target.join(name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let mut out = fs::File::create(&out_path)
            .map_err(|err| format!("failed to create {}: {err}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|err| format!("failed to extract {}: {err}", out_path.display()))?;
    }
    Ok(())
}

pub(super) fn pack_dir(source: &Path, pack_path: &Path) -> Result<(), String> {
    let tmp = pack_path.with_extension("gtpack.tmp");
    let file = fs::File::create(&tmp)
        .map_err(|err| format!("failed to create {}: {err}", tmp.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for path in sorted_files(source)? {
        let rel = path
            .strip_prefix(source)
            .map_err(|err| format!("failed to compute relative zip path: {err}"))?
            .to_string_lossy()
            .replace('\\', "/");
        writer
            .start_file(rel, options)
            .map_err(|err| format!("failed to start zip entry: {err}"))?;
        let bytes =
            fs::read(&path).map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        writer
            .write_all(&bytes)
            .map_err(|err| format!("failed to write zip entry: {err}"))?;
    }
    writer
        .finish()
        .map_err(|err| format!("failed to finish {}: {err}", tmp.display()))?;
    fs::rename(&tmp, pack_path).map_err(|err| {
        format!(
            "failed to replace {} with {}: {err}",
            pack_path.display(),
            tmp.display()
        )
    })
}

pub(super) fn sorted_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|err| format!("failed to read {}: {err}", root.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub(super) fn absolutize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map_err(|err| format!("failed to resolve current directory: {err}"))
            .map(|cwd| cwd.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, contents).expect("write file");
    }

    #[test]
    fn write_json_pretty_prints_and_appends_a_trailing_newline() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("card.json");
        write_json(&path, &json!({"a": 1})).expect("writes");
        let text = fs::read_to_string(&path).expect("read");
        assert_eq!(text, "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn write_json_creates_missing_parent_directories() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("deep/nested/card.json");
        write_json(&path, &json!([])).expect("writes");
        assert!(path.is_file());
    }

    #[test]
    fn write_json_reports_the_path_when_the_parent_is_a_file() {
        let dir = TempDir::new().expect("tempdir");
        let blocker = dir.path().join("blocker");
        write(&blocker, "not a directory");
        let path = blocker.join("card.json");
        let err = write_json(&path, &json!({})).unwrap_err();
        assert!(err.contains("failed to create"), "unexpected error: {err}");
    }

    #[test]
    fn sorted_files_walks_recursively_and_sorts_deterministically() {
        let dir = TempDir::new().expect("tempdir");
        write(&dir.path().join("b.txt"), "b");
        write(&dir.path().join("a/z.txt"), "z");
        write(&dir.path().join("a/a.txt"), "a");
        let files = sorted_files(dir.path()).expect("walk");
        let rel = files
            .iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .expect("relative")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(rel, vec!["a/a.txt", "a/z.txt", "b.txt"]);
    }

    #[test]
    fn sorted_files_returns_an_empty_list_for_an_empty_directory() {
        let dir = TempDir::new().expect("tempdir");
        assert!(sorted_files(dir.path()).expect("walk").is_empty());
    }

    #[test]
    fn sorted_files_reports_a_missing_root() {
        let dir = TempDir::new().expect("tempdir");
        let err = sorted_files(&dir.path().join("absent")).unwrap_err();
        assert!(err.starts_with("failed to read"), "unexpected error: {err}");
    }

    #[test]
    fn pack_dir_then_extract_zip_to_dir_round_trips_content() {
        let dir = TempDir::new().expect("tempdir");
        let source = dir.path().join("src");
        write(&source.join("assets/cards/welcome.json"), "{\"a\":1}");
        write(&source.join("manifest.yaml"), "id: demo");

        let pack = dir.path().join("default.gtpack");
        pack_dir(&source, &pack).expect("pack");
        assert!(pack.is_file());
        assert!(
            !pack.with_extension("gtpack.tmp").exists(),
            "tmp file left behind"
        );

        let out = dir.path().join("out");
        extract_zip_to_dir(&pack, &out).expect("extract");
        assert_eq!(
            fs::read_to_string(out.join("assets/cards/welcome.json")).expect("read"),
            "{\"a\":1}"
        );
        assert_eq!(
            fs::read_to_string(out.join("manifest.yaml")).expect("read"),
            "id: demo"
        );
    }

    #[test]
    fn pack_dir_replaces_an_existing_pack_in_place() {
        let dir = TempDir::new().expect("tempdir");
        let source = dir.path().join("src");
        write(&source.join("a.txt"), "first");
        let pack = dir.path().join("p.gtpack");
        write(&pack, "stale bytes");

        pack_dir(&source, &pack).expect("pack");

        let out = dir.path().join("out");
        extract_zip_to_dir(&pack, &out).expect("extract");
        assert_eq!(
            fs::read_to_string(out.join("a.txt")).expect("read"),
            "first"
        );
    }

    #[test]
    fn extract_zip_to_dir_cleans_a_pre_existing_target() {
        let dir = TempDir::new().expect("tempdir");
        let source = dir.path().join("src");
        write(&source.join("keep.txt"), "keep");
        let pack = dir.path().join("p.gtpack");
        pack_dir(&source, &pack).expect("pack");

        let out = dir.path().join("out");
        write(&out.join("stale.txt"), "stale");
        extract_zip_to_dir(&pack, &out).expect("extract");

        assert!(out.join("keep.txt").is_file());
        assert!(
            !out.join("stale.txt").exists(),
            "stale file survived extraction"
        );
    }

    #[test]
    fn extract_zip_to_dir_rejects_a_traversing_entry_instead_of_escaping_the_target() {
        let dir = TempDir::new().expect("tempdir");
        let pack = dir.path().join("evil.gtpack");
        let mut writer = ZipWriter::new(fs::File::create(&pack).expect("create zip"));
        writer
            .start_file("../escaped.txt", SimpleFileOptions::default())
            .expect("start entry");
        writer.write_all(b"pwned").expect("write entry");
        writer.finish().expect("finish zip");

        let out = dir.path().join("out");
        let err = extract_zip_to_dir(&pack, &out).unwrap_err();
        assert_eq!(err, "unsafe zip entry path `../escaped.txt`");
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "traversing entry escaped the target directory"
        );
    }

    #[test]
    fn extract_zip_to_dir_skips_directory_entries() {
        let dir = TempDir::new().expect("tempdir");
        let pack = dir.path().join("dirs.gtpack");
        let mut writer = ZipWriter::new(fs::File::create(&pack).expect("create zip"));
        writer
            .add_directory("assets/cards", SimpleFileOptions::default())
            .expect("add directory");
        writer
            .start_file("assets/cards/a.json", SimpleFileOptions::default())
            .expect("start entry");
        writer.write_all(b"{}").expect("write entry");
        writer.finish().expect("finish zip");

        let out = dir.path().join("out");
        extract_zip_to_dir(&pack, &out).expect("extract");
        assert!(out.join("assets/cards").is_dir());
        assert_eq!(
            fs::read_to_string(out.join("assets/cards/a.json")).expect("read"),
            "{}"
        );
    }

    #[test]
    fn extract_zip_to_dir_reports_a_missing_archive() {
        let dir = TempDir::new().expect("tempdir");
        let err = extract_zip_to_dir(&dir.path().join("absent.gtpack"), &dir.path().join("out"))
            .unwrap_err();
        assert!(err.starts_with("failed to open"), "unexpected error: {err}");
    }

    #[test]
    fn extract_zip_to_dir_reports_a_file_that_is_not_a_zip() {
        let dir = TempDir::new().expect("tempdir");
        let pack = dir.path().join("bogus.gtpack");
        write(&pack, "definitely not a zip archive");
        let err = extract_zip_to_dir(&pack, &dir.path().join("out")).unwrap_err();
        assert!(
            err.starts_with("failed to read zip"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pack_dir_reports_a_missing_source_directory() {
        let dir = TempDir::new().expect("tempdir");
        let err = pack_dir(&dir.path().join("absent"), &dir.path().join("p.gtpack")).unwrap_err();
        assert!(err.starts_with("failed to read"), "unexpected error: {err}");
    }

    #[test]
    fn absolutize_passes_through_an_absolute_path() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("bundle");
        assert_eq!(absolutize(&path).expect("absolutize"), path);
    }

    #[test]
    fn absolutize_joins_a_relative_path_onto_the_current_directory() {
        let resolved = absolutize(Path::new("relative/bundle")).expect("absolutize");
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("relative/bundle"));
    }
}
