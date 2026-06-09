//! Persist config and secrets from QA apply-answers output.
//!
//! After a provider's `apply-answers` op returns a config object, this module:
//! - Writes every visible answer to the dev secrets store under
//!   `secrets://<env>/<tenant>/<team>/<provider>/<key>` (legacy universal
//!   write — WASM components have historically read both secret and
//!   non-secret config values through the secrets API).
//! - Emits a sibling `pack-config-input.v1` file via
//!   [`emit_pack_config_input`] when the wizard scope is known. This is the
//!   C7 producer for the `pack-config.v1.non_secret` channel: the
//!   greentic-deployer picks up the file at revision-create, stamps the
//!   active `revision_id`, and writes the final `pack-config.v1` consumed by
//!   the runtime (C4). The universal DevStore write stays alive for one
//!   release as the C4.2 compatibility shim.
//! - Writes remaining (non-secret) fields to the provider config envelope.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use greentic_secrets_lib::{
    ApplyOptions, DevStore, SecretFormat, SeedDoc, SeedEntry, SeedValue, apply_seed,
};
use qa_spec::{FormSpec, VisibilityMode, resolve_visibility};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};

use crate::secrets_gate::canonical_secret_uri;
use crate::secrets_setup::resolve_env;

/// Extract secret fields from the QA config output and write them to the dev store.
///
/// Returns a list of secret keys that were persisted.
pub async fn persist_qa_secrets(
    store: &DevStore,
    env: &str,
    tenant: &str,
    team: Option<&str>,
    provider_id: &str,
    config: &Value,
    form_spec: &FormSpec,
) -> Result<Vec<String>> {
    // Collect visible question IDs — WASM components read both secret and
    // non-secret config values via the secrets API, so we must persist
    // everything that is currently visible (skip conditionally-invisible
    // questions to avoid leaking stale answers).
    let visibility = resolve_visibility(form_spec, config, VisibilityMode::Visible);
    let visible_question_ids: Vec<&str> = form_spec
        .questions
        .iter()
        .filter(|q| visibility.get(&q.id).copied().unwrap_or(true))
        .map(|q| q.id.as_str())
        .collect();

    if visible_question_ids.is_empty() {
        return Ok(vec![]);
    }

    let Some(config_map) = config.as_object() else {
        return Ok(vec![]);
    };

    let mut entries = Vec::new();
    let mut saved_keys = Vec::new();

    for &key in &visible_question_ids {
        if let Some(value) = config_map.get(key) {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if text.is_empty() || text == "null" {
                continue;
            }
            let uri = canonical_secret_uri(env, tenant, team, provider_id, key);
            entries.push(SeedEntry {
                uri,
                format: SecretFormat::Text,
                value: SeedValue::Text { text },
                description: Some(format!("from QA setup for {provider_id}")),
            });
            saved_keys.push(key.to_string());
        }
    }

    if entries.is_empty() {
        return Ok(vec![]);
    }

    let report = apply_seed(store, &SeedDoc { entries }, ApplyOptions::default()).await;

    if !report.failed.is_empty() {
        return Err(anyhow::anyhow!(
            "failed to persist {} secret(s): {:?}",
            report.failed.len(),
            report.failed
        ));
    }

    Ok(saved_keys)
}

/// Write non-secret config fields to the provider config envelope.
///
/// Filters out secret fields (identified by the FormSpec) before writing.
pub fn persist_qa_config(
    providers_root: &Path,
    provider_id: &str,
    config: &Value,
    pack_path: &Path,
    form_spec: &FormSpec,
    backup: bool,
) -> Result<()> {
    let secret_ids: Vec<&str> = form_spec
        .questions
        .iter()
        .filter(|q| q.secret)
        .map(|q| q.id.as_str())
        .collect();

    let filtered_config = if secret_ids.is_empty() {
        config.clone()
    } else {
        filter_secrets(config, &secret_ids)
    };

    crate::provider_config_envelope::write_provider_config_envelope(
        providers_root,
        provider_id,
        "qa-setup",
        &filtered_config,
        pack_path,
        backup,
    )?;

    Ok(())
}

