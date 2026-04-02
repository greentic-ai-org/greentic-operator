use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use zip::ZipArchive;

const BACKEND_CONFIG_PATHS: &[&str] = &[
    "assets/secrets_backend.json",
    "assets/secrets-backend.json",
    "secrets_backend.json",
    "secrets-backend.json",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretsBackendKind {
    DevStore,
    Env,
}

impl std::fmt::Display for SecretsBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            SecretsBackendKind::DevStore => "dev-store",
            SecretsBackendKind::Env => "env",
        };
        f.write_str(label)
    }
}

#[derive(Deserialize)]
struct PackBackendConfig {
    backend: Option<String>,
}

pub fn backend_kind_from_pack(pack_path: &Path) -> Result<SecretsBackendKind> {
    let file = File::open(pack_path)
        .with_context(|| format!("open secrets manager pack {}", pack_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("read secrets manager pack {}", pack_path.display()))?;
    for entry_name in BACKEND_CONFIG_PATHS {
        if let Ok(mut entry) = archive.by_name(entry_name) {
            let mut contents = String::new();
            entry
                .read_to_string(&mut contents)
                .with_context(|| format!("read backend config {}", entry_name))?;
            let config: PackBackendConfig = serde_json::from_str(&contents)
                .with_context(|| format!("parse secrets backend config in {}", entry_name))?;
            if let Some(kind) = config.backend {
                return match kind.trim().to_ascii_lowercase().as_str() {
                    "" | "default" | "dev-store" | "devstore" => Ok(SecretsBackendKind::DevStore),
                    "env" | "environment" => Ok(SecretsBackendKind::Env),
                    other => Err(anyhow!(
                        "unsupported secrets backend '{other}' in pack {}",
                        pack_path.display()
                    )),
                };
            }
            return Ok(SecretsBackendKind::DevStore);
        }
    }
    Err(anyhow!(
        "missing secrets backend config in pack {}",
        pack_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    fn write_pack(
        entries: &[(&str, &str)],
    ) -> anyhow::Result<(tempfile::TempDir, std::path::PathBuf)> {
        let dir = tempdir()?;
        let pack_path = dir.path().join("secrets.gtpack");
        let file = File::create(&pack_path)?;
        let mut zip = zip::ZipWriter::new(file);
        for (name, contents) in entries {
            zip.start_file(name, SimpleFileOptions::default())?;
            use std::io::Write;
            zip.write_all(contents.as_bytes())?;
        }
        zip.finish()?;
        Ok((dir, pack_path))
    }

    #[test]
    fn defaults_to_dev_store_when_backend_missing() -> anyhow::Result<()> {
        let (_dir, pack_path) = write_pack(&[("secrets_backend.json", "{}")])?;
        assert_eq!(
            backend_kind_from_pack(&pack_path)?,
            SecretsBackendKind::DevStore
        );
        Ok(())
    }

    #[test]
    fn accepts_env_backend_aliases_case_insensitively() -> anyhow::Result<()> {
        let (_dir, pack_path) = write_pack(&[(
            "assets/secrets-backend.json",
            r#"{ "backend": "Environment" }"#,
        )])?;
        assert_eq!(backend_kind_from_pack(&pack_path)?, SecretsBackendKind::Env);
        Ok(())
    }

    #[test]
    fn rejects_unknown_backend() -> anyhow::Result<()> {
        let (_dir, pack_path) =
            write_pack(&[("secrets-backend.json", r#"{ "backend": "vault" }"#)])?;
        let err = backend_kind_from_pack(&pack_path).unwrap_err().to_string();
        assert!(err.contains("unsupported secrets backend 'vault'"));
        Ok(())
    }

    #[test]
    fn errors_when_no_backend_config_exists() -> anyhow::Result<()> {
        let (_dir, pack_path) = write_pack(&[("README.txt", "hello")])?;
        let err = backend_kind_from_pack(&pack_path).unwrap_err().to_string();
        assert!(err.contains("missing secrets backend config"));
        Ok(())
    }
}
