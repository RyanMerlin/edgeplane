use serde::{Deserialize, Serialize};
use crate::manifest::{CapabilityManifest, RiskLevel};

// ─── Policy bundle ────────────────────────────────────────────────────────────

fn default_deny() -> PolicyAction {
    PolicyAction::Deny
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyBundle {
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
    /// Fallback action when no rule matches. Defaults to Deny (fail-closed).
    #[serde(default = "default_deny")]
    pub default_action: PolicyAction,
}

impl Default for PolicyBundle {
    fn default() -> Self {
        PolicyBundle {
            rules: vec![],
            default_action: PolicyAction::Deny,
        }
    }
}

impl PolicyBundle {
    /// Construct a bundle that allows all unmatched capabilities.
    pub fn allow_all() -> Self {
        PolicyBundle {
            rules: vec![],
            default_action: PolicyAction::Allow,
        }
    }
}

/// A rule that matches capability executions. All fields are optional filters;
/// `None` means "match any value" (wildcard). The first matching rule wins.
/// A rule with all fields `None` matches every capability for every context.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    #[serde(default)]
    pub capability: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
    /// Match by risk level string: "low", "medium", "high", "critical".
    #[serde(default)]
    pub risk: Option<String>,
    pub action: PolicyAction,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyAction {
    #[default]
    Allow,
    Deny,
    RequireApproval,
}

// ─── Execution context ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub profile: String,
    pub env: String,
}

// ─── Decision ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Decision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

// ─── Evaluation ───────────────────────────────────────────────────────────────

pub fn evaluate_policy(
    policy: &PolicyBundle,
    ctx: &ExecutionContext,
    cap: &CapabilityManifest,
) -> Decision {
    for rule in &policy.rules {
        if !rule_matches(rule, ctx, cap) {
            continue;
        }
        let reason = rule
            .reason
            .clone()
            .unwrap_or_else(|| format!("matched policy rule for {}", cap.name));
        return match rule.action {
            PolicyAction::Allow => Decision::Allow,
            PolicyAction::Deny => Decision::Deny { reason },
            PolicyAction::RequireApproval => Decision::RequireApproval { reason },
        };
    }
    match &policy.default_action {
        PolicyAction::Allow => match cap.risk {
            RiskLevel::Low | RiskLevel::Medium => Decision::Allow,
            RiskLevel::High | RiskLevel::Critical => Decision::RequireApproval {
                reason: format!(
                    "{} risk capability requires approval",
                    cap.risk
                ),
            },
        },
        PolicyAction::Deny => Decision::Deny {
            reason: format!(
                "no policy rule matched '{}'; default action is deny",
                cap.name
            ),
        },
        PolicyAction::RequireApproval => Decision::RequireApproval {
            reason: format!(
                "no policy rule matched '{}'; default action is require_approval",
                cap.name
            ),
        },
    }
}

fn rule_matches(rule: &PolicyRule, ctx: &ExecutionContext, cap: &CapabilityManifest) -> bool {
    if let Some(ref name) = rule.capability {
        if name != &cap.name {
            return false;
        }
    }
    if let Some(ref profile) = rule.profile {
        if profile != &ctx.profile {
            return false;
        }
    }
    if let Some(ref env) = rule.env {
        if env != &ctx.env {
            return false;
        }
    }
    if let Some(ref risk) = rule.risk {
        if risk != &cap.risk.to_string() {
            return false;
        }
    }
    true
}
