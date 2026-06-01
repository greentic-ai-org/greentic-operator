use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use jsonschema::validate;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;
use uuid::Uuid;
use zip::ZipArchive;

use crate::capability_runtime::{ResolvedSubscriptionRecord, ResolvedTopicRecord};
use crate::demo::runner_host::{DemoRunnerHost, FlowOutcome, OperatorContext};
use crate::state_layout;

pub trait CapabilityEventRunner {
    fn invoke_capability(
        &self,
        cap_id: &str,
        op: &str,
        payload_bytes: &[u8],
        ctx: &OperatorContext,
    ) -> Result<FlowOutcome>;
}

impl CapabilityEventRunner for DemoRunnerHost {
    fn invoke_capability(
        &self,
        cap_id: &str,
        op: &str,
        payload_bytes: &[u8],
        ctx: &OperatorContext,
    ) -> Result<FlowOutcome> {
        DemoRunnerHost::invoke_capability(self, cap_id, op, payload_bytes, ctx)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CapabilityEventEnvelope {
    pub event_id: String,
    pub topic: String,
    pub event_type: String,
    pub payload: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub published_at_unix_ms: u64,
}

#[derive(Clone, Debug)]
pub struct PublishEventRequest {
    pub topic: String,
    pub event_type: Option<String>,
    pub payload: JsonValue,
    pub correlation_id: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityDeliveryRecord {
    pub delivery_id: String,
    pub event_id: String,
    pub subscription_id: String,
    pub capability_id: String,
    pub op: String,
    pub status: String,
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub delivered_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CapabilityDeadLetterRecord {
    pub delivery_id: String,
    pub event: CapabilityEventEnvelope,
    pub subscription_id: String,
    pub capability_id: String,
    pub op: String,
    pub attempts: u32,
    pub error: String,
    pub failed_at_unix_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishReport {
    pub matched_subscriptions: usize,
    pub successful_deliveries: usize,
    pub failed_deliveries: usize,
}

pub fn publish_event(
    bundle_root: &Path,
    runner: &impl CapabilityEventRunner,
    ctx: &OperatorContext,
    topics: &[ResolvedTopicRecord],
    subscriptions: &[ResolvedSubscriptionRecord],
    request: PublishEventRequest,
) -> Result<PublishReport> {
    publish_event_internal(
        bundle_root,
        runner,
        ctx,
        topics,
        subscriptions,
        request,
        None,
        None,
    )
}

pub fn replay_event(
    bundle_root: &Path,
    runner: &impl CapabilityEventRunner,
    ctx: &OperatorContext,
    topics: &[ResolvedTopicRecord],
    subscriptions: &[ResolvedSubscriptionRecord],
    event_id: Option<&str>,
    delivery_id: Option<&str>,
) -> Result<PublishReport> {
    let delivery = if let Some(id) = delivery_id {
        Some(
            read_jsonl::<CapabilityDeadLetterRecord>(&dlq_path(
                bundle_root,
                &ctx.tenant,
                ctx.team.as_deref(),
            ))?
            .into_iter()
            .find(|item| item.delivery_id == id)
            .ok_or_else(|| anyhow!("dlq delivery {id} not found"))?,
        )
    } else {
        None
    };

    let envelope = if let Some(item) = delivery.as_ref() {
        item.event.clone()
    } else if let Some(id) = event_id {
        read_jsonl::<CapabilityEventEnvelope>(&events_path(
            bundle_root,
            &ctx.tenant,
            ctx.team.as_deref(),
        ))?
        .into_iter()
        .find(|item| item.event_id == id)
        .ok_or_else(|| anyhow!("event {id} not found"))?
    } else {
        anyhow::bail!("either event_id or delivery_id is required for replay");
    };

    let subscription_filter = delivery.as_ref().map(|item| item.subscription_id.as_str());
    let delivery_id_filter = delivery.as_ref().map(|item| item.delivery_id.as_str());
    publish_event_internal(
        bundle_root,
        runner,
        ctx,
        topics,
        subscriptions,
        PublishEventRequest {
            topic: envelope.topic,
            event_type: Some(envelope.event_type),
            payload: envelope.payload,
            correlation_id: envelope.correlation_id,
            source: envelope.source,
        },
        subscription_filter,
        delivery_id_filter,
    )
}

pub fn read_events(
    bundle_root: &Path,
    tenant: &str,
    team: Option<&str>,
) -> Result<Vec<CapabilityEventEnvelope>> {
    read_jsonl(&events_path(bundle_root, tenant, team))
}

pub fn read_dead_letters(
    bundle_root: &Path,
    tenant: &str,
    team: Option<&str>,
) -> Result<Vec<CapabilityDeadLetterRecord>> {
    read_jsonl(&dlq_path(bundle_root, tenant, team))
}

fn publish_event_internal(
    bundle_root: &Path,
    runner: &impl CapabilityEventRunner,
    ctx: &OperatorContext,
    topics: &[ResolvedTopicRecord],
    subscriptions: &[ResolvedSubscriptionRecord],
    request: PublishEventRequest,
    only_subscription_id: Option<&str>,
    replay_delivery_id: Option<&str>,
) -> Result<PublishReport> {
    let topic = topics
        .iter()
        .find(|item| {
            item.topic == request.topic
                && request
                    .event_type
                    .as_ref()
                    .map(|value| value == &item.event_type)
                    .unwrap_or(true)
        })
        .ok_or_else(|| anyhow!("topic {} is not defined in current scope", request.topic))?;

    if let Some(schema_ref) = topic.payload_schema_ref.as_deref() {
        validate_schema(&topic.pack_path, schema_ref, &request.payload)
            .with_context(|| format!("invalid event payload for topic {}", topic.topic))?;
    }

    let envelope = CapabilityEventEnvelope {
        event_id: replay_delivery_id
            .map(|value| format!("replay-{value}"))
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        topic: topic.topic.clone(),
        event_type: topic.event_type.clone(),
        payload: request.payload,
        correlation_id: request.correlation_id,
        source: request.source,
        published_at_unix_ms: now_unix_ms(),
    };
    append_jsonl(
        &events_path(bundle_root, &ctx.tenant, ctx.team.as_deref()),
        &envelope,
    )?;

    let matched: Vec<_> = subscriptions
        .iter()
        .filter(|item| {
            item.topic == envelope.topic
                && item.event_type == envelope.event_type
                && only_subscription_id
                    .map(|value| value == item.subscription_id)
                    .unwrap_or(true)
                && matches_filter(item, &envelope.payload)
        })
        .cloned()
        .collect();

    let mut report = PublishReport {
        matched_subscriptions: matched.len(),
        successful_deliveries: 0,
        failed_deliveries: 0,
    };

    let payload_bytes = serde_json::to_vec(&envelope.payload)?;

    for subscription in matched {
        if let Some(schema_ref) = subscription.payload_schema_ref.as_deref() {
            validate_schema(&subscription.pack_path, schema_ref, &envelope.payload).with_context(
                || {
                    format!(
                        "invalid payload for subscription {}",
                        subscription.subscription_id
                    )
                },
            )?;
        }

        let mut attempts = 0u32;
        let mut final_error = None;
        let max_attempts = subscription.max_attempts.max(1);
        while attempts < max_attempts {
            attempts += 1;
            let outcome = runner.invoke_capability(
                &subscription.capability_id,
                &subscription.op,
                &payload_bytes,
                ctx,
            )?;
            if outcome.success {
                let record = CapabilityDeliveryRecord {
                    delivery_id: Uuid::new_v4().to_string(),
                    event_id: envelope.event_id.clone(),
                    subscription_id: subscription.subscription_id.clone(),
                    capability_id: subscription.capability_id.clone(),
                    op: subscription.op.clone(),
                    status: "ok".to_string(),
                    attempts,
                    error: None,
                    delivered_at_unix_ms: now_unix_ms(),
                };
                append_jsonl(
                    &deliveries_path(bundle_root, &ctx.tenant, ctx.team.as_deref()),
                    &record,
                )?;
                report.successful_deliveries += 1;
                final_error = None;
                break;
            }
            final_error = Some(
                outcome
                    .error
                    .unwrap_or_else(|| "capability invocation failed".to_string()),
            );
        }

        if let Some(error) = final_error {
            let delivery_id = Uuid::new_v4().to_string();
            let record = CapabilityDeliveryRecord {
                delivery_id: delivery_id.clone(),
                event_id: envelope.event_id.clone(),
                subscription_id: subscription.subscription_id.clone(),
                capability_id: subscription.capability_id.clone(),
                op: subscription.op.clone(),
                status: "err".to_string(),
                attempts: max_attempts,
                error: Some(error.clone()),
                delivered_at_unix_ms: now_unix_ms(),
            };
            append_jsonl(
                &deliveries_path(bundle_root, &ctx.tenant, ctx.team.as_deref()),
                &record,
            )?;
            append_jsonl(
                &dlq_path(bundle_root, &ctx.tenant, ctx.team.as_deref()),
                &CapabilityDeadLetterRecord {
                    delivery_id,
                    event: envelope.clone(),
                    subscription_id: subscription.subscription_id,
                    capability_id: subscription.capability_id,
                    op: subscription.op,
                    attempts: max_attempts,
                    error,
                    failed_at_unix_ms: now_unix_ms(),
                },
            )?;
            report.failed_deliveries += 1;
        }
    }

    Ok(report)
}

fn matches_filter(subscription: &ResolvedSubscriptionRecord, payload: &JsonValue) -> bool {
    let Some(pointer) = subscription.filter_json_pointer.as_deref() else {
        return true;
    };
    let Some(expected) = subscription.filter_equals.as_deref() else {
        return true;
    };
    payload
        .pointer(pointer)
        .and_then(|value| value.as_str())
        .map(|value| value == expected)
        .unwrap_or(false)
}

fn validate_schema(pack_path: &str, schema_ref: &str, payload: &JsonValue) -> Result<()> {
    let schema = load_schema(pack_path, schema_ref)?;
    validate(&schema, payload).map_err(|err| anyhow!("{err}"))
}

fn load_schema(pack_path: &str, schema_ref: &str) -> Result<JsonValue> {
    let path = Path::new(pack_path);
    if path.is_dir() {
        let schema_path = path.join(schema_ref);
        let bytes = fs::read(&schema_path)
            .with_context(|| format!("failed to read schema {}", schema_path.display()))?;
        return serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse schema {}", schema_path.display()));
    }

    let file = fs::File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive
        .by_name(schema_ref)
        .with_context(|| format!("schema {schema_ref} missing in {}", path.display()))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse schema {schema_ref} in {}", path.display()))
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        items.push(serde_json::from_str(trimmed)?);
    }
    Ok(items)
}

fn events_path(root: &Path, tenant: &str, team: Option<&str>) -> std::path::PathBuf {
    state_layout::capability_state_path(root, tenant, team, "events.jsonl")
}

fn deliveries_path(root: &Path, tenant: &str, team: Option<&str>) -> std::path::PathBuf {
    state_layout::capability_state_path(root, tenant, team, "deliveries.jsonl")
}

fn dlq_path(root: &Path, tenant: &str, team: Option<&str>) -> std::path::PathBuf {
    state_layout::capability_state_path(root, tenant, team, "dlq.jsonl")
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::FileOptions;

    #[derive(Default)]
    struct FakeRunner {
        fail_capability: Option<String>,
        invocations: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl CapabilityEventRunner for FakeRunner {
        fn invoke_capability(
            &self,
            cap_id: &str,
            op: &str,
            _payload_bytes: &[u8],
            _ctx: &OperatorContext,
        ) -> Result<FlowOutcome> {
            self.invocations
                .lock()
                .expect("invocations lock")
                .push((cap_id.to_string(), op.to_string()));
            let success = self.fail_capability.as_deref() != Some(cap_id);
            Ok(FlowOutcome {
                success,
                output: None,
                raw: None,
                error: if success {
                    None
                } else {
                    Some("boom".to_string())
                },
                mode: crate::demo::runner_host::RunnerExecutionMode::Exec,
            })
        }
    }

    #[test]
    fn publish_event_writes_logs_and_invokes_subscriber() -> Result<()> {
        let tmp = tempdir()?;
        let pack_path = write_schema_pack(tmp.path(), "schemas/event.schema.json")?;
        let runner = FakeRunner::default();
        let ctx = OperatorContext {
            tenant: "tenant-a".to_string(),
            team: Some("team-b".to_string()),
            correlation_id: None,
        };
        let topics = vec![ResolvedTopicRecord {
            topic: "topic://example.domain.event.v1".to_string(),
            event_type: "example.domain.event.v1".to_string(),
            pack_id: "pack-a".to_string(),
            domain: "events".to_string(),
            pack_path: pack_path.display().to_string(),
            publisher_component_ref: "publisher".to_string(),
            publisher_op: "publish".to_string(),
            payload_schema_ref: Some("schemas/event.schema.json".to_string()),
            adapter_kind: crate::capability_runtime::CapabilityAdapterKind::DirectComponent,
            priority: 10,
            scope: crate::capability_runtime::ResolvedScopeRecord {
                env: None,
                tenant: Some("tenant-a".to_string()),
                team: Some("team-b".to_string()),
            },
        }];
        let subscriptions = vec![ResolvedSubscriptionRecord {
            subscription_id: "sub-a".to_string(),
            topic: "topic://example.domain.event.v1".to_string(),
            event_type: "example.domain.event.v1".to_string(),
            pack_id: "pack-b".to_string(),
            domain: "events".to_string(),
            pack_path: pack_path.display().to_string(),
            capability_id: "cap://subscriber.handle.v1".to_string(),
            op: "handle".to_string(),
            payload_schema_ref: None,
            filter_json_pointer: Some("/kind".to_string()),
            filter_equals: Some("match".to_string()),
            delivery_mode: "at_least_once".to_string(),
            max_attempts: 2,
            priority: 10,
            scope: crate::capability_runtime::ResolvedScopeRecord {
                env: None,
                tenant: Some("tenant-a".to_string()),
                team: Some("team-b".to_string()),
            },
        }];

        let report = publish_event(
            tmp.path(),
            &runner,
            &ctx,
            &topics,
            &subscriptions,
            PublishEventRequest {
                topic: "topic://example.domain.event.v1".to_string(),
                event_type: None,
                payload: serde_json::json!({"kind":"match"}),
                correlation_id: None,
                source: Some("test".to_string()),
            },
        )?;

        assert_eq!(report.matched_subscriptions, 1);
        assert_eq!(report.successful_deliveries, 1);
        assert_eq!(report.failed_deliveries, 0);
        assert_eq!(
            read_events(tmp.path(), "tenant-a", Some("team-b"))?.len(),
            1
        );
        assert_eq!(
            read_dead_letters(tmp.path(), "tenant-a", Some("team-b"))?.len(),
            0
        );
        Ok(())
    }

    #[test]
    fn failed_delivery_goes_to_dlq_and_replays() -> Result<()> {
        let tmp = tempdir()?;
        let pack_path = write_schema_pack(tmp.path(), "schemas/event.schema.json")?;
        let failing_runner = FakeRunner {
            fail_capability: Some("cap://subscriber.handle.v1".to_string()),
            ..Default::default()
        };
        let ctx = OperatorContext {
            tenant: "tenant-a".to_string(),
            team: Some("team-b".to_string()),
            correlation_id: None,
        };
        let topics = vec![ResolvedTopicRecord {
            topic: "topic://example.domain.event.v1".to_string(),
            event_type: "example.domain.event.v1".to_string(),
            pack_id: "pack-a".to_string(),
            domain: "events".to_string(),
            pack_path: pack_path.display().to_string(),
            publisher_component_ref: "publisher".to_string(),
            publisher_op: "publish".to_string(),
            payload_schema_ref: Some("schemas/event.schema.json".to_string()),
            adapter_kind: crate::capability_runtime::CapabilityAdapterKind::DirectComponent,
            priority: 10,
            scope: crate::capability_runtime::ResolvedScopeRecord {
                env: None,
                tenant: Some("tenant-a".to_string()),
                team: Some("team-b".to_string()),
            },
        }];
        let subscriptions = vec![ResolvedSubscriptionRecord {
            subscription_id: "sub-a".to_string(),
            topic: "topic://example.domain.event.v1".to_string(),
            event_type: "example.domain.event.v1".to_string(),
            pack_id: "pack-b".to_string(),
            domain: "events".to_string(),
            pack_path: pack_path.display().to_string(),
            capability_id: "cap://subscriber.handle.v1".to_string(),
            op: "handle".to_string(),
            payload_schema_ref: None,
            filter_json_pointer: None,
            filter_equals: None,
            delivery_mode: "at_least_once".to_string(),
            max_attempts: 2,
            priority: 10,
            scope: crate::capability_runtime::ResolvedScopeRecord {
                env: None,
                tenant: Some("tenant-a".to_string()),
                team: Some("team-b".to_string()),
            },
        }];

        let report = publish_event(
            tmp.path(),
            &failing_runner,
            &ctx,
            &topics,
            &subscriptions,
            PublishEventRequest {
                topic: "topic://example.domain.event.v1".to_string(),
                event_type: None,
                payload: serde_json::json!({"kind":"match"}),
                correlation_id: None,
                source: Some("test".to_string()),
            },
        )?;
        assert_eq!(report.failed_deliveries, 1);
        let dlq = read_dead_letters(tmp.path(), "tenant-a", Some("team-b"))?;
        assert_eq!(dlq.len(), 1);

        let runner = FakeRunner::default();
        let replay = replay_event(
            tmp.path(),
            &runner,
            &ctx,
            &topics,
            &subscriptions,
            None,
            Some(&dlq[0].delivery_id),
        )?;
        assert_eq!(replay.successful_deliveries, 1);
        Ok(())
    }

    #[test]
    fn invalid_payload_fails_schema_validation() -> Result<()> {
        let tmp = tempdir()?;
        let pack_path = write_schema_pack(tmp.path(), "schemas/event.schema.json")?;
        let runner = FakeRunner::default();
        let ctx = OperatorContext {
            tenant: "tenant-a".to_string(),
            team: Some("team-b".to_string()),
            correlation_id: None,
        };
        let topics = vec![ResolvedTopicRecord {
            topic: "topic://example.domain.event.v1".to_string(),
            event_type: "example.domain.event.v1".to_string(),
            pack_id: "pack-a".to_string(),
            domain: "events".to_string(),
            pack_path: pack_path.display().to_string(),
            publisher_component_ref: "publisher".to_string(),
            publisher_op: "publish".to_string(),
            payload_schema_ref: Some("schemas/event.schema.json".to_string()),
            adapter_kind: crate::capability_runtime::CapabilityAdapterKind::DirectComponent,
            priority: 10,
            scope: crate::capability_runtime::ResolvedScopeRecord {
                env: None,
                tenant: Some("tenant-a".to_string()),
                team: Some("team-b".to_string()),
            },
        }];

        let err = publish_event(
            tmp.path(),
            &runner,
            &ctx,
            &topics,
            &[],
            PublishEventRequest {
                topic: "topic://example.domain.event.v1".to_string(),
                event_type: None,
                payload: serde_json::json!({"kind": 1}),
                correlation_id: None,
                source: None,
            },
        )
        .expect_err("invalid payload");
        assert!(err.to_string().contains("invalid event payload"));
        Ok(())
    }

    fn write_schema_pack(root: &Path, schema_ref: &str) -> Result<std::path::PathBuf> {
        let pack_path = root.join("schema-pack.gtpack");
        let file = fs::File::create(&pack_path)?;
        let mut zip = ZipWriter::new(file);
        zip.start_file(schema_ref, FileOptions::<()>::default())?;
        zip.write_all(
            br#"{"type":"object","properties":{"kind":{"type":"string"}},"required":["kind"]}"#,
        )?;
        zip.finish()?;
        Ok(pack_path)
    }
}