/// Remove secret fields from a config object.
fn filter_secrets(config: &Value, secret_ids: &[&str]) -> Value {
    let Some(map) = config.as_object() else {
        return config.clone();
    };
    let filtered: JsonMap<String, Value> = map
        .iter()
        .filter(|(key, _)| !secret_ids.contains(&key.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Value::Object(filtered)
}

/// Persist all config values as secrets without requiring a FormSpec.
///
/// This is used by `demo start --setup-input` where the QA form spec may not
/// be available but WASM components still read config values via the secrets API.
///
/// Also reads the pack's `secret-requirements.json` (if a `pack_path` is
/// provided) and seeds aliases so that WASM components that look up secrets by
/// their canonical requirement key (e.g. `WEBEX_BOT_TOKEN` → `webex_bot_token`)
/// can find the value even though the answers file uses a shorter key
/// (e.g. `bot_token`).
pub async fn persist_all_config_as_secrets(
    bundle_root: &Path,
    env: &str,
    tenant: &str,
    team: Option<&str>,
    provider_id: &str,
    config: &Value,
    pack_path: Option<&Path>,
) -> Result<Vec<String>> {
    let Some(config_map) = config.as_object() else {
        return Ok(vec![]);
    };
    if config_map.is_empty() {
        return Ok(vec![]);
    }

    let store_path = crate::dev_store_path::ensure_path(bundle_root)?;
    let store = DevStore::with_path(&store_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to open dev secrets store {}: {err}",
            store_path.display()
        )
    })?;

    let mut entries = Vec::new();
    let mut saved_keys = Vec::new();

    for (key, value) in config_map {
        let text = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if text.is_empty() || text == "null" {
            continue;
        }
        let uri = canonical_secret_uri(env, tenant, team, provider_id, key);
        entries.push(SeedEntry {
            uri,
            format: SecretFormat::Text,
            value: SeedValue::Text { text },
            description: Some(format!("from setup-input for {provider_id}")),
        });
        saved_keys.push(key.to_string());
    }

    // Seed aliases from secret-requirements.json so WASM components can find
    // secrets by their canonical requirement key (e.g. WEBEX_BOT_TOKEN →
    // webex_bot_token) even when the answers file uses a shorter key (bot_token).
    if let Some(pp) = pack_path {
        seed_secret_requirement_aliases(
            &mut entries,
            config_map,
            env,
            tenant,
            team,
            provider_id,
            pp,
        );
    }

    if entries.is_empty() {
        return Ok(vec![]);
    }

    let report = apply_seed(&store, &SeedDoc { entries }, ApplyOptions::default()).await;

    if !report.failed.is_empty() {
        return Err(anyhow::anyhow!(
            "failed to persist {} secret(s): {:?}",
            report.failed.len(),
            report.failed
        ));
    }

    // C7: emit `pack-config-input.v1` for the deployer when a pack is on hand
    // and a FormSpec can be derived. Soft-fail — the C4.2 compat shim still
    // serves these keys from DevStore (just populated above).
    if let Some(pp) = pack_path {
        try_emit_pack_config_input(bundle_root, pp, env, provider_id, config);
    }

    Ok(saved_keys)
}

