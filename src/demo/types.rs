use anyhow::Error;
use greentic_runner_host::runner::engine::FlowSnapshot;
use serde_json::{Map, Value, json};

/// Describes the current blocked state of a demo flow execution.
#[derive(Debug)]
pub enum DemoBlockedOn {
    /// The flow is waiting for an external input before continuing.
    Waiting {
        reason: Option<String>,
        snapshot: Box<FlowSnapshot>,
        output: Value,
    },
    /// The flow finished and produced a terminal output value.
    Finished(Value),
    /// Execution hit an internal error before blocking/completing.
    Error(Error),
}

/// Represents an input event supplied to the runner.
#[derive(Clone, Debug)]
pub enum UserEvent {
    /// The user submitted an adaptive card via `action_id` and associated fields.
    CardSubmit {
        action_id: String,
        fields: Map<String, Value>,
    },
    /// Arbitrary raw event payload, used by tests or tooling.
    Raw(Value),
}

impl UserEvent {
    pub fn card_submit(action_id: impl Into<String>, fields: Map<String, Value>) -> Self {
        Self::CardSubmit {
            action_id: action_id.into(),
            fields,
        }
    }

    pub fn raw(value: Value) -> Self {
        Self::Raw(value)
    }

    pub fn into_value(self) -> Value {
        match self {
            UserEvent::CardSubmit { action_id, fields } => {
                json!({ "action_id": action_id, "inputs": Value::Object(fields) })
            }
            UserEvent::Raw(value) => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_submit_converts_to_expected_json_shape() {
        let mut fields = Map::new();
        fields.insert("token".to_string(), Value::String("secret".to_string()));
        let value = UserEvent::card_submit("save", fields).into_value();
        assert_eq!(value["action_id"], "save");
        assert_eq!(value["inputs"]["token"], "secret");
    }

    #[test]
    fn raw_event_passes_through_unchanged() {
        let value = json!({ "kind": "message", "text": "hello" });
        assert_eq!(UserEvent::raw(value.clone()).into_value(), value);
    }
}
