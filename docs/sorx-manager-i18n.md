# Sorx Manager I18n

Manager labels resolve through manager locale catalogs and deterministic fallbacks. CLI help i18n files are not business/manager translation catalogs.

Resolution order:

```text
label_key -> requested locale -> fallback locale -> humanized generated label
```

Examples:

```text
record.record_alpha.label
record.record_alpha.plural
field.record_alpha.name.label
action.record_alpha.create.label
policy.field_redacted.message
```

Missing labels do not break manager view or card rendering. Humanized fallbacks turn values such as `RecordAlpha`, `created_at`, and `record_alpha` into readable labels.

RTL languages such as Arabic produce `TextDirection::Rtl` in `ManagerLocaleContext`; cards include localized text, while channel-specific RTL transformation remains the messaging provider's responsibility.
