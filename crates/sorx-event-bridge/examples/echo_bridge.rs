//! Live cross-process e2e helper: runs the REAL `run_bridge` as a standalone
//! process with an echo `SorxInvoker`. Used to prove the runner's NATS dispatch
//! interoperates with the production bridge code across process boundaries,
//! without standing up a full sorx `.gtpack` deployment.
//!
//! Run: `GREENTIC_EVENTS_NATS_URL=nats://127.0.0.1:4222 cargo run -p sorx-event-bridge --example echo_bridge`

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use sorx_event_bridge::{InvokeOutcome, SorxInvoker, run_bridge};

struct EchoInvoker;

#[async_trait]
impl SorxInvoker for EchoInvoker {
    async fn invoke(
        &self,
        tenant: &str,
        _env: &str,
        target: &str,
        operation: &str,
        input: Value,
        _idempotency_key: Option<&str>,
    ) -> anyhow::Result<InvokeOutcome> {
        eprintln!(
            "[echo_bridge] invoke tenant={tenant} target={target} operation={operation} input={input}"
        );
        Ok(InvokeOutcome {
            ok: true,
            output: json!({ "echoed": input, "target": target, "operation": operation }),
            events: vec![],
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("GREENTIC_EVENTS_NATS_URL")
        .unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let client = async_nats::connect(&url).await?;
    eprintln!("[echo_bridge] connected to {url}; subscribing greentic.sorla.request.v1");
    run_bridge(client, Arc::new(EchoInvoker)).await
}
