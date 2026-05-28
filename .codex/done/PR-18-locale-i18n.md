# PR-03 — Add Locale and i18n Support for Manager Views

## Goal

Make Sorx Business Manager fully multi-language from the first version.

No generated manager card should require hardcoded English labels.

## Add locale model

Suggested location:

```text
crates/greentic-sorx-core/src/manager/
  locale.rs
```

Current codebase alignment:

- The repo already has CLI help i18n under `crates/greentic-sorx-cli/i18n/*.json`; these catalogs are command/help strings, not manager/business labels.
- Manager locale catalogs should be pack/runtime metadata, not global CLI assets. Prefer optional SorLa/SORX pack assets such as `assets/sorla/manager-i18n.json` or a manifest-referenced manager locale asset, and validate/load them through `greentic-sorx-pack` when this PR implements the feature.
- `Accept-Language` is not currently used by the HTTP runtime; PR-01's context resolver should supply the locale value before this PR adds label resolution.

### Types

```rust
pub struct ManagerLocaleContext {
    pub locale: String,
    pub fallback_locale: String,
    pub direction: TextDirection,
    pub date_format: Option<String>,
    pub number_format: Option<String>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextDirection {
    Ltr,
    Rtl,
}
```

## Label resolution

Every record, field, relationship, action, status, validation message, and policy message should support:

```text
label_key -> localized string -> fallback localized string -> humanized generated label
```

Example keys:

```text
record.record_alpha.label
record.record_alpha.plural
field.record_alpha.name.label
action.create_record.label
policy.field_redacted.message
```

## Humanization fallback

Examples:

```text
RecordAlpha -> Record Alpha
created_at -> Created at
record_alpha -> Record Alpha
```

## Locale-aware formatting

Add helper functions for:

- dates
- date-times
- decimals
- currency-like fields
- booleans/status labels

Initial implementation can be simple and deterministic; advanced ICU formatting can be a future PR.

## Acceptance criteria

- Manager view/card labels are resolved through locale layer.
- Missing translations do not break rendering.
- RTL locale produces direction metadata/hint where supported.
- CLI i18n assets are not repurposed as manager business catalogs.
- Tests cover at least English, French or Spanish, and one RTL locale using generic fixtures.

## Non-goals

- Do not add translation automation.
- Do not embed domain-specific language catalogs.
- Do not duplicate existing Greentic messaging provider translation features; SORX should emit localized card metadata/content.
