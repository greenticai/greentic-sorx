use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{RiskLevel, SorxError, SorxResult};

pub trait AuditSink: Send + Sync {
    fn emit(&self, event: SorxAuditEvent) -> SorxResult<()>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SorxAuditEvent {
    pub event: String,
    pub pack: String,
    pub version: String,
    pub tenant_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub risk: RiskLevel,
    pub caller_id: String,
    pub decision: Option<String>,
    pub duration_ms: Option<u64>,
    pub idempotency_key_present: bool,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

#[derive(Debug, Default)]
pub struct DisabledAuditSink;

impl AuditSink for DisabledAuditSink {
    fn emit(&self, _event: SorxAuditEvent) -> SorxResult<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct StdoutAuditSink;

impl AuditSink for StdoutAuditSink {
    fn emit(&self, event: SorxAuditEvent) -> SorxResult<()> {
        let encoded = serde_json::to_string(&event)
            .map_err(|err| SorxError::new("audit_encode_failed", err.to_string()))?;
        println!("{encoded}");
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryAuditSink {
    events: Arc<Mutex<Vec<SorxAuditEvent>>>,
}

impl MemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> SorxResult<Vec<SorxAuditEvent>> {
        self.events
            .lock()
            .map_err(|_| SorxError::new("audit_lock_failed", "audit sink lock was poisoned"))
            .map(|events| events.clone())
    }
}

impl AuditSink for MemoryAuditSink {
    fn emit(&self, event: SorxAuditEvent) -> SorxResult<()> {
        self.events
            .lock()
            .map_err(|_| SorxError::new("audit_lock_failed", "audit sink lock was poisoned"))?
            .push(event);
        Ok(())
    }
}
