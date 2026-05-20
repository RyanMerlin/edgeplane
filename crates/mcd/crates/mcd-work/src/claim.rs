use mcd_core::client::BackendClient;
use mcd_core::types::{Capability, MeshTaskRecord};
use anyhow::Result;

/// Result of a successful claim attempt.
pub struct ClaimOutcome {
    pub task: MeshTaskRecord,
    pub claim_lease_id: Option<String>,
}

/// Filter a list of tasks to those matching the given capabilities.
///
/// A task that declares `consumes` but no `depends_on` is misconfigured: its
/// inputs can't have been produced yet. Those tasks are skipped with a warning
/// so we don't claim something that will fail immediately.  Well-formed tasks
/// rely on the backend's `unblock_dependents` to hold them in `pending` until
/// their deps finish; by the time they appear as `ready` here, the gate is
/// already satisfied.
pub fn filter_eligible<'a>(tasks: &'a [MeshTaskRecord], caps: &[Capability]) -> Vec<&'a MeshTaskRecord> {
    tasks
        .iter()
        .filter(|t| {
            // Capability gate.
            if !t.required_capabilities
                .iter()
                .all(|req| caps.iter().any(|c| c.0 == *req))
            {
                return false;
            }
            // Consumes sanity gate: if a task declares consumes but has no
            // depends_on, its inputs can't exist yet — skip it.
            let has_consumes = t.consumes
                .as_object()
                .map(|m| !m.is_empty())
                .unwrap_or(false);
            if has_consumes && t.depends_on.is_empty() {
                tracing::warn!(
                    task_id = %t.id,
                    "task declares consumes but has no depends_on — skipping (misconfigured task)"
                );
                return false;
            }
            true
        })
        .collect()
}

/// Find and claim the highest-priority eligible task. Returns the claimed record
/// together with the `claim_lease_id`, or `None` if nothing was claimable.
pub async fn try_claim_one(
    client: &BackendClient,
    kluster_id: &str,
    caps: &[Capability],
) -> Result<Option<ClaimOutcome>> {
    let tasks = crate::task::poll_ready_tasks(client, kluster_id, caps).await?;
    let eligible = filter_eligible(&tasks, caps);
    let Some(candidate) = eligible.first() else {
        return Ok(None);
    };

    // Best-effort claim — another agent may race us; that's fine.
    match crate::task::claim_task(client, &candidate.id).await {
        Ok(result) => {
            let mut task = result.task;
            task.status = "claimed".into();
            Ok(Some(ClaimOutcome { task, claim_lease_id: result.claim_lease_id }))
        }
        Err(e) => {
            tracing::debug!("claim race lost for task {}: {e}", candidate.id);
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, required_caps: &[&str], claim_policy: &str) -> MeshTaskRecord {
        MeshTaskRecord {
            id: id.to_string(),
            kluster_id: "k1".to_string(),
            mission_id: "m1".to_string(),
            title: id.to_string(),
            description: String::new(),
            status: "ready".to_string(),
            claim_policy: claim_policy.to_string(),
            required_capabilities: required_caps.iter().map(|s| s.to_string()).collect(),
            lease_expires_at: None,
            claim_lease_id: None,
            depends_on: vec![],
            produces: serde_json::json!({}),
            consumes: serde_json::json!({}),
        }
    }

    fn task_with_consumes(id: &str, depends_on: Vec<String>) -> MeshTaskRecord {
        MeshTaskRecord {
            consumes: serde_json::json!({"prior_output": {}}),
            depends_on,
            ..task(id, &[], "first_claim")
        }
    }

    fn caps(names: &[&str]) -> Vec<Capability> {
        names.iter().map(|s| Capability::new(*s)).collect()
    }

    #[test]
    fn no_caps_required_always_eligible() {
        let tasks = vec![task("t1", &[], "first_claim")];
        let agent_caps = caps(&["code.edit"]);
        let eligible = filter_eligible(&tasks, &agent_caps);
        assert_eq!(eligible.len(), 1);
    }

    #[test]
    fn matching_caps_eligible() {
        let tasks = vec![task("t1", &["code.edit", "test.run"], "first_claim")];
        let agent_caps = caps(&["code.edit", "test.run", "code.read"]);
        let eligible = filter_eligible(&tasks, &agent_caps);
        assert_eq!(eligible.len(), 1);
    }

    #[test]
    fn missing_one_cap_not_eligible() {
        let tasks = vec![task("t1", &["code.edit", "test.run"], "first_claim")];
        let agent_caps = caps(&["code.edit"]); // missing test.run
        let eligible = filter_eligible(&tasks, &agent_caps);
        assert!(eligible.is_empty());
    }

    #[test]
    fn partial_match_across_tasks() {
        let tasks = vec![
            task("t1", &["gemini"], "first_claim"),
            task("t2", &["code.edit"], "first_claim"),
            task("t3", &[], "first_claim"),
        ];
        let agent_caps = caps(&["code.edit"]);
        let eligible = filter_eligible(&tasks, &agent_caps);
        // t1 needs gemini (missing), t2 and t3 match
        assert_eq!(eligible.len(), 2);
        assert!(eligible.iter().any(|t| t.id == "t2"));
        assert!(eligible.iter().any(|t| t.id == "t3"));
    }

    #[test]
    fn empty_task_list_returns_empty() {
        let eligible = filter_eligible(&[], &caps(&["code.edit"]));
        assert!(eligible.is_empty());
    }

    #[test]
    fn empty_agent_caps_only_matches_no_requirement_tasks() {
        let tasks = vec![
            task("t1", &["code.edit"], "first_claim"),
            task("t2", &[], "first_claim"),
        ];
        let eligible = filter_eligible(&tasks, &[]);
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].id, "t2");
    }

    #[test]
    fn consumes_with_depends_on_is_eligible() {
        let tasks = vec![task_with_consumes("t1", vec!["dep-task".into()])];
        let eligible = filter_eligible(&tasks, &[]);
        assert_eq!(eligible.len(), 1, "task with consumes + depends_on should be eligible");
    }

    #[test]
    fn consumes_without_depends_on_is_rejected() {
        // Misconfigured: declares consumes but has no depends_on — inputs can't exist yet.
        let tasks = vec![task_with_consumes("t1", vec![])];
        let eligible = filter_eligible(&tasks, &[]);
        assert!(eligible.is_empty(), "misconfigured task (consumes with no depends_on) should be skipped");
    }

    #[test]
    fn empty_consumes_object_is_fine_without_depends_on() {
        let tasks = vec![task("t1", &[], "first_claim")]; // consumes: {}
        let eligible = filter_eligible(&tasks, &[]);
        assert_eq!(eligible.len(), 1, "empty consumes should not trigger the gate");
    }
}
