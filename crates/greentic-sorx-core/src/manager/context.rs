use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CallerContext, SorxError, SorxResult, SorxRuntimeConfig};

use super::ManagerChannel;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SorxManagerContext {
    pub tenant_id: String,
    pub environment_id: Option<String>,
    pub sor_id: String,
    pub team_id: Option<String>,
    pub caller_id: String,
    pub channel: ManagerChannel,
    pub locale: String,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    #[serde(default)]
    pub claims: Value,
}

impl SorxManagerContext {
    pub fn caller(&self) -> CallerContext {
        CallerContext {
            subject: self.caller_id.clone(),
            roles: self.roles.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerContextDefaults {
    pub tenant_id: String,
    pub environment_id: Option<String>,
    pub sor_id: String,
    pub caller_id: String,
    pub locale: String,
    pub allow_local_defaults: bool,
}

impl ManagerContextDefaults {
    pub fn from_runtime_config(config: &SorxRuntimeConfig) -> Self {
        Self {
            tenant_id: config.tenant_id.clone(),
            environment_id: Some(config.environment.clone()),
            sor_id: config.deployment.sor_name.clone(),
            caller_id: "local".to_string(),
            locale: "en".to_string(),
            allow_local_defaults: config.environment == "local",
        }
    }
}

pub fn resolve_manager_context(
    headers: &BTreeMap<String, String>,
    defaults: &ManagerContextDefaults,
) -> SorxResult<SorxManagerContext> {
    let tenant_id = required_header_or_default(
        headers,
        &["x-greentic-tenant-id"],
        &defaults.tenant_id,
        defaults.allow_local_defaults,
    )?;
    let caller_id = required_header_or_default(
        headers,
        &["x-greentic-caller-id"],
        &defaults.caller_id,
        defaults.allow_local_defaults,
    )?;
    let roles = header(headers, "x-greentic-caller-role")
        .map(split_header_values)
        .filter(|roles| !roles.is_empty())
        .unwrap_or_else(|| vec!["local".to_string()]);
    let groups = header(headers, "x-greentic-caller-group")
        .map(split_header_values)
        .unwrap_or_default();
    let locale = manager_locale(headers).unwrap_or_else(|| defaults.locale.clone());
    let channel = header(headers, "x-greentic-channel")
        .map(|value| ManagerChannel::parse(&value))
        .unwrap_or_default();
    let sor_id = header(headers, "x-greentic-sor").unwrap_or_else(|| defaults.sor_id.clone());
    let team_id = header(headers, "x-greentic-team").filter(|value| !value.is_empty());

    Ok(SorxManagerContext {
        tenant_id,
        environment_id: defaults.environment_id.clone(),
        sor_id,
        team_id,
        caller_id,
        channel,
        locale,
        roles,
        groups,
        claims: Value::Object(Default::default()),
    })
}

fn required_header_or_default(
    headers: &BTreeMap<String, String>,
    names: &[&str],
    fallback: &str,
    allow_default: bool,
) -> SorxResult<String> {
    for name in names {
        if let Some(value) = header(headers, name)
            && !value.is_empty()
        {
            return Ok(value);
        }
    }
    if allow_default {
        Ok(fallback.to_string())
    } else {
        Err(SorxError::new(
            "context_missing",
            format!("missing required header `{}`", names[0]),
        ))
    }
}

fn header(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers
        .get(name)
        .or_else(|| headers.get(&name.to_ascii_lowercase()))
        .or_else(|| headers.get(&canonical_header_name(name)))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn canonical_header_name(name: &str) -> String {
    name.split('-')
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn split_header_values(value: String) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn manager_locale(headers: &BTreeMap<String, String>) -> Option<String> {
    for name in [
        "x-greentic-locale",
        "x-greentic-ui-locale",
        "accept-language",
    ] {
        if let Some(locale) = header(headers, name).and_then(primary_language) {
            return Some(locale);
        }
    }
    None
}

fn primary_language(value: String) -> Option<String> {
    value
        .split(',')
        .next()
        .and_then(|part| part.split(';').next())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(allow_local_defaults: bool) -> ManagerContextDefaults {
        ManagerContextDefaults {
            tenant_id: "tenant-a".to_string(),
            environment_id: Some("test".to_string()),
            sor_id: "demo-sor".to_string(),
            caller_id: "local".to_string(),
            locale: "en".to_string(),
            allow_local_defaults,
        }
    }

    #[test]
    fn resolves_existing_http_headers() {
        let headers = BTreeMap::from([
            ("x-greentic-tenant-id".to_string(), "tenant-b".to_string()),
            ("x-greentic-caller-id".to_string(), "actor-1".to_string()),
            (
                "x-greentic-caller-role".to_string(),
                "reader, approver".to_string(),
            ),
            ("x-greentic-team".to_string(), "team-alpha".to_string()),
            ("x-greentic-channel".to_string(), "teams".to_string()),
            ("accept-language".to_string(), "fr-FR,fr;q=0.8".to_string()),
        ]);
        let context = resolve_manager_context(&headers, &defaults(false)).unwrap();

        assert_eq!(context.tenant_id, "tenant-b");
        assert_eq!(context.caller_id, "actor-1");
        assert_eq!(context.roles, vec!["reader", "approver"]);
        assert_eq!(context.team_id.as_deref(), Some("team-alpha"));
        assert_eq!(context.channel, ManagerChannel::Teams);
        assert_eq!(context.locale, "fr-FR");
    }

    #[test]
    fn explicit_greentic_locale_takes_precedence_over_accept_language() {
        let headers = BTreeMap::from([
            ("x-greentic-tenant-id".to_string(), "tenant-b".to_string()),
            ("x-greentic-caller-id".to_string(), "actor-1".to_string()),
            ("x-greentic-locale".to_string(), "es-ES".to_string()),
            ("accept-language".to_string(), "fr-FR,fr;q=0.8".to_string()),
        ]);
        let context = resolve_manager_context(&headers, &defaults(false)).unwrap();

        assert_eq!(context.locale, "es-ES");
    }

    #[test]
    fn local_defaults_fill_required_headers() {
        let context = resolve_manager_context(&BTreeMap::new(), &defaults(true)).unwrap();
        assert_eq!(context.tenant_id, "tenant-a");
        assert_eq!(context.caller_id, "local");
        assert_eq!(context.roles, vec!["local"]);
        assert_eq!(context.sor_id, "demo-sor");
    }

    #[test]
    fn missing_required_header_fails_outside_local_defaults() {
        let err = resolve_manager_context(&BTreeMap::new(), &defaults(false)).unwrap_err();
        assert_eq!(err.code, "context_missing");
    }

    #[test]
    fn serializes_and_deserializes_context() {
        let context = resolve_manager_context(&BTreeMap::new(), &defaults(true)).unwrap();
        let json = serde_json::to_string(&context).unwrap();
        let decoded: SorxManagerContext = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, context);
    }
}
