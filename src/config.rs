use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Default)]
pub struct OperatorConfig {
    #[serde(default)]
    pub services: Option<OperatorServicesConfig>,
    #[serde(default)]
    pub binaries: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DomainEnabledMode {
    #[default]
    Auto,
    True,
    False,
}

impl DomainEnabledMode {
    pub fn is_enabled(self, has_providers: bool) -> bool {
        match self {
            Self::Auto => has_providers,
            Self::True => true,
            Self::False => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct OperatorServicesConfig {
    #[serde(default)]
    pub messaging: DomainServicesConfig,
    #[serde(default)]
    pub events: DomainServicesConfig,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct DomainServicesConfig {
    #[serde(default)]
    pub enabled: DomainEnabledMode,
    #[serde(default)]
    pub components: Vec<ServiceComponentConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServiceComponentConfig {
    pub id: String,
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
}

pub fn load_operator_config(root: &Path) -> anyhow::Result<Option<OperatorConfig>> {
    let path = root.join("greentic.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)?;
    if contents
        .lines()
        .all(|line| line.trim().is_empty() || line.trim().starts_with('#'))
    {
        return Ok(None);
    }
    let config: OperatorConfig = serde_yaml_bw::from_str(&contents)?;
    Ok(Some(config))
}

pub fn binary_override(
    config: Option<&OperatorConfig>,
    name: &str,
    config_dir: &Path,
) -> Option<PathBuf> {
    config.and_then(|config| config_binary_path(config, name, config_dir))
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoConfig {
    #[serde(default = "default_demo_tenant")]
    pub tenant: String,
    #[serde(default = "default_demo_team")]
    pub team: String,
    #[serde(default)]
    pub services: DemoServicesConfig,
    #[serde(default)]
    pub providers: Option<std::collections::BTreeMap<String, DemoProviderConfig>>,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            tenant: default_demo_tenant(),
            team: default_demo_team(),
            services: DemoServicesConfig::default(),
            providers: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct DemoServicesConfig {
    #[serde(default)]
    pub nats: DemoNatsConfig,
    #[serde(default)]
    pub gateway: DemoGatewayConfig,
    #[serde(default)]
    pub egress: DemoEgressConfig,
    #[serde(default)]
    pub subscriptions: DemoSubscriptionsConfig,
    #[serde(default)]
    pub events: DemoEventsConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoNatsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_nats_url")]
    pub url: String,
    #[serde(default)]
    pub spawn: DemoNatsSpawnConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoNatsSpawnConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_nats_binary")]
    pub binary: String,
    #[serde(default = "default_nats_args")]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoGatewayConfig {
    #[serde(default = "default_gateway_binary")]
    pub binary: String,
    #[serde(default = "default_gateway_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoEgressConfig {
    #[serde(default = "default_egress_binary")]
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoSubscriptionsConfig {
    #[serde(default = "default_subscriptions_mode")]
    pub mode: DemoSubscriptionsMode,
    #[serde(default)]
    pub universal: DemoSubscriptionsUniversalConfig,
    #[serde(default)]
    pub msgraph: DemoMsgraphSubscriptionsConfig,
}

impl Default for DemoSubscriptionsConfig {
    fn default() -> Self {
        Self {
            mode: default_subscriptions_mode(),
            universal: DemoSubscriptionsUniversalConfig::default(),
            msgraph: DemoMsgraphSubscriptionsConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DemoSubscriptionsMode {
    #[default]
    LegacyGsm,
    UniversalOps,
}

fn default_subscriptions_mode() -> DemoSubscriptionsMode {
    DemoSubscriptionsMode::LegacyGsm
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoSubscriptionsUniversalConfig {
    #[serde(default = "default_universal_renew_interval")]
    pub renew_interval_seconds: u64,
    #[serde(default = "default_universal_renew_skew")]
    pub renew_skew_minutes: u64,
    #[serde(default)]
    pub desired: Vec<DemoDesiredSubscription>,
}

impl Default for DemoSubscriptionsUniversalConfig {
    fn default() -> Self {
        Self {
            renew_interval_seconds: default_universal_renew_interval(),
            renew_skew_minutes: default_universal_renew_skew(),
            desired: Vec::new(),
        }
    }
}

fn default_universal_renew_interval() -> u64 {
    60
}

fn default_universal_renew_skew() -> u64 {
    10
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoDesiredSubscription {
    pub provider: String,
    pub resource: String,
    #[serde(default = "default_change_types")]
    pub change_types: Vec<String>,
    #[serde(default)]
    pub notification_url: Option<String>,
    #[serde(default)]
    pub client_state: Option<String>,
    #[serde(default)]
    pub binding_id: Option<String>,
    #[serde(default)]
    pub user: Option<AuthUserConfig>,
}

fn default_change_types() -> Vec<String> {
    vec!["created".to_string()]
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthUserConfig {
    pub user_id: String,
    pub token_key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoEventsConfig {
    #[serde(default)]
    pub enabled: DomainEnabledMode,
    #[serde(default = "default_events_components")]
    pub components: Vec<ServiceComponentConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoMsgraphSubscriptionsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_msgraph_binary")]
    pub binary: String,
    #[serde(default = "default_msgraph_mode")]
    pub mode: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DemoProviderConfig {
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(default)]
    pub setup_flow: Option<String>,
    #[serde(default)]
    pub verify_flow: Option<String>,
}

impl Default for DemoNatsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: default_nats_url(),
            spawn: DemoNatsSpawnConfig::default(),
        }
    }
}

impl Default for DemoNatsSpawnConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            binary: default_nats_binary(),
            args: default_nats_args(),
        }
    }
}

impl Default for DemoGatewayConfig {
    fn default() -> Self {
        Self {
            binary: default_gateway_binary(),
            listen_addr: default_gateway_listen_addr(),
            port: default_gateway_port(),
            args: Vec::new(),
        }
    }
}

impl Default for DemoEgressConfig {
    fn default() -> Self {
        Self {
            binary: default_egress_binary(),
            args: Vec::new(),
        }
    }
}

impl Default for DemoMsgraphSubscriptionsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            binary: default_msgraph_binary(),
            mode: default_msgraph_mode(),
            args: Vec::new(),
        }
    }
}

impl Default for DemoEventsConfig {
    fn default() -> Self {
        Self {
            enabled: DomainEnabledMode::Auto,
            components: default_events_components(),
        }
    }
}

pub fn load_demo_config(path: &Path) -> anyhow::Result<DemoConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: DemoConfig = serde_yaml_bw::from_str(&contents)?;
    Ok(config)
}

fn config_binary_path(config: &OperatorConfig, name: &str, config_dir: &Path) -> Option<PathBuf> {
    config
        .binaries
        .get(name)
        .map(|value| resolve_path(config_dir, value))
}

fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn default_demo_tenant() -> String {
    "demo".to_string()
}

fn default_demo_team() -> String {
    "default".to_string()
}

fn default_true() -> bool {
    true
}

pub fn default_nats_url() -> String {
    "nats://127.0.0.1:4222".to_string()
}

pub fn default_receive_nats_url() -> String {
    "nats://127.0.0.1:4347".to_string()
}

fn default_nats_binary() -> String {
    "nats-server".to_string()
}

fn default_nats_args() -> Vec<String> {
    vec!["-p".to_string(), "4222".to_string(), "-js".to_string()]
}

fn default_gateway_binary() -> String {
    "gateway".to_string()
}

fn default_gateway_listen_addr() -> String {
    "127.0.0.1".to_string()
}

fn default_gateway_port() -> u16 {
    8080
}

fn default_egress_binary() -> String {
    "egress".to_string()
}

fn default_msgraph_binary() -> String {
    "subscriptions-msgraph".to_string()
}

fn default_msgraph_mode() -> String {
    "poll".to_string()
}

pub(crate) fn default_events_components() -> Vec<ServiceComponentConfig> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_operator_config_returns_none_for_missing_or_comment_only_file() -> anyhow::Result<()> {
        let dir = tempdir()?;
        assert!(load_operator_config(dir.path())?.is_none());

        std::fs::write(
            dir.path().join("greentic.yaml"),
            "# comment only\n   \n# still comment\n",
        )?;
        assert!(load_operator_config(dir.path())?.is_none());
        Ok(())
    }

    #[test]
    fn load_operator_config_parses_binaries_and_service_components() -> anyhow::Result<()> {
        let dir = tempdir()?;
        std::fs::write(
            dir.path().join("greentic.yaml"),
            r#"
binaries:
  gateway: bin/gateway
services:
  messaging:
    enabled: "true"
    components:
      - id: gateway
        binary: gateway
        args: ["--port", "8080"]
"#,
        )?;

        let config = load_operator_config(dir.path())?.expect("config should exist");
        assert_eq!(
            config.binaries.get("gateway"),
            Some(&"bin/gateway".to_string())
        );
        let services = config.services.expect("services should be present");
        assert!(services.messaging.enabled.is_enabled(false));
        assert_eq!(services.messaging.components.len(), 1);
        assert_eq!(services.messaging.components[0].id, "gateway");
        assert_eq!(
            services.messaging.components[0].args,
            vec!["--port".to_string(), "8080".to_string()]
        );
        Ok(())
    }

    #[test]
    fn binary_override_resolves_relative_and_absolute_paths() {
        let mut config = OperatorConfig::default();
        config
            .binaries
            .insert("gateway".to_string(), "bin/gateway".to_string());
        config
            .binaries
            .insert("nats".to_string(), "/usr/bin/nats-server".to_string());

        assert_eq!(
            binary_override(Some(&config), "gateway", Path::new("/tmp/demo")),
            Some(PathBuf::from("/tmp/demo/bin/gateway"))
        );
        assert_eq!(
            binary_override(Some(&config), "nats", Path::new("/tmp/demo")),
            Some(PathBuf::from("/usr/bin/nats-server"))
        );
        assert_eq!(
            binary_override(Some(&config), "missing", Path::new("/tmp/demo")),
            None
        );
        assert_eq!(
            binary_override(None, "gateway", Path::new("/tmp/demo")),
            None
        );
    }

    #[test]
    fn load_demo_config_applies_defaults_for_optional_fields() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("demo.yaml");
        std::fs::write(
            &path,
            r#"
providers:
  github:
    pack: providers/github.gtpack
services:
  subscriptions:
    universal:
      desired:
        - provider: github
          resource: repo/issues
          user:
            user_id: demo-user
            token_key: gh_token
"#,
        )?;

        let config = load_demo_config(&path)?;
        assert_eq!(config.tenant, "demo");
        assert_eq!(config.team, "default");
        assert!(config.services.nats.enabled);
        assert_eq!(config.services.nats.url, default_nats_url());
        assert!(config.services.nats.spawn.enabled);
        assert_eq!(config.services.nats.spawn.binary, "nats-server");
        assert_eq!(config.services.gateway.binary, "gateway");
        assert_eq!(config.services.gateway.listen_addr, "127.0.0.1");
        assert_eq!(config.services.gateway.port, 8080);
        assert_eq!(config.services.egress.binary, "egress");
        assert!(matches!(
            config.services.subscriptions.mode,
            DemoSubscriptionsMode::LegacyGsm
        ));
        assert_eq!(
            config
                .services
                .subscriptions
                .universal
                .renew_interval_seconds,
            60
        );
        assert_eq!(
            config.services.subscriptions.universal.renew_skew_minutes,
            10
        );
        assert_eq!(
            config.services.subscriptions.universal.desired[0].change_types,
            vec!["created".to_string()]
        );
        assert_eq!(
            config.services.subscriptions.universal.desired[0]
                .user
                .as_ref()
                .unwrap()
                .token_key,
            "gh_token"
        );
        assert!(config.services.subscriptions.msgraph.enabled);
        assert_eq!(
            config.services.subscriptions.msgraph.binary,
            "subscriptions-msgraph"
        );
        assert_eq!(config.services.subscriptions.msgraph.mode, "poll");
        assert_eq!(config.services.events.components.len(), 0);
        assert!(!config.providers.expect("providers should exist").is_empty());
        Ok(())
    }

    #[test]
    fn domain_enabled_mode_respects_auto_and_explicit_values() {
        assert!(DomainEnabledMode::Auto.is_enabled(true));
        assert!(!DomainEnabledMode::Auto.is_enabled(false));
        assert!(DomainEnabledMode::True.is_enabled(false));
        assert!(!DomainEnabledMode::False.is_enabled(true));
    }
}
