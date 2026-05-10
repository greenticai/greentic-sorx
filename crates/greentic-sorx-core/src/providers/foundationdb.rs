use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CreateOp, DeleteOp, DeleteResult, EntityRecord, GetOp, QueryOp, QueryResult, SorStoreProvider,
    SorxError, SorxResult, UpdateOp,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundationDbProviderConfig {
    pub cluster_file: Option<String>,
    pub database: Option<String>,
    pub config_ref: Option<String>,
}

impl FoundationDbProviderConfig {
    pub fn from_parts(config_ref: Option<String>, config: Option<Value>) -> Self {
        let object = config.and_then(|value| value.as_object().cloned());
        Self {
            cluster_file: object
                .as_ref()
                .and_then(|value| string_field(value, "cluster_file")),
            database: object
                .as_ref()
                .and_then(|value| string_field(value, "database")),
            config_ref,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FoundationDbProviderAdapter {
    config: FoundationDbProviderConfig,
}

impl FoundationDbProviderAdapter {
    pub fn unavailable(config: FoundationDbProviderConfig) -> Self {
        Self { config }
    }

    fn unavailable_error(&self) -> SorxError {
        let config_hint = self
            .config
            .config_ref
            .as_deref()
            .unwrap_or("direct local/test config");
        SorxError::new(
            "provider_unavailable",
            format!(
                "FoundationDB provider adapter is not wired to a SORX store provider yet; config boundary `{config_hint}` was accepted"
            ),
        )
    }
}

impl SorStoreProvider for FoundationDbProviderAdapter {
    fn create(&self, _op: CreateOp) -> SorxResult<EntityRecord> {
        Err(self.unavailable_error())
    }

    fn get(&self, _op: GetOp) -> SorxResult<Option<EntityRecord>> {
        Err(self.unavailable_error())
    }

    fn update(&self, _op: UpdateOp) -> SorxResult<EntityRecord> {
        Err(self.unavailable_error())
    }

    fn query(&self, _op: QueryOp) -> SorxResult<QueryResult> {
        Err(self.unavailable_error())
    }

    fn delete(&self, _op: DeleteOp) -> SorxResult<DeleteResult> {
        Err(self.unavailable_error())
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToString::to_string)
}
