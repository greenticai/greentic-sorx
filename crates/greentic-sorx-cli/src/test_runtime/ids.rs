//! Identifier, slug and locale helpers used to build manager card names.

pub(super) fn role_card_id(role: &str, target: &str) -> String {
    route_card_id(&format!("roles/{role}/{target}"))
}

pub(super) fn route_card_id(target: &str) -> String {
    if target == "dashboard" {
        return "sorx_dashboard".to_string();
    }
    target
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn humanize(value: &str) -> String {
    let mut out = String::new();
    let mut previous_was_space = true;
    for ch in value.replace(['_', '-'], " ").chars() {
        if ch.is_whitespace() {
            if !previous_was_space {
                out.push(' ');
            }
            previous_was_space = true;
        } else if previous_was_space {
            out.extend(ch.to_uppercase());
            previous_was_space = false;
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

pub(super) fn default_collection_name(entity: &str) -> String {
    let mut chars = entity.chars();
    match chars.next() {
        Some(first) => format!("{}{}s", first.to_lowercase(), chars.as_str()),
        None => "records".to_string(),
    }
}

pub(super) fn locale_codes(locale: &str) -> Vec<String> {
    let mut codes = vec!["en".to_string(), "es".to_string()];
    if !codes.iter().any(|value| value == locale) {
        codes.push(locale.to_string());
    }
    if let Some(language) = locale.split('-').next()
        && !language.is_empty()
        && !codes.iter().any(|value| value == language)
    {
        codes.push(language.to_string());
    }
    codes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_card_id_maps_dashboard_to_the_well_known_card() {
        assert_eq!(route_card_id("dashboard"), "sorx_dashboard");
    }

    #[test]
    fn route_card_id_replaces_every_non_slug_character() {
        assert_eq!(route_card_id("records/tenant"), "records_tenant");
        assert_eq!(route_card_id("metrics/open?x=1"), "metrics_open_x_1");
        assert_eq!(
            route_card_id("keep_dash-and_underscore"),
            "keep_dash-and_underscore"
        );
    }

    #[test]
    fn route_card_id_only_special_cases_the_exact_dashboard_target() {
        assert_eq!(route_card_id("dashboards"), "dashboards");
        assert_eq!(route_card_id("sub/dashboard"), "sub_dashboard");
    }

    #[test]
    fn role_card_id_namespaces_the_target_under_the_role() {
        assert_eq!(role_card_id("admin", "dashboard"), "roles_admin_dashboard");
        assert_eq!(
            role_card_id("landlord", "records/tenant"),
            "roles_landlord_records_tenant"
        );
    }

    #[test]
    fn role_card_id_does_not_inherit_the_dashboard_special_case() {
        // The dashboard short-circuit only fires on a bare "dashboard" target,
        // and role_card_id always prefixes "roles/<role>/".
        assert_ne!(role_card_id("admin", "dashboard"), "sorx_dashboard");
    }

    #[test]
    fn sanitize_id_keeps_dots_underscores_and_dashes() {
        assert_eq!(
            sanitize_id("landlord.tenant_sor-1"),
            "landlord.tenant_sor-1"
        );
    }

    #[test]
    fn sanitize_id_replaces_everything_else_with_a_dash() {
        assert_eq!(sanitize_id("a b/c:d"), "a-b-c-d");
        assert_eq!(sanitize_id(""), "");
    }

    #[test]
    fn sanitize_id_and_route_card_id_use_different_filler_characters() {
        assert_eq!(sanitize_id("a/b"), "a-b");
        assert_eq!(route_card_id("a/b"), "a_b");
    }

    #[test]
    fn humanize_title_cases_words_split_on_dashes_and_underscores() {
        assert_eq!(humanize("landlord_tenant"), "Landlord Tenant");
        assert_eq!(humanize("landlord-tenant-sor"), "Landlord Tenant Sor");
    }

    #[test]
    fn humanize_collapses_runs_of_separators_and_trims() {
        assert_eq!(humanize("  a__b  "), "A B");
        assert_eq!(humanize("_-_"), "");
        assert_eq!(humanize(""), "");
    }

    #[test]
    fn humanize_preserves_interior_capitalisation() {
        assert_eq!(humanize("openAPI_spec"), "OpenAPI Spec");
    }

    #[test]
    fn default_collection_name_lowercases_the_first_character_and_pluralises() {
        assert_eq!(default_collection_name("Tenant"), "tenants");
        assert_eq!(default_collection_name("tenant"), "tenants");
        assert_eq!(default_collection_name("Property"), "propertys");
    }

    #[test]
    fn default_collection_name_falls_back_for_the_empty_entity() {
        assert_eq!(default_collection_name(""), "records");
    }

    #[test]
    fn locale_codes_always_offers_en_and_es() {
        assert_eq!(locale_codes("en"), vec!["en", "es"]);
        assert_eq!(locale_codes("es"), vec!["en", "es"]);
    }

    #[test]
    fn locale_codes_appends_an_unknown_locale() {
        assert_eq!(locale_codes("fr"), vec!["en", "es", "fr"]);
    }

    #[test]
    fn locale_codes_also_appends_the_bare_language_of_a_regional_locale() {
        assert_eq!(locale_codes("pt-BR"), vec!["en", "es", "pt-BR", "pt"]);
    }

    #[test]
    fn locale_codes_does_not_duplicate_the_language_when_it_is_already_present() {
        assert_eq!(locale_codes("es-ES"), vec!["en", "es", "es-ES"]);
        assert_eq!(locale_codes("en-GB"), vec!["en", "es", "en-GB"]);
    }
}
