use std::sync::Arc;

use greentic_ext_runtime::ExtensionRuntime;
use greentic_sorx_core::{
    ControlDecision, RuntimeExtensionAdapter, RuntimeExtensionBinding, SorxError, SorxResult,
};
use serde_json::Value;

/// Runs SoRX control/observe hooks against signed WASM extension packs loaded by
/// `greentic-ext-runtime`. The binding's `pack_ref` is the extension id.
pub struct WasmExtensionRuntime {
    runtime: Arc<ExtensionRuntime>,
}

impl std::fmt::Debug for WasmExtensionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmExtensionRuntime")
            .finish_non_exhaustive()
    }
}

impl WasmExtensionRuntime {
    pub fn new(runtime: Arc<ExtensionRuntime>) -> Self {
        Self { runtime }
    }
}

impl RuntimeExtensionAdapter for WasmExtensionRuntime {
    fn control(
        &self,
        hook: &str,
        binding: &RuntimeExtensionBinding,
        request: &Value,
        response: Option<&Value>,
    ) -> SorxResult<ControlDecision> {
        let binding_json = serde_json::to_string(binding)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        let request_json = serde_json::to_string(request)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        let response_json = match response {
            Some(value) => Some(
                serde_json::to_string(value)
                    .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?,
            ),
            None => None,
        };
        let out = self
            .runtime
            .control(
                &binding.pack_ref,
                hook,
                &binding_json,
                &request_json,
                response_json.as_deref(),
            )
            .map_err(|e| SorxError::new("wasm_extension_control_failed", e.to_string()))?;
        serde_json::from_str::<ControlDecision>(&out)
            .map_err(|e| SorxError::new("wasm_extension_decision_invalid", e.to_string()))
    }

    fn observe(
        &self,
        subscription: &str,
        binding: &RuntimeExtensionBinding,
        event: &Value,
    ) -> SorxResult<()> {
        let binding_json = serde_json::to_string(binding)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        let event_json = serde_json::to_string(event)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        self.runtime
            .observe(&binding.pack_ref, subscription, &binding_json, &event_json)
            .map_err(|e| SorxError::new("wasm_extension_observe_failed", e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_ext_runtime::{DiscoveryPaths, ExtensionRuntime, RuntimeConfig};
    use greentic_sorx_core::{ExtensionFailMode, RuntimeExtensionAdapter, RuntimeExtensionBinding};
    use serde_json::json;
    use std::sync::Arc;

    fn binding(pack_ref: &str) -> RuntimeExtensionBinding {
        RuntimeExtensionBinding {
            id: "x".into(),
            contract: "greentic.cap.extension.control.v1".into(),
            pack_ref: pack_ref.into(),
            fail_mode: ExtensionFailMode::Closed,
        }
    }

    fn test_runtime() -> ExtensionRuntime {
        // `ExtensionRuntime::new` only stores the discovery paths (it does not
        // scan the directory during construction), so the tempdir can be
        // dropped as soon as this function returns.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let config = RuntimeConfig::from_paths(DiscoveryPaths::new(tmp.path().to_path_buf()));
        ExtensionRuntime::new(config).expect("construct empty test runtime")
    }

    #[test]
    fn control_on_unloaded_extension_errors() {
        let rt = Arc::new(test_runtime());
        let adapter = WasmExtensionRuntime::new(rt);
        let err = adapter
            .control("pre_call", &binding("does.not.exist"), &json!({}), None)
            .unwrap_err();
        // maps ext-runtime NotFound to a SorxError (does not panic, does not silently allow)
        assert!(err.to_string().to_lowercase().contains("not") || !err.code.is_empty());
    }

    #[test]
    fn observe_on_unloaded_extension_errors() {
        let rt = Arc::new(test_runtime());
        let adapter = WasmExtensionRuntime::new(rt);
        assert!(
            adapter
                .observe("post_call", &binding("does.not.exist"), &json!({}))
                .is_err()
        );
    }
}
