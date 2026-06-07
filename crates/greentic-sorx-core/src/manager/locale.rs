use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ManagerViewModel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerLocaleContext {
    pub locale: String,
    pub fallback_locale: String,
    pub direction: TextDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

impl ManagerLocaleContext {
    pub fn new(locale: impl Into<String>, fallback_locale: impl Into<String>) -> Self {
        let locale = locale.into();
        Self {
            direction: TextDirection::for_locale(&locale),
            locale,
            fallback_locale: fallback_locale.into(),
            date_format: None,
            number_format: None,
            currency: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextDirection {
    Ltr,
    Rtl,
}

impl TextDirection {
    pub fn for_locale(locale: &str) -> Self {
        let language = locale
            .split(['-', '_'])
            .next()
            .unwrap_or(locale)
            .to_ascii_lowercase();
        match language.as_str() {
            "ar" | "fa" | "he" | "ur" => Self::Rtl,
            _ => Self::Ltr,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerLocaleCatalog {
    pub locale: String,
    #[serde(default)]
    pub messages: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerLocaleBundle {
    pub fallback_locale: String,
    #[serde(default)]
    pub catalogs: BTreeMap<String, ManagerLocaleCatalog>,
}

impl ManagerLocaleBundle {
    pub fn new(fallback_locale: impl Into<String>) -> Self {
        Self {
            fallback_locale: fallback_locale.into(),
            catalogs: BTreeMap::new(),
        }
    }

    pub fn with_catalog(mut self, catalog: ManagerLocaleCatalog) -> Self {
        self.catalogs.insert(catalog.locale.clone(), catalog);
        self
    }

    pub fn resolve(&self, locale: &str, key: &str, fallback_seed: &str) -> String {
        let language = locale.split(['-', '_']).next().unwrap_or(locale);
        self.catalogs
            .get(locale)
            .and_then(|catalog| catalog.messages.get(key))
            .or_else(|| {
                self.catalogs
                    .get(language)
                    .and_then(|catalog| catalog.messages.get(key))
            })
            .or_else(|| {
                self.catalogs
                    .get(&self.fallback_locale)
                    .and_then(|catalog| catalog.messages.get(key))
            })
            .cloned()
            .unwrap_or_else(|| humanize_identifier(fallback_seed))
    }
}

pub fn localize_manager_view(
    mut view: ManagerViewModel,
    locale: &ManagerLocaleContext,
    bundle: &ManagerLocaleBundle,
) -> ManagerViewModel {
    view.locale = locale.locale.clone();
    for item in &mut view.navigation {
        item.label = bundle.resolve(&locale.locale, &item.label_key, &item.collection);
    }
    for record in &mut view.records {
        record.label = bundle.resolve(&locale.locale, &record.label_key, &record.record);
        record.plural_label =
            bundle.resolve(&locale.locale, &record.plural_label_key, &record.collection);
        for field in &mut record.fields {
            field.label = bundle.resolve(&locale.locale, &field.label_key, &field.name);
        }
    }
    for relationship in &mut view.relationships {
        relationship.label =
            bundle.resolve(&locale.locale, &relationship.label_key, &relationship.id);
    }
    for action in &mut view.actions {
        action.label = bundle.resolve(&locale.locale, &action.label_key, &action.action_id);
    }
    view
}

pub fn humanize_identifier(value: &str) -> String {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lowercase = false;
    for ch in value.chars() {
        if ch == '_' || ch == '-' || ch == '.' || ch == '/' {
            push_word(&mut words, &mut current);
            previous_lowercase = false;
        } else if ch.is_ascii_uppercase() {
            if previous_lowercase {
                push_word(&mut words, &mut current);
            }
            current.push(ch);
            previous_lowercase = false;
        } else {
            current.push(ch);
            previous_lowercase = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    push_word(&mut words, &mut current);

    words
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn format_manager_value(value: &Value, locale: &ManagerLocaleContext) -> String {
    match value {
        Value::Bool(true) => localized_bool(true, locale),
        Value::Bool(false) => localized_bool(false, locale),
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn localized_bool(value: bool, locale: &ManagerLocaleContext) -> String {
    let language = locale
        .locale
        .split(['-', '_'])
        .next()
        .unwrap_or(locale.locale.as_str());
    match (language, value) {
        ("fr", true) => "Oui".to_string(),
        ("fr", false) => "Non".to_string(),
        ("es", true) => "Si".to_string(),
        ("es", false) => "No".to_string(),
        (_, true) => "Yes".to_string(),
        (_, false) => "No".to_string(),
    }
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{
        ManagerFieldView, ManagerNavItem, ManagerPolicyDecision, ManagerRecordView,
        ManagerViewModel,
    };

    #[test]
    fn humanizes_fallback_labels() {
        assert_eq!(humanize_identifier("RecordAlpha"), "Record Alpha");
        assert_eq!(humanize_identifier("created_at"), "Created At");
        assert_eq!(humanize_identifier("record_alpha"), "Record Alpha");
    }

    #[test]
    fn rtl_locale_sets_direction() {
        assert_eq!(
            ManagerLocaleContext::new("ar", "en").direction,
            TextDirection::Rtl
        );
        assert_eq!(
            ManagerLocaleContext::new("fr-FR", "en").direction,
            TextDirection::Ltr
        );
    }

    #[test]
    fn localizes_labels_with_fallback() {
        let bundle = ManagerLocaleBundle::new("en")
            .with_catalog(ManagerLocaleCatalog {
                locale: "en".to_string(),
                messages: BTreeMap::from([(
                    "record.record_alpha.label".to_string(),
                    "Record Alpha".to_string(),
                )]),
            })
            .with_catalog(ManagerLocaleCatalog {
                locale: "fr-FR".to_string(),
                messages: BTreeMap::from([(
                    "field.record_alpha.name.label".to_string(),
                    "Nom".to_string(),
                )]),
            });
        let view = localize_manager_view(
            sample_view(),
            &ManagerLocaleContext::new("fr-FR", "en"),
            &bundle,
        );

        assert_eq!(view.records[0].label, "Record Alpha");
        assert_eq!(view.records[0].fields[0].label, "Nom");
        assert_eq!(view.navigation[0].label, "Record Alpha");
    }

    fn sample_view() -> ManagerViewModel {
        ManagerViewModel {
            schema: "greentic.sorx.manager-view.v1".to_string(),
            tenant_id: "tenant-a".to_string(),
            sor_id: "generic-sor".to_string(),
            title: "Generic Sor".to_string(),
            description: "Manage Record Alpha.".to_string(),
            locale: "en".to_string(),
            navigation: vec![ManagerNavItem {
                record: "RecordAlpha".to_string(),
                label_key: "record.record_alpha.label".to_string(),
                label: String::new(),
                collection: "record_alpha".to_string(),
            }],
            records: vec![ManagerRecordView {
                record: "RecordAlpha".to_string(),
                collection: "record_alpha".to_string(),
                label_key: "record.record_alpha.label".to_string(),
                label: String::new(),
                plural_label_key: "record.record_alpha.plural".to_string(),
                plural_label: String::new(),
                create_field_names: Vec::new(),
                fields: vec![ManagerFieldView {
                    name: "name".to_string(),
                    label_key: "field.record_alpha.name.label".to_string(),
                    label: String::new(),
                    json_type: Some("string".to_string()),
                    rules: None,
                    generated: false,
                    relationship: None,
                    required: false,
                    read_only: false,
                    redacted: false,
                    value: None,
                    policy: ManagerPolicyDecision::allow(),
                }],
                endpoint_ids: Vec::new(),
                policy: ManagerPolicyDecision::allow(),
            }],
            relationships: Vec::new(),
            actions: Vec::new(),
            policies: Vec::new(),
        }
    }
}
