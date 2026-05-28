# PR-01 — Add Sorx Business Manager Context Model

## Goal

Introduce the core context model required for Sorx Business Manager to render different business-admin cards for different tenants, teams, users, channels, and locales.

This PR should not implement the full manager UI. It should establish the stable context and capability types that later PRs build on.

## Why

Sorx Business Manager must generate runtime business interfaces from SorLa/SORX metadata. The same system of record may be accessed by different teams with different permissions and languages. Card generation must therefore start from context.

## Add core types

Suggested location:

```text
crates/greentic-sorx-core/src/manager/
  mod.rs
  context.rs
  channel.rs
```

Current codebase alignment:

- `crates/greentic-sorx-core/src/model.rs` already has `CallerContext { subject, roles }` and `EndpointInvocation { tenant_id, caller, source }`; the manager context should wrap/convert from these rather than replacing them.
- `crates/greentic-sorx-core/src/startup.rs` already exposes `SorxRuntimeConfig` with `tenant_id`, `environment`, and deployment `sor_name`; `sor_id` should be derived from `config.deployment.sor_name` unless pack metadata later provides a stronger canonical id.
- The local HTTP adapter currently lives in `crates/greentic-sorx-cli/src/http_runtime.rs`, not `greentic-sorx-core`; keep pure context types in core and add HTTP header extraction either as a core helper over a generic header map or in the CLI adapter.

### `SorxManagerContext`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    pub claims: serde_json::Value,
}
```

### `ManagerChannel`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagerChannel {
    WebChat,
    Teams,
    Slack,
    Webex,
    Web,
    Api,
    Unknown(String),
}
```

### `ChannelCapabilities`

Keep this lightweight because card transformations are handled by Greentic messaging providers.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelCapabilities {
    pub canonical_adaptive_cards: bool,
    pub supports_submit: bool,
    pub supports_refresh: bool,
    pub supports_dynamic_choices: bool,
    pub supports_svg_image: bool,
    pub supports_rtl_hint: bool,
    pub max_card_size_bytes: Option<usize>,
    pub max_actions: Option<usize>,
}
```

## Context resolution

Add a resolver that can build context from HTTP headers/session/auth claims.

Initial header support should preserve current HTTP runtime conventions and add manager-specific aliases only where needed:

```text
Authorization: Bearer ...
X-Greentic-Tenant-Id: <tenant-id>        # existing required header outside local mode
X-Greentic-Caller-Id: <user-or-actor-id> # existing required header outside local mode
X-Greentic-Caller-Role: role-a,role-b    # existing comma-separated role header
X-Greentic-Team: <team-id>
X-Greentic-Channel: webchat|teams|slack|webex|web|api
X-Greentic-Sor: <sor-id>                 # optional; default from runtime deployment.sor_name
Accept-Language: <locale>
```

Do not rename the existing tenant/caller headers to `X-Greentic-Tenant` or a separate user-only concept. If friendlier manager aliases are added, they must be backward-compatible with `X-Greentic-Tenant-Id`, `X-Greentic-Caller-Id`, and `X-Greentic-Caller-Role`.

The resolver should produce a safe error when required context is missing.

## Non-goals

- No policy engine yet.
- No card rendering yet.
- No Slack/Webex renderer work. Messaging providers already transform Adaptive Cards.
- No domain-specific manager behavior.

## Acceptance criteria

- New `manager` module compiles.
- `SorxManagerContext` serializes/deserializes cleanly.
- Resolver preserves current HTTP header behavior for existing `/v1/agent/...` routes.
- Header/context resolver has tests.
- Unknown channels degrade safely.
- No existing SORX runtime behavior changes.
