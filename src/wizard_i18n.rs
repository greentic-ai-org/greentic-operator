use anyhow::Context;
use greentic_qa_lib::ResolvedI18nMap;
use include_dir::{Dir, include_dir};

use crate::operator_i18n::normalize_locale;

static WIZARD_I18N: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/i18n/operator_wizard");

pub fn load(locale: &str) -> anyhow::Result<ResolvedI18nMap> {
    for candidate in locale_candidates(locale) {
        if let Some(file) = WIZARD_I18N.get_file(&candidate) {
            let raw = file.contents_utf8().ok_or_else(|| {
                anyhow::anyhow!("wizard i18n file is not valid UTF-8: {candidate}")
            })?;
            return serde_json::from_str(raw)
                .with_context(|| format!("parse embedded wizard i18n map {candidate}"));
        }
    }
    Ok(ResolvedI18nMap::new())
}

fn locale_candidates(locale: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push_candidate = |candidate: String| {
        if !out.iter().any(|existing| existing == &candidate) {
            out.push(candidate);
        }
    };

    let trimmed = locale.trim();
    if !trimmed.is_empty() {
        push_candidate(format!("{}.json", trimmed));

        let canonical = canonical_locale_tag(trimmed);
        if canonical != trimmed {
            push_candidate(format!("{}.json", canonical));
        }

        let primary = normalize_locale(trimmed);
        push_candidate(format!("{}.json", primary));
    }
    push_candidate("en-GB.json".to_string());
    push_candidate("en.json".to_string());
    out
}

fn canonical_locale_tag(raw: &str) -> String {
    let without_encoding = raw.split('.').next().unwrap_or(raw);
    let without_modifier = without_encoding
        .split('@')
        .next()
        .unwrap_or(without_encoding);
    let normalized = without_modifier.replace('_', "-");
    let mut parts = normalized.split('-');
    let language = parts.next().unwrap_or("en").to_ascii_lowercase();
    let mut out = language;
    if let Some(region) = parts.next()
        && !region.is_empty()
    {
        out.push('-');
        if region.len() == 2 {
            out.push_str(&region.to_ascii_uppercase());
        } else {
            out.push_str(region);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_requested_locale_when_present() {
        let map = load("de").expect("load de");
        assert_eq!(
            map.get("wizard.create.bundle_name").map(String::as_str),
            Some("Bundle-Name")
        );
    }

    #[test]
    fn falls_back_to_primary_language() {
        let map = load("de-DE").expect("load de-DE");
        assert_eq!(
            map.get("wizard.create.execution_mode").map(String::as_str),
            Some("Ausführungsmodus")
        );
    }

    #[test]
    fn prefers_requested_locale_before_english_fallbacks() {
        let map = load("nl").expect("load nl");
        assert_eq!(
            map.get("wizard.create.bundle_name").map(String::as_str),
            Some("Bundelnaam")
        );
    }

    #[test]
    fn falls_back_to_english_for_unknown_locale() {
        let map = load("zz-ZZ").expect("load fallback");
        assert_eq!(
            map.get("wizard.create.bundle_path").map(String::as_str),
            Some("Bundle output path")
        );
    }
}
