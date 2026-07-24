//! Durable JSON-file ledger that persists `AppliedMigrations` across process
//! restarts. Mirrors the design of `LocalDeploymentRegistryStore` in
//! `deployment.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{SorxError, SorxResult};
use crate::migration::orchestrate::MigrationLedger;
use crate::migration::runner::AppliedMigrations;
use crate::provider::ProviderNamespace;

/// JSON-file-backed, namespace-scoped store for the applied-migration ledger.
///
/// The on-disk file holds a per-namespace map (see [`NamespacedLedger`]), so a
/// single shared file keeps tenants from colliding. The [`MigrationLedger`]
/// trait impl is the only access path. A missing file is treated as an empty
/// ledger — no error is raised; recording creates any missing parent
/// directories automatically.
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
}

/// On-disk shape for the namespace-scoped [`MigrationLedger`] impl: a map from
/// each namespace's [`key_prefix`][ProviderNamespace::key_prefix] to its
/// applied-migration set. A single shared file therefore keeps tenants from
/// colliding — `tenant-a` and `tenant-b` get independent entries.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct NamespacedLedger {
    by_namespace: BTreeMap<String, AppliedMigrations>,
}

impl LocalMigrationLedger {
    /// Read the namespaced map from disk (empty map when the file is absent).
    fn load_namespaced(&self) -> SorxResult<NamespacedLedger> {
        if !self.path.exists() {
            return Ok(NamespacedLedger::default());
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

    /// Write the namespaced map to disk, creating parent directories.
    fn save_namespaced(&self, ledger: &NamespacedLedger) -> SorxResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                SorxError::new(
                    "migration_ledger_io",
                    format!("failed to create {}: {err}", parent.display()),
                )
            })?;
        }
        let encoded = serde_json::to_vec_pretty(ledger).map_err(|err| {
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

impl MigrationLedger for LocalMigrationLedger {
    fn load(&self, namespace: &ProviderNamespace) -> SorxResult<AppliedMigrations> {
        let mut ledger = self.load_namespaced()?;
        Ok(ledger
            .by_namespace
            .remove(&namespace.key_prefix())
            .unwrap_or_default())
    }

    fn record_applied(&self, namespace: &ProviderNamespace, migration_id: &str) -> SorxResult<()> {
        let mut ledger = self.load_namespaced()?;
        ledger
            .by_namespace
            .entry(namespace.key_prefix())
            .or_default()
            .record(migration_id);
        self.save_namespaced(&ledger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(tenant: &str) -> ProviderNamespace {
        ProviderNamespace {
            tenant_id: tenant.into(),
            sor_name: "tenancy".into(),
        }
    }

    #[test]
    fn missing_file_returns_empty_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let applied = ledger.load(&ns("acme")).expect("load missing");
        assert!(
            !applied.contains("v1"),
            "empty ledger should not contain v1"
        );
    }

    #[test]
    fn record_and_reload_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sub").join("ledger.json");
        let ledger = LocalMigrationLedger::new(&path);

        ledger.record_applied(&ns("acme"), "v2").expect("record");

        let reloaded = LocalMigrationLedger::new(&path)
            .load(&ns("acme"))
            .expect("reload");
        assert!(reloaded.contains("v2"), "reloaded ledger must contain v2");
        assert!(
            !reloaded.contains("v1"),
            "reloaded ledger must not contain v1"
        );
    }

    #[test]
    fn namespaces_do_not_collide_in_shared_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.json");
        let ledger = LocalMigrationLedger::new(&path);

        ledger.record_applied(&ns("tenant-a"), "v2").expect("a");
        ledger.record_applied(&ns("tenant-b"), "v3").expect("b");

        let a = ledger.load(&ns("tenant-a")).expect("load a");
        let b = ledger.load(&ns("tenant-b")).expect("load b");
        assert!(a.contains("v2") && !a.contains("v3"));
        assert!(b.contains("v3") && !b.contains("v2"));
    }
}
