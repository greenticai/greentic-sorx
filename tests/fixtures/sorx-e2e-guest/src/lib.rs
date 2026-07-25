#[allow(warnings)]
mod bindings;

use bindings::exports::greentic::extension_sorx::{control, observe};
use bindings::greentic::extension_host::logging;

struct Component;

impl control::Guest for Component {
    fn control(
        _hook: String,
        _binding_json: String,
        request_json: String,
        _response_json: Option<String>,
    ) -> Result<String, String> {
        let deny = serde_json::from_str::<serde_json::Value>(&request_json)
            .ok()
            .and_then(|v| v.get("deny").and_then(|d| d.as_bool()))
            .unwrap_or(false);
        if deny {
            Ok(r#"{"action":"deny","reason":"e2e guest denied"}"#.to_string())
        } else {
            Ok(r#"{"action":"allow"}"#.to_string())
        }
    }
}

impl observe::Guest for Component {
    fn observe(subscription: String, _binding_json: String, _event_json: String) -> Result<(), String> {
        logging::log(logging::Level::Info, "sorx-e2e-guest", &format!("observed {subscription}"));
        Ok(())
    }
}

bindings::export!(Component with_types_in bindings);
