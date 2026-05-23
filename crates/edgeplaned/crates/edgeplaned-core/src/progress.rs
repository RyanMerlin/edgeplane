use serde::{Deserialize, Serialize};

/// A typed progress event emitted by an agent while executing a task.
///
/// The event stream is the primary surface for "seeing what agents are doing".
/// Events are structured so UIs and CLIs can render them well — not just raw logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub event_type: ProgressEventType,
    pub phase: Option<String>,
    pub step: Option<String>,
    pub summary: String,
    pub payload: serde_json::Value,
}

impl ProgressEvent {
    pub fn info(summary: impl Into<String>) -> Self {
        ProgressEvent {
            event_type: ProgressEventType::Info,
            phase: None,
            step: None,
            summary: summary.into(),
            payload: serde_json::Value::Null,
        }
    }

    pub fn phase_started(phase: impl Into<String>, summary: impl Into<String>) -> Self {
        let phase = phase.into();
        ProgressEvent {
            event_type: ProgressEventType::PhaseStarted,
            phase: Some(phase.clone()),
            step: None,
            summary: summary.into(),
            payload: serde_json::Value::Null,
        }
    }

    pub fn step_finished(
        phase: impl Into<String>,
        step: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        ProgressEvent {
            event_type: ProgressEventType::StepFinished,
            phase: Some(phase.into()),
            step: Some(step.into()),
            summary: summary.into(),
            payload: serde_json::Value::Null,
        }
    }

    pub fn artifact_produced(name: impl Into<String>, path: impl Into<String>) -> Self {
        let name = name.into();
        let path = path.into();
        ProgressEvent {
            event_type: ProgressEventType::ArtifactProduced,
            phase: None,
            step: None,
            summary: format!("produced artifact: {name}"),
            payload: serde_json::json!({ "name": name, "path": path }),
        }
    }

    pub fn needs_input(prompt: impl Into<String>) -> Self {
        let prompt = prompt.into();
        ProgressEvent {
            event_type: ProgressEventType::NeedsInput,
            phase: None,
            step: None,
            summary: format!("needs input: {prompt}"),
            payload: serde_json::json!({ "prompt": prompt }),
        }
    }

    pub fn error(summary: impl Into<String>, detail: serde_json::Value) -> Self {
        ProgressEvent {
            event_type: ProgressEventType::Error,
            phase: None,
            step: None,
            summary: summary.into(),
            payload: detail,
        }
    }
}

/// The closed set of typed event kinds.
///
/// Closed in v1 so UIs can render each case well; extensible via payload_json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressEventType {
    PhaseStarted,
    PhaseFinished,
    StepStarted,
    StepFinished,
    ArtifactProduced,
    ArtifactConsumed,
    WaitingOn,
    Unblocked,
    NeedsInput,
    InputReceived,
    MessageSent,
    MessageReceived,
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for ProgressEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{self:?}").to_lowercase());
        write!(f, "{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_has_correct_type_and_summary() {
        let ev = ProgressEvent::info("all good");
        assert_eq!(ev.event_type, ProgressEventType::Info);
        assert_eq!(ev.summary, "all good");
        assert!(ev.phase.is_none());
        assert!(ev.step.is_none());
    }

    #[test]
    fn phase_started_sets_phase_field() {
        let ev = ProgressEvent::phase_started("planning", "starting plan");
        assert_eq!(ev.event_type, ProgressEventType::PhaseStarted);
        assert_eq!(ev.phase.as_deref(), Some("planning"));
        assert_eq!(ev.summary, "starting plan");
    }

    #[test]
    fn step_finished_sets_both_phase_and_step() {
        let ev = ProgressEvent::step_finished("running", "compile", "done");
        assert_eq!(ev.event_type, ProgressEventType::StepFinished);
        assert_eq!(ev.phase.as_deref(), Some("running"));
        assert_eq!(ev.step.as_deref(), Some("compile"));
    }

    #[test]
    fn error_carries_payload() {
        let detail = serde_json::json!({"code": 1});
        let ev = ProgressEvent::error("something broke", detail.clone());
        assert_eq!(ev.event_type, ProgressEventType::Error);
        assert_eq!(ev.payload, detail);
    }

    #[test]
    fn artifact_produced_encodes_name_and_path() {
        let ev = ProgressEvent::artifact_produced("design.md", "/work/design.md");
        assert_eq!(ev.event_type, ProgressEventType::ArtifactProduced);
        assert_eq!(ev.payload["name"], "design.md");
        assert_eq!(ev.payload["path"], "/work/design.md");
    }

    #[test]
    fn needs_input_encodes_prompt_in_payload() {
        let ev = ProgressEvent::needs_input("which approach?");
        assert_eq!(ev.event_type, ProgressEventType::NeedsInput);
        assert_eq!(ev.payload["prompt"], "which approach?");
    }

    #[test]
    fn display_uses_snake_case() {
        assert_eq!(ProgressEventType::PhaseStarted.to_string(), "phase_started");
        assert_eq!(ProgressEventType::ArtifactProduced.to_string(), "artifact_produced");
        assert_eq!(ProgressEventType::Error.to_string(), "error");
    }

    #[test]
    fn round_trips_through_json() {
        let ev = ProgressEvent::phase_started("running", "go");
        let json = serde_json::to_string(&ev).unwrap();
        let ev2: ProgressEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev2.event_type, ProgressEventType::PhaseStarted);
        assert_eq!(ev2.phase.as_deref(), Some("running"));
    }
}
