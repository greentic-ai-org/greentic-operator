use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::capabilities::{
    CapabilityOfferRecord, CapabilitySubscriptionRecord, CapabilityTopicRecord, ResolveScope,
};
use crate::demo::runner_host::OperatorContext;
use crate::domains::Domain;
use crate::state_layout;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAdapterKind {
    DirectComponent,
    HttpCompatibility,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedCallableRecord {
    pub capability_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub stable_id: String,
    pub pack_id: String,
    pub domain: String,
    pub pack_path: String,
    pub provider_component_ref: String,
    pub provider_op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema_ref: Option<String>,
    pub version: String,
    pub requires_setup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_qa_ref: Option<String>,
    pub adapter_kind: CapabilityAdapterKind,
    pub scope: ResolvedScopeRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedScopeRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedTopicRecord {
    pub topic: String,
    pub event_type: String,
    pub pack_id: String,
    pub domain: String,
    pub pack_path: String,
    pub publisher_component_ref: String,
    pub publisher_op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_schema_ref: Option<String>,
    pub adapter_kind: CapabilityAdapterKind,
    pub priority: i32,
    pub scope: ResolvedScopeRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedSubscriptionRecord {
    pub subscription_id: String,
    pub topic: String,
    pub event_type: String,
    pub pack_id: String,
    pub domain: String,
    pub pack_path: String,
    pub capability_id: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_schema_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_json_pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_equals: Option<String>,
    pub delivery_mode: String,
    pub max_attempts: u32,
    pub priority: i32,
    pub scope: ResolvedScopeRecord,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityCatalogArtifacts {
    pub callables: Vec<ResolvedCallableRecord>,
    pub subscriptions: Vec<ResolvedSubscriptionRecord>,
    pub topics: Vec<ResolvedTopicRecord>,
}

impl CapabilityCatalogArtifacts {
    pub fn empty() -> Self {
        Self::default()
    }
}

pub fn build_catalog_artifacts(
    offers: &[CapabilityOfferRecord],
    topics: &[CapabilityTopicRecord],
    subscriptions: &[CapabilitySubscriptionRecord],
    ctx: &OperatorContext,
    scope: &ResolveScope,
) -> CapabilityCatalogArtifacts {
    let callables = offers
        .iter()
        .map(|offer| ResolvedCallableRecord {
            capability_id: offer.cap_id.clone(),
            operation_id: offer.operation_id.clone(),
            stable_id: offer.stable_id.clone(),
            pack_id: offer.pack_id.clone(),
            domain: domain_name(offer.domain).to_string(),
            pack_path: offer.pack_path.display().to_string(),
            provider_component_ref: offer.provider_component_ref.clone(),
            provider_op: offer.provider_op.clone(),
            input_schema_ref: offer.input_schema_ref.clone(),
            output_schema_ref: offer.output_schema_ref.clone(),
            version: offer.version.clone(),
            requires_setup: offer.requires_setup,
            setup_qa_ref: offer.setup_qa_ref.clone(),
            adapter_kind: adapter_kind(offer.adapter_kind.as_deref()),
            scope: ResolvedScopeRecord {
                env: scope.env.clone(),
                tenant: Some(ctx.tenant.clone()),
                team: ctx.team.clone(),
            },
        })
        .collect();
    let topics = topics
        .iter()
        .map(|topic| ResolvedTopicRecord {
            topic: topic.topic.clone(),
            event_type: topic.event_type.clone(),
            pack_id: topic.pack_id.clone(),
            domain: domain_name(topic.domain).to_string(),
            pack_path: topic.pack_path.display().to_string(),
            publisher_component_ref: topic.publisher_component_ref.clone(),
            publisher_op: topic.publisher_op.clone(),
            payload_schema_ref: topic.payload_schema_ref.clone(),
            adapter_kind: adapter_kind(topic.adapter_kind.as_deref()),
            priority: topic.priority,
            scope: ResolvedScopeRecord {
                env: scope.env.clone(),
                tenant: Some(ctx.tenant.clone()),
                team: ctx.team.clone(),
            },
        })
        .collect();
    let subscriptions = subscriptions
        .iter()
        .map(|subscription| ResolvedSubscriptionRecord {
            subscription_id: subscription.subscription_id.clone(),
            topic: subscription.topic.clone(),
            event_type: subscription.event_type.clone(),
            pack_id: subscription.pack_id.clone(),
            domain: domain_name(subscription.domain).to_string(),
            pack_path: subscription.pack_path.display().to_string(),
            capability_id: subscription.capability_id.clone(),
            op: subscription.op.clone(),
            payload_schema_ref: subscription.payload_schema_ref.clone(),
            filter_json_pointer: subscription.filter_json_pointer.clone(),
            filter_equals: subscription.filter_equals.clone(),
            delivery_mode: subscription.delivery_mode.clone(),
            max_attempts: subscription.max_attempts,
            priority: subscription.priority,
            scope: ResolvedScopeRecord {
                env: scope.env.clone(),
                tenant: Some(ctx.tenant.clone()),
                team: ctx.team.clone(),
            },
        })
        .collect();

    CapabilityCatalogArtifacts {
        callables,
        subscriptions,
        topics,
    }
}

pub fn write_capability_catalog_artifacts(
    root: &Path,
    tenant: &str,
    team: Option<&str>,
    artifacts: &CapabilityCatalogArtifacts,
) -> anyhow::Result<CapabilityCatalogPaths> {
    let callables_path = state_layout::capability_state_path(root, tenant, team, "callables.json");
    let subscriptions_path =
        state_layout::capability_state_path(root, tenant, team, "subscriptions.json");
    let topics_path = state_layout::capability_state_path(root, tenant, team, "topics.json");

    write_json(&callables_path, &artifacts.callables)?;
    write_json(&subscriptions_path, &artifacts.subscriptions)?;
    write_json(&topics_path, &artifacts.topics)?;

    Ok(CapabilityCatalogPaths {
        callables: callables_path,
        subscriptions: subscriptions_path,
        topics: topics_path,
    })
}

fn write_json(path: &PathBuf, value: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes)?;
    Ok(())
}

fn domain_name(domain: Domain) -> &'static str {
    match domain {
        Domain::Messaging => "messaging",
        Domain::Events => "events",
        Domain::Secrets => "secrets",
        Domain::OAuth => "oauth",
    }
}

fn adapter_kind(raw: Option<&str>) -> CapabilityAdapterKind {
    match raw {
        Some("http_compatibility") => CapabilityAdapterKind::HttpCompatibility,
        _ => CapabilityAdapterKind::DirectComponent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn writes_catalog_files_in_generic_state_layout() -> anyhow::Result<()> {
        let tmp = tempdir()?;
        let callable = ResolvedCallableRecord {
            stable_id: "offer.1".to_string(),
            pack_id: "pack-a".to_string(),
            domain: "messaging".to_string(),
            pack_path: "/tmp/pack-a.gtpack".to_string(),
            capability_id: "cap://example.v1".to_string(),
            operation_id: Some("perform_action".to_string()),
            version: "1".to_string(),
            provider_component_ref: "provider.component".to_string(),
            provider_op: "provider.invoke".to_string(),
            input_schema_ref: Some("schemas/input.json".to_string()),
            output_schema_ref: Some("schemas/output.json".to_string()),
            requires_setup: false,
            setup_qa_ref: None,
            adapter_kind: CapabilityAdapterKind::DirectComponent,
            scope: ResolvedScopeRecord {
                env: Some("dev".to_string()),
                tenant: Some("tenant-a".to_string()),
                team: Some("team-b".to_string()),
            },
        };
        let artifacts = CapabilityCatalogArtifacts {
            callables: vec![callable],
            subscriptions: vec![ResolvedSubscriptionRecord {
                subscription_id: "sub-a".to_string(),
                topic: "topic://example.v1".to_string(),
                event_type: "example.v1".to_string(),
                pack_id: "pack-b".to_string(),
                domain: "events".to_string(),
                pack_path: "/tmp/pack-b.gtpack".to_string(),
                capability_id: "cap://subscriber.v1".to_string(),
                op: "handle".to_string(),
                payload_schema_ref: Some("schemas/event.json".to_string()),
                filter_json_pointer: Some("/kind".to_string()),
                filter_equals: Some("match".to_string()),
                delivery_mode: "at_least_once".to_string(),
                max_attempts: 2,
                priority: 10,
                scope: ResolvedScopeRecord {
                    env: Some("dev".to_string()),
                    tenant: Some("tenant-a".to_string()),
                    team: Some("team-b".to_string()),
                },
            }],
            topics: vec![ResolvedTopicRecord {
                topic: "topic://example.v1".to_string(),
                event_type: "example.v1".to_string(),
                pack_id: "pack-c".to_string(),
                domain: "events".to_string(),
                pack_path: "/tmp/pack-c.gtpack".to_string(),
                publisher_component_ref: "events.component".to_string(),
                publisher_op: "publish".to_string(),
                payload_schema_ref: Some("schemas/event.json".to_string()),
                adapter_kind: CapabilityAdapterKind::DirectComponent,
                priority: 5,
                scope: ResolvedScopeRecord {
                    env: Some("dev".to_string()),
                    tenant: Some("tenant-a".to_string()),
                    team: Some("team-b".to_string()),
                },
            }],
        };
        let paths =
            write_capability_catalog_artifacts(tmp.path(), "tenant-a", Some("team-b"), &artifacts)?;

        assert!(paths.callables.exists());
        assert!(paths.subscriptions.exists());
        assert!(paths.topics.exists());

        let callables = std::fs::read_to_string(paths.callables)?;
        let subscriptions = std::fs::read_to_string(paths.subscriptions)?;
        let topics = std::fs::read_to_string(paths.topics)?;
        assert!(callables.contains("\"capability_id\": \"cap://example.v1\""));
        assert!(callables.contains("\"adapter_kind\": \"direct_component\""));
        assert!(subscriptions.contains("\"subscription_id\": \"sub-a\""));
        assert!(topics.contains("\"topic\": \"topic://example.v1\""));
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityCatalogPaths {
    pub callables: PathBuf,
    pub subscriptions: PathBuf,
    pub topics: PathBuf,
}
