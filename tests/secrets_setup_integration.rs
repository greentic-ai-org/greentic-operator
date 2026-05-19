use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Result;
use serde_json::json;
use tempfile::tempdir;
use tokio::runtime::Runtime;
use zip::{ZipWriter, write::FileOptions};

use greentic_operator::{
    config::{DemoConfig, DemoProviderConfig},
    dev_store_path,
    providers::{ProviderSetupOptions, run_provider_setup},
    secrets_gate,
};
use greentic_secrets_lib::{SecretsStore, core::seed::DevStore};

fn fake_bin(name: &str) -> PathBuf {
    if name == "greentic-operator" {
        return PathBuf::from(env!("CARGO_BIN_EXE_greentic-operator"));
    }
    example_bin(name)
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn example_bin(name: &str) -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.file_name().and_then(|name| name.to_str()) == Some("deps") {
        path.pop();
    }
    let candidate = path.join("examples").join(binary_name(name));
    if candidate.exists() {
        return candidate;
    }
    let status = Command::new("cargo")
        .args(["build", "--example", name])
        .status()
        .expect("failed to build example binary");
    assert!(status.success(), "failed to build example binary");
    candidate
}

fn write_pack_with_secret_requirements(
    path: &Path,
    pack_id: &str,
    entry_flows: &[&str],
    secret_keys: &[&str],
) -> Result<()> {
    let file = File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let manifest_options: FileOptions<'_, ()> = FileOptions::default();
    zip.start_file("manifest.cbor", manifest_options)?;
    let manifest = json!({
        "meta": {
            "pack_id": pack_id,
            "entry_flows": entry_flows,
        }
    });
    let manifest_bytes = serde_cbor::to_vec(&manifest)?;
    zip.write_all(&manifest_bytes)?;
    if !secret_keys.is_empty() {
        let requirements_options: FileOptions<'_, ()> = FileOptions::default();
        zip.start_file("secret_requirements.json", requirements_options)?;
        let requirements: Vec<_> = secret_keys
            .iter()
            .map(|key| json!({ "key": key, "required": true }))
            .collect();
        let body = serde_json::to_string(&requirements)?;
        zip.write_all(body.as_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

#[test]
fn provider_setup_seeds_and_resolves_secret() -> Result<()> {
    let bundle_temp = tempdir()?;
    let bundle_root = bundle_temp.path().to_path_buf();
    let pack_dir = bundle_root.join("provider-packs");
    fs::create_dir_all(&pack_dir)?;
    let pack_path = pack_dir.join("messaging-telegram.gtpack");
    write_pack_with_secret_requirements(
        &pack_path,
        "messaging-telegram",
        &["setup_default"],
        &["telegram_bot_token"],
    )?;

    let config = DemoConfig {
        tenant: "demo".to_string(),
        team: "default".to_string(),
        providers: Some(BTreeMap::from([(
            "messaging-telegram".to_string(),
            DemoProviderConfig {
                pack: Some("provider-packs/messaging-telegram.gtpack".to_string()),
                setup_flow: Some("setup_default".to_string()),
                verify_flow: None,
            },
        )])),
        ..Default::default()
    };
    let options = ProviderSetupOptions {
        providers: Some(vec!["messaging-telegram".to_string()]),
        verify_webhooks: false,
        force_setup: true,
        skip_setup: false,
        skip_secrets_init: false,
        allow_contract_change: false,
        backup: false,
        setup_input: None,
        runner_binary: Some(fake_bin("fake_runner")),
        continue_on_error: false,
    };

    run_provider_setup(&bundle_root, &config, None, options)?;

    let store_path = dev_store_path::ensure_path(&bundle_root)?;
    assert!(store_path.exists());
    let store = DevStore::with_path(store_path.clone())?;
    // A4b: `resolve_env(None)` now defaults to `local` (was `dev`); the
    // seeded URI scope must match.
    let uri = secrets_gate::canonical_secret_uri(
        "local",
        "demo",
        Some("default"),
        "messaging-telegram",
        "telegram_bot_token",
    );
    let runtime = Runtime::new()?;
    let stored_value = runtime.block_on(async { store.get(&uri).await })?;
    assert_eq!(stored_value, format!("placeholder for {uri}").into_bytes());

    let handle = secrets_gate::resolve_secrets_manager(&bundle_root, "demo", Some("default"))?;
    let resolved_value = runtime.block_on(async { handle.manager().read(&uri).await })?;
    assert_eq!(stored_value, resolved_value);
    Ok(())
}