/// Read `assets/secret-requirements.json` from a pack and seed alias entries
/// for any requirement key that differs from the answers key after
/// canonicalization.
fn seed_secret_requirement_aliases(
    entries: &mut Vec<SeedEntry>,
    config_map: &JsonMap<String, Value>,
    env: &str,
    tenant: &str,
    team: Option<&str>,
    provider_id: &str,
    pack_path: &Path,
) {
    let reqs = match read_secret_requirements(pack_path) {
        Ok(r) => r,
        Err(_) => return,
    };
    let normalize = crate::secret_name::canonical_secret_name;
    let existing_keys: std::collections::HashSet<String> = entries
        .iter()
        .filter_map(|e| e.uri.rsplit('/').next().map(String::from))
        .collect();

    for req in &reqs {
        let canonical_req_key = normalize(&req.key);
        if existing_keys.contains(&canonical_req_key) {
            continue;
        }
        // Try to find a matching value in config_map by checking if the
        // requirement key ends with the config key (e.g. WEBEX_BOT_TOKEN
        // matches bot_token, SLACK_BOT_TOKEN matches bot_token).
        let matched_value = config_map.iter().find_map(|(cfg_key, cfg_val)| {
            let norm_cfg = normalize(cfg_key);
            if canonical_req_key.ends_with(&norm_cfg) {
                let text = match cfg_val {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if text.is_empty() || text == "null" {
                    None
                } else {
                    Some(text)
                }
            } else {
                None
            }
        });
        if let Some(text) = matched_value {
            let uri = canonical_secret_uri(env, tenant, team, provider_id, &canonical_req_key);
            entries.push(SeedEntry {
                uri,
                format: SecretFormat::Text,
                value: SeedValue::Text { text },
                description: Some(format!("alias from {} for {provider_id}", req.key)),
            });
        }
    }
}

/// Minimal representation of a secret-requirements.json entry.
#[derive(serde::Deserialize)]
struct SecretRequirement {
    key: String,
}

fn read_secret_requirements(pack_path: &Path) -> Result<Vec<SecretRequirement>> {
    let file = std::fs::File::open(pack_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let entry = archive.by_name("assets/secret-requirements.json")?;
    let reqs: Vec<SecretRequirement> = serde_json::from_reader(entry)?;
    Ok(reqs)
}

/// Convenience function to persist both secrets and config from QA results.
///
/// Creates a `DevStore` from the bundle root and persists both.
#[allow(clippy::too_many_arguments)]
pub async fn persist_qa_results(
    bundle_root: &Path,
    providers_root: &Path,
    tenant: &str,
    team: Option<&str>,
    provider_id: &str,
    config: &Value,
    pack_path: &Path,
    form_spec: &FormSpec,
    backup: bool,
) -> Result<(Vec<String>, bool)> {
    let env = resolve_env(None);
    let store_path = crate::dev_store_path::ensure_path(bundle_root)?;
    let store = DevStore::with_path(&store_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to open dev secrets store {}: {err}",
            store_path.display()
        )
    })?;

    let saved_secrets =
        persist_qa_secrets(&store, &env, tenant, team, provider_id, config, form_spec).await?;

    // C7: emit `pack-config-input.v1` so the deployer can populate the
    // `pack-config.v1.non_secret` channel at revision-create. Soft-fail —
    // the C4.2 compat shim still serves these keys from DevStore (already
    // populated above), so a wizard run does not regress on emit failure.
    let bundle_id = infer_bundle_id(bundle_root);
    if let Err(err) = emit_pack_config_input(
        bundle_root,
        &env,
        &bundle_id,
        provider_id,
        config,
        form_spec,
    ) {
        tracing::warn!(
            provider_id,
            env = %env,
            bundle_id = %bundle_id,
            bundle_root = %bundle_root.display(),
            error = %err,
            "pack-config-input emission failed; runtime falls back to legacy DevStore reads via C4.2 compat shim",
        );
    }

    let config_written = if config.as_object().is_some_and(|m| !m.is_empty()) {
        persist_qa_config(
            providers_root,
            provider_id,
            config,
            pack_path,
            form_spec,
            backup,
        )?;
        true
    } else {
        false
    };

    Ok((saved_secrets, config_written))
}

