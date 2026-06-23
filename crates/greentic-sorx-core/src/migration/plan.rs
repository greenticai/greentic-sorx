//! SorX-local migration IR.
//!
//! # Justified fork of the sorla wire shape
//!
//! `greentic-sorx` deliberately does NOT depend on `greentic-sorla-ir` or
//! `greentic-sorla-lang`.  Migrations arrive in `.gtpack` files (as
//! `model_cbor` / executable-contract entries) using sorla's
//! `MigrationDecl` / `CompatibilityIr` wire shape (kebab-case JSON/CBOR).
//! This module defines a small local struct that deserialises that *same*
//! shape without pulling the sorla crates into the operator dependency tree.
//! Any future divergence from the canonical sorla wire shape must be
//! documented here.

use serde::{Deserialize, Serialize};

/// Mirrors sorla's `CompatibilityIr` enum (kebab-case wire names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompatibilityMode {
    Additive,
    BackwardCompatible,
    Breaking,
}

/// A single backfill directive within a migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationBackfill {
    pub record: String,
    pub field: String,
    pub default: serde_json::Value,
}

/// Mirrors sorla's `MigrationDecl` wire shape.
///
/// `#[serde(default)]` on vecs / options ensures partial JSON (fields absent
/// from older pack versions) still deserialises successfully.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompatibilityMigration {
    pub name: String,
    pub compatibility: CompatibilityMode,
    #[serde(default)]
    pub from_version: Option<String>,
    #[serde(default)]
    pub to_version: Option<String>,
    #[serde(default)]
    pub projection_updates: Vec<String>,
    #[serde(default)]
    pub backfills: Vec<MigrationBackfill>,
    #[serde(default)]
    pub idempotence_key: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl CompatibilityMigration {
    /// Returns the idempotence key if set, otherwise falls back to `name`.
    pub fn idempotence_id(&self) -> &str {
        self.idempotence_key.as_deref().unwrap_or(&self.name)
    }

    /// Returns `true` when applying this migration is a breaking change.
    pub fn is_breaking(&self) -> bool {
        self.compatibility == CompatibilityMode::Breaking
    }

    /// Returns `true` when the migration can be applied automatically
    /// (i.e. it is `Additive` or `BackwardCompatible`).
    pub fn auto_appliable(&self) -> bool {
        matches!(
            self.compatibility,
            CompatibilityMode::Additive | CompatibilityMode::BackwardCompatible
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CompatibilityMigration, CompatibilityMode};
    fn parse(j: &str) -> CompatibilityMigration {
        serde_json::from_str(j).expect("parse")
    }

    #[test]
    fn additive_auto_applies_and_not_breaking() {
        let m = parse(r#"{"name":"landlord-tenant-v2-fields","compatibility":"additive","idempotence_key":"landlord-tenant-v2-fields","backfills":[{"record":"Tenant","field":"date_of_birth","default":null}],"projection_updates":["ActiveTenants"]}"#);
        assert_eq!(m.compatibility, CompatibilityMode::Additive);
        assert!(!m.is_breaking());
        assert!(m.auto_appliable());
        assert_eq!(m.idempotence_id(), "landlord-tenant-v2-fields");
        assert_eq!(m.backfills.len(), 1);
    }
    #[test]
    fn breaking_not_auto_appliable_and_falls_back_to_name_for_id() {
        let m = parse(r#"{"name":"split","compatibility":"breaking","backfills":[],"projection_updates":[]}"#);
        assert!(m.is_breaking());
        assert!(!m.auto_appliable());
        assert_eq!(m.idempotence_id(), "split");
    }
    #[test]
    fn backward_compatible_auto_applies() {
        let m = parse(r#"{"name":"bc","compatibility":"backward-compatible","backfills":[],"projection_updates":[]}"#);
        assert!(m.auto_appliable());
        assert!(!m.is_breaking());
    }
}
