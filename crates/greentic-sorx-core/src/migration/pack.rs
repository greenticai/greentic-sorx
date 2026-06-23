//! Helpers for extracting migration data from pack assets.
//!
//! `parse_pack_migrations` reads the `"migrations"` array from an
//! `assets/sorla/executable-contract.json` value and deserialises each element
//! into a [`CompatibilityMigration`].  The raw JSON is supplied by the pack
//! loader (`greentic-sorx-pack`) which exposes it without depending on this
//! crate.

use serde_json::Value;

use crate::SorxError;

use super::plan::CompatibilityMigration;

/// Extract migrations from an executable-contract JSON value
/// (`assets/sorla/executable-contract.json`).
///
/// - Returns `Ok(vec![])` when the `"migrations"` field is absent or `null`.
/// - Returns `Err` when `"migrations"` is present but is not a JSON array.
/// - Returns `Err` when any array element cannot be deserialised into a
///   [`CompatibilityMigration`] — no entry is silently dropped.
pub fn parse_pack_migrations(
    contract: &Value,
) -> Result<Vec<CompatibilityMigration>, SorxError> {
    let Some(migrations_val) = contract.get("migrations") else {
        return Ok(Vec::new());
    };
    if migrations_val.is_null() {
        return Ok(Vec::new());
    }
    let array = migrations_val.as_array().ok_or_else(|| {
        SorxError::new(
            "migration_parse_failed",
            "executable-contract `migrations` field is not a JSON array",
        )
    })?;

    let mut result = Vec::with_capacity(array.len());
    for (index, elem) in array.iter().enumerate() {
        let migration =
            serde_json::from_value::<CompatibilityMigration>(elem.clone()).map_err(|err| {
                SorxError::new(
                    "migration_parse_failed",
                    format!(
                        "executable-contract migrations[{index}] is not a valid \
                         CompatibilityMigration: {err}"
                    ),
                )
            })?;
        result.push(migration);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landlord_v2_migration_json() -> &'static str {
        include_str!("../../tests/fixtures/landlord_v2_migration.json")
    }

    /// Build a minimal executable-contract wrapping the landlord fixture migration.
    fn landlord_contract() -> Value {
        let migration: Value =
            serde_json::from_str(landlord_v2_migration_json()).expect("fixture parse");
        serde_json::json!({
            "schema": "greentic.sorla.executable-contract.v1",
            "relationships": [],
            "migrations": [migration]
        })
    }

    #[test]
    fn parses_landlord_contract_one_migration_twelve_backfills() {
        let contract = landlord_contract();
        let migrations = parse_pack_migrations(&contract)
            .expect("should parse without error");
        assert_eq!(migrations.len(), 1, "expected exactly one migration");
        let m = &migrations[0];
        assert_eq!(m.idempotence_id(), "landlord-tenant-v2-fields");
        assert_eq!(
            m.backfills.len(),
            12,
            "expected 12 backfills, got {}",
            m.backfills.len()
        );
    }

    #[test]
    fn absent_migrations_field_returns_empty_vec() {
        let contract = serde_json::json!({
            "schema": "greentic.sorla.executable-contract.v1",
            "relationships": []
        });
        let result = parse_pack_migrations(&contract).expect("should return Ok");
        assert!(result.is_empty(), "expected empty vec for absent migrations field");
    }

    #[test]
    fn null_migrations_field_returns_empty_vec() {
        let contract = serde_json::json!({ "migrations": null });
        let result = parse_pack_migrations(&contract).expect("should return Ok");
        assert!(result.is_empty());
    }

    #[test]
    fn malformed_entry_missing_compatibility_returns_err() {
        let contract = serde_json::json!({
            "migrations": [{ "name": "x" }]
        });
        let err = parse_pack_migrations(&contract)
            .expect_err("should fail for entry missing `compatibility`");
        assert_eq!(err.code, "migration_parse_failed");
        assert!(
            err.message.contains("migrations[0]"),
            "error message should reference index: {}",
            err.message
        );
    }

    #[test]
    fn migrations_not_array_returns_err() {
        let contract = serde_json::json!({ "migrations": "not-an-array" });
        let err = parse_pack_migrations(&contract)
            .expect_err("should fail when migrations is not an array");
        assert_eq!(err.code, "migration_parse_failed");
    }
}