/// OAuth authorization stub.
///
/// Prints the authorization URL and returns `None`. This is a placeholder for
/// future integration with `greentic-oauth` for providers that require OAuth
/// (e.g., Teams, Slack with OAuth scopes).
pub fn oauth_authorize_stub(provider_id: &str, auth_url: Option<&str>) -> Option<String> {
    if let Some(url) = auth_url {
        println!("[oauth] Authorize {provider_id} at: {url}");
        println!("[oauth] After authorizing, re-run setup to complete configuration.");
    } else {
        println!("[oauth] Provider {provider_id} requires OAuth authorization.");
        println!("[oauth] OAuth integration is not yet implemented.");
    }
    None
}

// ── pack-config-input.v1 emitter (C7) ──────────────────────────────────────

/// Schema tag for the wizard-emitted intermediate file consumed by the
/// greentic-deployer at revision-create.
pub const PACK_CONFIG_INPUT_SCHEMA: &str = "greentic.pack-config-input.v1";

/// Directory under `bundle_root` where wizard-emitted pack-config inputs land.
/// The deployer joins on `<bundle_root>/<PACK_CONFIG_INPUT_DIR>/<pack_id>.json`
/// at revision-create.
pub const PACK_CONFIG_INPUT_DIR: &str = "state/pack-configs";

/// Wizard-emitted intermediate file the deployer picks up at revision-create
/// (C7). The deployer stamps the active `revision_id` and writes the final
/// `pack-config.v1` referenced by `pack_config_refs` in `runtime-config.v1`.
/// We keep `revision_id` OUT of this shape on purpose: revisions are minted
/// by the deployer, not the wizard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackConfigInput {
    pub schema: String,
    pub pack_id: String,
    pub env_id: String,
    pub bundle_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub non_secret: BTreeMap<String, Value>,
    /// `secret://<env>/<bundle>/<pack>/<question>` URIs (kept as plain
    /// strings here — `greentic-deploy-spec::SecretRef` validates at the
    /// deployer side when it materializes the final `pack-config.v1`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_refs: BTreeMap<String, String>,
}

/// Emit a `pack-config-input.v1` file at
/// `<bundle_root>/state/pack-configs/<pack_id>.json` carrying the FormSpec-
/// split of one provider's QA answers (C7). Idempotent: overwrites in place
/// so re-running the wizard produces the same on-disk shape.
///
/// Secret-marked answers are recorded as `secret://<env>/<bundle>/<pack>/<key>`
/// URI references (no plaintext); non-secret answers stay inline. Empty
/// `config` is a no-op (no file written).
pub fn emit_pack_config_input(
    bundle_root: &Path,
    env_id: &str,
    bundle_id: &str,
    pack_id: &str,
    config: &Value,
    form_spec: &FormSpec,
) -> Result<Option<std::path::PathBuf>> {
    validate_segment("env_id", env_id)?;
    validate_segment("bundle_id", bundle_id)?;
    validate_segment("pack_id", pack_id)?;

    let Some(config_map) = config.as_object() else {
        return Ok(None);
    };
    if config_map.is_empty() {
        return Ok(None);
    }

    // Apply the same visibility filter that `persist_qa_secrets` uses so
    // that conditionally-invisible answers do not leak into the
    // pack-config-input file (and from there into runtime config).
    let visibility = resolve_visibility(form_spec, config, VisibilityMode::Visible);

    let secret_ids: std::collections::HashSet<&str> = form_spec
        .questions
        .iter()
        .filter(|q| q.secret)
        .map(|q| q.id.as_str())
        .collect();

    let visible_ids: std::collections::HashSet<&str> = form_spec
        .questions
        .iter()
        .filter(|q| visibility.get(&q.id).copied().unwrap_or(true))
        .map(|q| q.id.as_str())
        .collect();

    let mut non_secret = BTreeMap::new();
    let mut secret_refs = BTreeMap::new();
    for (key, value) in config_map {
        if !visible_ids.contains(key.as_str()) {
            continue;
        }
        let text = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if text.is_empty() || text == "null" {
            continue;
        }
        if secret_ids.contains(key.as_str()) {
            validate_segment("question.id", key)?;
            let uri = format!("secret://{env_id}/{bundle_id}/{pack_id}/{key}");
            secret_refs.insert(key.clone(), uri);
        } else {
            non_secret.insert(key.clone(), value.clone());
        }
    }

    if non_secret.is_empty() && secret_refs.is_empty() {
        return Ok(None);
    }

    let input = PackConfigInput {
        schema: PACK_CONFIG_INPUT_SCHEMA.to_string(),
        pack_id: pack_id.to_string(),
        env_id: env_id.to_string(),
        bundle_id: bundle_id.to_string(),
        non_secret,
        secret_refs,
    };

    let dir = bundle_root.join(PACK_CONFIG_INPUT_DIR);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create pack-config-input dir {}", dir.display()))?;
    let path = dir.join(format!("{pack_id}.json"));
    let body = serde_json::to_string_pretty(&input).context("serialize pack-config-input.v1")?;
    std::fs::write(&path, format!("{body}\n"))
        .with_context(|| format!("write pack-config-input {}", path.display()))?;

    tracing::debug!(
        pack_id,
        env_id,
        bundle_id,
        non_secret_count = input.non_secret.len(),
        secret_ref_count = input.secret_refs.len(),
        path = %path.display(),
        "wizard emitted pack-config-input.v1 (C7) for deployer pickup",
    );
    Ok(Some(path))
}

