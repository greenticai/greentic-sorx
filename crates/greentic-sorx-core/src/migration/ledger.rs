//! Durable JSON-file ledger that persists `AppliedMigrations` across process
//! restarts. Mirrors the design of `LocalDeploymentRegistryStore` in
//! `deployment.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{SorxError, SorxResult};
use crate::migration::runner::AppliedMigrations;

/// JSON-file-backed store for the applied-migration ledger.
///
/// A missing file is treated as an empty (default) ledger — no error is raised.
/// On [`save`][LocalMigrationLedger::save], any missing parent directories are
/// created automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMigrationLedger {
    path: PathBuf,
}

impl LocalMigrationLedger {
    /// Create a ledger that persists to `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Return the file path this ledger writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the applied-migration set from disk.
    ///
    /// If the file does not exist, returns an empty [`AppliedMigrations`]
    /// rather than an error.
    pub fn load(&self) -> SorxResult<AppliedMigrations> {
        if !self.path.exists() {
            return Ok(AppliedMigrations::default());
        }
        let bytes = fs::read(&self.path).map_err(|err| {
            SorxError::new(
                "migration_ledger_io",
                format!("failed to read {}: {err}", self.path.display()),
            )
        })?;
        serde_json::from_slice(&bytes).map_err(|err| {
            SorxError::new(
                "migration_ledger_decode",
                format!("failed to decode {}: {err}", self.path.display()),
            )
        })
    }

    /// Persist the applied-migration set to disk.
    ///
    /// Creates any missing parent directories before writing.
    pub fn save(&self, applied: &AppliedMigrations) -> SorxResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                SorxError::new(
                    "migration_ledger_io",
                    format!("failed to create {}: {err}", parent.display()),
                )
            })?;
        }
        let encoded = serde_json::to_vec_pretty(applied).map_err(|err| {
            SorxError::new(
                "migration_ledger_decode",
                format!("failed to encode migration ledger: {err}"),
            )
        })?;
        fs::write(&self.path, encoded).map_err(|err| {
            SorxError::new(
                "migration_ledger_io",
                format!("failed to write {}: {err}", self.path.display()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_empty_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.json");
        let ledger = LocalMigrationLedger::new(&path);
        let applied = ledger.load().expect("load missing");
        assert!(!applied.contains("v1"), "empty ledger should not contain v1");
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sub").join("ledger.json");
        let ledger = LocalMigrationLedger::new(&path);

        let mut applied = AppliedMigrations::default();
        applied.record("v2");
        ledger.save(&applied).expect("save");

        let reloaded = LocalMigrationLedger::new(&path).load().expect("reload");
        assert!(reloaded.contains("v2"), "reloaded ledger must contain v2");
        assert!(!reloaded.contains("v1"), "reloaded ledger must not contain v1");
    }
}
