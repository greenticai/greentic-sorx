//! `--answers env:NAME` reads the answers JSON from an environment variable,
//! so a container needs no file for them. Anything else is a file path.
//!
//! A value staged from an environment variable is written to a uniquely
//! named temp file so the existing file-based answers loader can read it
//! unchanged. That file carries the raw answers payload, so it is created
//! owner-only (unix: directory `0700`, file `0600`) and removed — file and
//! directory — as soon as it has been read, when [`AnswersSource`]'s staged
//! [`tempfile::TempDir`] drops. It is never left on disk for the life of a
//! long-running `start`.

use std::path::{Path, PathBuf};

use crate::{CliError, CliResult};

/// A resolved `--answers` source: `path()` is what the answers loader reads.
///
/// For a plain file path this is a thin wrapper with nothing to clean up.
/// For a value staged from `env:NAME`, dropping it removes the staged file
/// and its directory (via the held `TempDir`'s own `Drop`) — callers that
/// read the file should do so and then explicitly `drop` (or otherwise let
/// go of) the value at that point, rather than holding it until the end of
/// a long-running command.
#[derive(Debug)]
pub(crate) struct AnswersSource {
    path: PathBuf,
    /// Kept alive only so its directory is removed when this drops; `None`
    /// for a plain file path, which this type does not own and must not
    /// delete. Each call to `resolve` that stages a value gets its OWN
    /// uniquely named directory (via `tempfile`), so two concurrent staged
    /// answers — e.g. two tests in one process — can never collide on one
    /// path and race each other's cleanup.
    _staged: Option<tempfile::TempDir>,
}

impl AnswersSource {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Resolve an `--answers` argument to a readable answers source.
pub(crate) fn resolve(raw: PathBuf) -> CliResult<AnswersSource> {
    let Some(name) = raw.to_str().and_then(|text| text.strip_prefix("env:")) else {
        return Ok(AnswersSource {
            path: raw,
            _staged: None,
        });
    };
    let value = std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CliError::usage(format!(
                "--answers names the environment variable `{name}`, which is unset or empty"
            ))
        })?;
    let dir = create_staging_dir()
        .map_err(|err| CliError::runtime(format!("cannot stage answers: {err}")))?;
    let path = dir.path().join("answers.json");
    write_staged_answers(&path, &value)
        .map_err(|err| CliError::runtime(format!("cannot stage answers: {err}")))?;
    Ok(AnswersSource {
        path,
        _staged: Some(dir),
    })
}

/// Create a uniquely named staging directory, owner-only (`0700`) on unix.
///
/// `tempfile::Builder` (rather than a fixed, pid-keyed path) is what makes
/// this safe to call more than once per process: `std::process::id()` is
/// stable for the whole process, so a fixed `greentic-sorx-answers-<pid>`
/// path staged by two calls in one process — e.g. two tests running under
/// the default parallel test harness — resolves to the SAME directory, and
/// the first one to finish removes it out from under the other. A `tempfile`
/// directory carries its own random suffix, so each call gets its own path
/// regardless of how many run concurrently in this process.
#[cfg(unix)]
fn create_staging_dir() -> std::io::Result<tempfile::TempDir> {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .prefix("greentic-sorx-answers-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
}

#[cfg(not(unix))]
fn create_staging_dir() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("greentic-sorx-answers-")
        .tempdir()
}

/// Write the staged answers file owner-only (`0600` on unix), rather than
/// creating it world-readable and narrowing permissions after the fact — the
/// content never exists on disk at a wider mode.
#[cfg(unix)]
fn write_staged_answers(path: &Path, value: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(value.as_bytes())
}

#[cfg(not(unix))]
fn write_staged_answers(path: &Path, value: &str) -> std::io::Result<()> {
    std::fs::write(path, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_is_returned_unchanged() {
        let path = PathBuf::from("/tmp/answers.json");
        let source = resolve(path.clone()).expect("plain path");
        assert_eq!(source.path(), path.as_path());
    }

    #[test]
    fn an_unset_variable_names_itself_in_the_error() {
        // SAFETY: test-only; no other thread in this process reads or writes
        // this variable name.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_UNSET");
        }
        let err = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_UNSET"))
            .expect_err("unset variable must be refused");
        assert!(
            err.message.contains("SORX_ANSWERS_SOURCE_TEST_UNSET"),
            "{}",
            err.message
        );
    }

    #[test]
    fn an_empty_variable_is_treated_as_unset() {
        // SAFETY: test-only; see above.
        unsafe {
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_EMPTY", "   ");
        }
        let err = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_EMPTY"))
            .expect_err("blank variable must be refused");
        assert!(
            err.message.contains("SORX_ANSWERS_SOURCE_TEST_EMPTY"),
            "{}",
            err.message
        );
        // SAFETY: test-only; see above.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_EMPTY");
        }
    }

    #[test]
    fn a_set_variable_is_staged_to_a_readable_file() {
        // SAFETY: test-only; see above.
        unsafe {
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_SET", "{\"ok\":true}");
        }
        let source = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_SET")).expect("staged");
        let staged = std::fs::read_to_string(source.path()).expect("read staged answers");
        assert_eq!(staged, "{\"ok\":true}");
        // SAFETY: test-only; see above.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_SET");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_staged_file_is_owner_only_and_removed_once_dropped() {
        use std::os::unix::fs::PermissionsExt;

        // SAFETY: test-only; see above.
        unsafe {
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_PERMS", "{\"ok\":true}");
        }
        let source = resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_PERMS")).expect("staged");
        let path = source.path().to_path_buf();
        let dir = path
            .parent()
            .expect("staged file has a parent directory")
            .to_path_buf();

        let file_mode = std::fs::metadata(&path)
            .expect("staged file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600, "staged file must be owner-only");

        let dir_mode = std::fs::metadata(&dir)
            .expect("staged directory metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "staged directory must be owner-only");

        drop(source);
        assert!(
            !path.exists(),
            "staged answers file must be removed once dropped"
        );
        assert!(
            !dir.exists(),
            "staged answers directory must be removed once dropped"
        );

        // SAFETY: test-only; see above.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_PERMS");
        }
    }

    /// Regression for the staging-path collision: two calls in one process
    /// must never resolve to the same directory. Before `tempfile` replaced
    /// the fixed `greentic-sorx-answers-<pid>` path, this raced with any
    /// other test staging a value at the same time under the default
    /// parallel test harness — one call's `Drop` could delete the other's
    /// still-in-use file.
    #[test]
    fn two_concurrent_staged_sources_never_share_a_directory() {
        // SAFETY: test-only; see above.
        unsafe {
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_COLLISION_A", "{\"a\":1}");
            std::env::set_var("SORX_ANSWERS_SOURCE_TEST_COLLISION_B", "{\"b\":2}");
        }
        let a =
            resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_COLLISION_A")).expect("staged a");
        let b =
            resolve(PathBuf::from("env:SORX_ANSWERS_SOURCE_TEST_COLLISION_B")).expect("staged b");
        assert_ne!(
            a.path().parent(),
            b.path().parent(),
            "two staged answers must not share a directory"
        );
        // Dropping `a` must not disturb `b`, which is exactly the failure
        // mode a shared pid-keyed directory produced.
        drop(a);
        let staged = std::fs::read_to_string(b.path()).expect("b must still be readable");
        assert_eq!(staged, "{\"b\":2}");
        // SAFETY: test-only; see above.
        unsafe {
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_COLLISION_A");
            std::env::remove_var("SORX_ANSWERS_SOURCE_TEST_COLLISION_B");
        }
    }
}
