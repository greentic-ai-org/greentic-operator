use greentic_operator::{wizard, wizard_i18n, wizard_spec_builder};
use greentic_qa_lib::{I18nConfig, WizardDriver, WizardFrontend, WizardRunConfig};
use serde_json::Value;

#[test]
fn wizard_uses_requested_locale_for_question_titles() {
    let locale = "nl";
    let spec =
        wizard_spec_builder::build_validation_form_with_providers(wizard::WizardMode::Create, &[]);
    let resolved = wizard_i18n::load(locale).expect("load wizard i18n");
    let expected = resolved
        .get("wizard.create.bundle_name")
        .cloned()
        .expect("missing wizard.create.bundle_name in locale map");

    let mut driver = WizardDriver::new(WizardRunConfig {
        spec_json: spec.to_string(),
        initial_answers_json: Some(
            serde_json::json!({
                "bundle_path": "/tmp/example",
            })
            .to_string(),
        ),
        frontend: WizardFrontend::JsonUi,
        i18n: I18nConfig {
            locale: Some(locale.to_string()),
            resolved: Some(resolved),
            debug: false,
        },
        verbose: false,
        env_id: "local".into(),
    })
    .expect("create wizard driver");

    let ui_raw = driver.next_payload_json().expect("render ui");
    let ui: Value = serde_json::from_str(&ui_raw).expect("parse ui");
    let question_title = ui
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| {
            questions
                .iter()
                .find(|q| q.get("id").and_then(Value::as_str) == Some("bundle_name"))
        })
        .and_then(|q| q.get("title"))
        .and_then(Value::as_str)
        .expect("bundle_name title in rendered ui");

    assert_eq!(question_title, expected);
}

#[test]
fn wizard_uses_primary_language_from_region_locale_tag() {
    let locale = "de-DE";
    let spec =
        wizard_spec_builder::build_validation_form_with_providers(wizard::WizardMode::Create, &[]);
    let resolved = wizard_i18n::load(locale).expect("load wizard i18n");
    let expected = resolved
        .get("wizard.create.bundle_path")
        .cloned()
        .expect("missing wizard.create.bundle_path in locale map");

    let mut driver = WizardDriver::new(WizardRunConfig {
        spec_json: spec.to_string(),
        initial_answers_json: None,
        frontend: WizardFrontend::JsonUi,
        i18n: I18nConfig {
            locale: Some(locale.to_string()),
            resolved: Some(resolved),
            debug: false,
        },
        verbose: false,
        env_id: "local".into(),
    })
    .expect("create wizard driver");

    let ui_raw = driver.next_payload_json().expect("render ui");
    let ui: Value = serde_json::from_str(&ui_raw).expect("parse ui");
    let question_title = ui
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| {
            questions
                .iter()
                .find(|q| q.get("id").and_then(Value::as_str) == Some("bundle_path"))
        })
        .and_then(|q| q.get("title"))
        .and_then(Value::as_str)
        .expect("bundle_path title in rendered ui");

    assert_eq!(question_title, expected);
}