/// Reject empty, `/`-bearing, or relative-component (`.`, `..`) identifiers —
/// these would silently corrupt the `secret://<env>/<bundle>/<pack>/<question>`
/// path structure or the `<dir>/<pack_id>.json` file path.
fn validate_segment(label: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("{label} must not be empty for pack-config-input emission");
    }
    if value.contains('/') {
        anyhow::bail!(
            "{label} `{value}` contains '/' which would corrupt the pack-config-input layout"
        );
    }
    if value == "." || value == ".." {
        anyhow::bail!(
            "{label} `{value}` is a relative path component and would corrupt the pack-config-input layout"
        );
    }
    Ok(())
}

/// Derive a stable bundle id from the bundle root path, with a `"bundle"`
/// fallback when the directory name is missing or empty.
pub(crate) fn infer_bundle_id(root: &Path) -> String {
    root.file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "bundle".to_string())
}

/// C7: attempt to emit a `pack-config-input.v1` file for one provider when
/// a FormSpec can be derived from the on-disk pack. Soft-fails on missing
/// FormSpec or emission error — the C4.2 compat shim still serves these
/// keys from DevStore.
fn try_emit_pack_config_input(
    bundle_root: &Path,
    pack_path: &Path,
    env: &str,
    provider_id: &str,
    answers: &Value,
) {
    let Some(form_spec) =
        greentic_setup::setup_to_formspec::pack_to_form_spec(pack_path, provider_id)
    else {
        return;
    };
    let bundle_id = infer_bundle_id(bundle_root);
    if let Err(err) = emit_pack_config_input(
        bundle_root,
        env,
        &bundle_id,
        provider_id,
        answers,
        &form_spec,
    ) {
        tracing::warn!(
            provider_id,
            env = %env,
            bundle_id = %bundle_id,
            bundle_root = %bundle_root.display(),
            pack_path = %pack_path.display(),
            error = %err,
            "pack-config-input emission failed; runtime falls back to legacy DevStore reads via C4.2 compat shim",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_spec::{QuestionSpec, QuestionType};
    use serde_json::json;

    fn make_form_spec(questions: Vec<QuestionSpec>) -> FormSpec {
        FormSpec {
            id: "test".into(),
            title: "Test".into(),
            version: "1.0.0".into(),
            description: None,
            presentation: None,
            progress_policy: None,
            secrets_policy: None,
            store: vec![],
            validations: vec![],
            includes: vec![],
            questions,
        }
    }

    fn question(id: &str, secret: bool) -> QuestionSpec {
        QuestionSpec {
            id: id.into(),
            kind: QuestionType::String,
            title: id.into(),
            title_i18n: None,
            description: None,
            description_i18n: None,
            required: false,
            choices: None,
            default_value: None,
            secret,
            visible_if: None,
            constraint: None,
            list: None,
            computed: None,
            policy: Default::default(),
            computed_overridable: false,
        }
    }

    #[test]
    fn filters_out_secret_fields() {
        let config = json!({
            "enabled": true,
            "bot_token": "secret123",
            "public_url": "https://example.com"
        });
        let secret_ids = vec!["bot_token"];
        let filtered = filter_secrets(&config, &secret_ids);
        assert!(filtered.get("enabled").is_some());
        assert!(filtered.get("public_url").is_some());
        assert!(filtered.get("bot_token").is_none());
    }

    #[test]
    fn no_secrets_returns_full_config() {
        let config = json!({"enabled": true, "url": "https://example.com"});
        let filtered = filter_secrets(&config, &[]);
        assert_eq!(filtered, config);
    }

    #[test]
    fn identifies_secret_questions() {
        let spec = make_form_spec(vec![
            question("enabled", false),
            question("bot_token", true),
            question("api_secret", true),
            question("url", false),
        ]);
        let secret_ids: Vec<&str> = spec
            .questions
            .iter()
            .filter(|q| q.secret)
            .map(|q| q.id.as_str())
            .collect();
        assert_eq!(secret_ids, vec!["bot_token", "api_secret"]);
    }

    // ── C7: pack-config-input.v1 emitter ──────────────────────────────────

    /// Secrets land as `secret://` URI refs (no plaintext); non-secrets stay
    /// inline. Empty config → no file written.
    #[test]
    fn emit_pack_config_input_splits_secret_vs_non_secret() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let form = make_form_spec(vec![
            question("enabled", false),
            question("bot_token", true),
            question("public_url", false),
        ]);
        let config = json!({
            "enabled": true,
            "bot_token": "shhh",
            "public_url": "https://example.com",
        });
        let path =
            emit_pack_config_input(root, "local", "test-bundle", "provider-a", &config, &form)
                .expect("emit")
                .expect("path");
        assert!(path.exists());
        let bytes = std::fs::read(&path).expect("read");
        let parsed: PackConfigInput = serde_json::from_slice(&bytes).expect("parse");
        assert_eq!(parsed.schema, PACK_CONFIG_INPUT_SCHEMA);
        assert_eq!(parsed.pack_id, "provider-a");
        assert_eq!(parsed.env_id, "local");
        assert_eq!(parsed.bundle_id, "test-bundle");
        assert_eq!(parsed.non_secret.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(
            parsed.non_secret.get("public_url"),
            Some(&Value::String("https://example.com".into())),
        );
        assert!(
            !parsed.non_secret.contains_key("bot_token"),
            "secret must not be in non_secret"
        );
        assert_eq!(
            parsed.secret_refs.get("bot_token").map(String::as_str),
            Some("secret://local/test-bundle/provider-a/bot_token"),
            "secret recorded as URI ref"
        );
        // No plaintext for the secret anywhere in the file.
        let body = String::from_utf8(bytes).expect("utf8");
        assert!(
            !body.contains("shhh"),
            "plaintext secret leaked into pack-config-input: {body}"
        );
    }

    /// Same answers + same bundle_id + same provider_id, different env_id →
    /// different secret_refs. Pins the env-segment integrity of the URI.
    #[test]
    fn emit_pack_config_input_secret_refs_discriminate_on_env_id() {
        let tmp_a = tempfile::TempDir::new().expect("tempdir-a");
        let tmp_b = tempfile::TempDir::new().expect("tempdir-b");
        let form = make_form_spec(vec![question("api_token", true)]);
        let cfg = json!({"api_token": "x"});
        let pa = emit_pack_config_input(tmp_a.path(), "local", "b", "p", &cfg, &form)
            .expect("emit-a")
            .expect("path-a");
        let pb = emit_pack_config_input(tmp_b.path(), "staging", "b", "p", &cfg, &form)
            .expect("emit-b")
            .expect("path-b");
        let parsed_a: PackConfigInput =
            serde_json::from_slice(&std::fs::read(&pa).unwrap()).unwrap();
        let parsed_b: PackConfigInput =
            serde_json::from_slice(&std::fs::read(&pb).unwrap()).unwrap();
        assert_eq!(
            parsed_a.secret_refs.get("api_token").map(String::as_str),
            Some("secret://local/b/p/api_token")
        );
        assert_eq!(
            parsed_b.secret_refs.get("api_token").map(String::as_str),
            Some("secret://staging/b/p/api_token")
        );
    }

    /// Empty config → no file written (caller treats `Ok(None)` as no-op).
    #[test]
    fn emit_pack_config_input_skips_empty_config() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let form = make_form_spec(vec![question("enabled", false)]);
        let empty = json!({});
        assert!(
            emit_pack_config_input(root, "local", "b", "p", &empty, &form)
                .expect("emit")
                .is_none()
        );
        assert!(!root.join(PACK_CONFIG_INPUT_DIR).exists());
    }

    /// Reject `/`-bearing, empty, or relative-component path segments —
    /// `secret://` URI integrity + on-disk `<dir>/<pack_id>.json` layout
    /// depend on it.
    #[test]
    fn emit_pack_config_input_rejects_invalid_segments() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let form = make_form_spec(vec![question("k", false)]);
        let cfg = json!({"k": "v"});
        assert!(
            emit_pack_config_input(root, "", "b", "p", &cfg, &form).is_err(),
            "empty env_id rejected"
        );
        assert!(
            emit_pack_config_input(root, "local", "b", "../p", &cfg, &form).is_err(),
            "pack_id with `/` rejected"
        );
        assert!(
            emit_pack_config_input(root, "local", "b/c", "p", &cfg, &form).is_err(),
            "bundle_id with `/` rejected"
        );
        assert!(
            emit_pack_config_input(root, "local", "b", "..", &cfg, &form).is_err(),
            "pack_id `..` rejected"
        );
        assert!(
            emit_pack_config_input(root, ".", "b", "p", &cfg, &form).is_err(),
            "env_id `.` rejected"
        );
    }

    /// Invisible questions (conditional `visible_if` that evaluates to false)
    /// must not leak into the pack-config-input file.
    #[test]
    fn emit_pack_config_input_respects_visibility() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let form = make_form_spec(vec![question("mode", false), {
            let mut q = question("advanced_url", false);
            q.visible_if = Some(qa_spec::Expr::Eq {
                left: Box::new(qa_spec::Expr::Answer {
                    path: "mode".into(),
                }),
                right: Box::new(qa_spec::Expr::Literal {
                    value: Value::String("advanced".into()),
                }),
            });
            q
        }]);
        let config = json!({
            "mode": "basic",
            "advanced_url": "https://should-be-hidden.example.com",
        });
        let path = emit_pack_config_input(root, "local", "b", "p", &config, &form)
            .expect("emit")
            .expect("path");
        let parsed: PackConfigInput =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            !parsed.non_secret.contains_key("advanced_url"),
            "invisible question should not appear in non_secret: {parsed:?}"
        );
        assert_eq!(
            parsed.non_secret.get("mode"),
            Some(&Value::String("basic".into())),
        );
    }

    /// `infer_bundle_id` returns the directory name, with `"bundle"`
    /// fallback when the root has no usable file name.
    #[test]
    fn infer_bundle_id_uses_dir_name_with_bundle_fallback() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let nested = tmp.path().join("my-bundle");
        std::fs::create_dir_all(&nested).expect("nested");
        assert_eq!(infer_bundle_id(&nested), "my-bundle");
        assert_eq!(infer_bundle_id(Path::new("/")), "bundle");
    }
}
