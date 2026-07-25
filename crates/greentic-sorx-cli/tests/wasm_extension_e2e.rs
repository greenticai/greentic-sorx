#![cfg(feature = "wasm-extensions-dev-unsigned")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use greentic_ext_runtime::{DiscoveryPaths, ExtensionRuntime, RuntimeConfig};
use greentic_sorx_cli::wasm_extensions::WasmExtensionRuntime;
use greentic_sorx_core::{
    ControlDecisionAction, ExtensionFailMode, RuntimeExtensionAdapter, RuntimeExtensionBinding,
};

const EXT_ID: &str = "greentic.sorx.e2e-guest";

/// Builds the `tests/fixtures/sorx-e2e-guest` guest (Task 1) via
/// `cargo component build` and returns the path to the produced
/// `wasm32-wasip2` component. Requires `cargo-component` on PATH.
fn guest_wasm() -> PathBuf {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/sorx-e2e-guest");
    let status = Command::new("cargo")
        .args([
            "component",
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
        ])
        .current_dir(&fixture)
        .status()
        .expect("cargo-component must be installed to run this opt-in e2e test");
    assert!(status.success(), "guest build failed");
    fixture.join("target/wasm32-wasip2/release/sorx_e2e_guest.wasm")
}

/// Minimal schema-valid v2 `describe.json` for the guest fixture. sha256
/// fields are placeholders — the dev-unsigned load path (gated by
/// `GREENTIC_EXT_ALLOW_UNSIGNED=1`) never checks the bytes against them.
fn describe_json() -> String {
    let zero_sha = "0".repeat(64);
    format!(
        r#"{{
      "apiVersion":"greentic.ai/v2","kind":"ProviderExtension",
      "compat":{{"min_designer_version":">=1.0.0","min_runner_version":"^0.12.0","contract_version":"1.2.0"}},
      "metadata":{{"id":"{EXT_ID}","name":"{EXT_ID}","version":"0.1.0","summary":"sorx e2e guest","author":{{"name":"test"}},"license":"MIT"}},
      "engine":{{"greenticDesigner":"*","extRuntime":"*"}},
      "capabilities":{{"offered":[],"required":[]}},
      "runtime":{{"permissions":{{"network":[],"secrets":[],"callExtensionKinds":[]}},
        "components":{{"sorx-guest":{{"gtpack":{{"file":"extension.wasm","sha256":"{zero_sha}","pack_id":"{EXT_ID}","component_version":"0.1.0"}},"sha256":"{zero_sha}","world":"greentic:extension-sorx/sorx-runtime-extension@0.1.0"}}}}}},
      "contributions":{{}}
    }}"#
    )
}

fn binding() -> RuntimeExtensionBinding {
    RuntimeExtensionBinding {
        id: "e2e".into(),
        contract: "greentic.cap.extension.control.v1".into(),
        pack_ref: EXT_ID.into(),
        fail_mode: ExtensionFailMode::Closed,
    }
}

#[test]
#[ignore = "requires cargo-component + the wasm32-wasip2 target; run explicitly with -- --ignored"]
fn guest_control_and_observe_execute_end_to_end() {
    let wasm = guest_wasm();
    let root = tempfile::TempDir::new().unwrap();
    let dir = root.path().join(EXT_ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&wasm, dir.join("extension.wasm")).unwrap();
    std::fs::write(dir.join("describe.json"), describe_json()).unwrap();

    // SAFETY: single-threaded test process; the dev-unsigned gate reads this env var.
    unsafe { std::env::set_var("GREENTIC_EXT_ALLOW_UNSIGNED", "1") };

    // `ExtensionRuntime::new` only stores the discovery paths — it does not
    // scan the directory during construction — so the extension must be
    // registered explicitly.
    let mut rt = ExtensionRuntime::new(RuntimeConfig::from_paths(DiscoveryPaths::new(
        root.path().to_path_buf(),
    )))
    .expect("construct extension runtime");
    rt.register_loaded_from_dir(&dir)
        .expect("load unsigned guest");
    let adapter = WasmExtensionRuntime::new(Arc::new(rt));

    // deny path — the guest returns a real deny decision:
    let denied = adapter
        .control(
            "pre_call",
            &binding(),
            &serde_json::json!({"deny": true}),
            None,
        )
        .expect("control dispatch");
    assert_eq!(denied.action, ControlDecisionAction::Deny);
    assert!(denied.reason.as_deref().unwrap_or("").contains("denied"));

    // allow path:
    let allowed = adapter
        .control("pre_call", &binding(), &serde_json::json!({}), None)
        .expect("control dispatch");
    assert_eq!(allowed.action, ControlDecisionAction::Allow);

    // observe runs (logs + Ok):
    adapter
        .observe(
            "post_call",
            &binding(),
            &serde_json::json!({"event_type": "stack.call.completed"}),
        )
        .expect("observe dispatch");
}
