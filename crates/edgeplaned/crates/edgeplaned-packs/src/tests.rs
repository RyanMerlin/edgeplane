use crate::manifest::{Backend, RiskLevel};
use crate::policy::{
    evaluate_policy, Decision, ExecutionContext, PolicyAction, PolicyBundle, PolicyRule,
};
use crate::registry::PackRegistry;

fn ctx() -> ExecutionContext {
    ExecutionContext {
        profile: "base".to_string(),
        env: "default".to_string(),
    }
}

#[test]
fn load_builtin_succeeds() {
    let registry = PackRegistry::load_builtin().expect("load_builtin should succeed");
    assert_eq!(registry.pack_count(), 14, "expected 14 builtin packs");
}

#[test]
fn total_capability_count_nonzero() {
    let registry = PackRegistry::load_builtin().unwrap();
    let all = registry.capabilities(None);
    assert_eq!(
        all.len(),
        71,
        "expected exactly 71 builtin capabilities; update this if packs change"
    );
}

#[test]
fn get_by_full_name_kubectl_get_pods() {
    let registry = PackRegistry::load_builtin().unwrap();
    // Full name = "<pack>.<cap.name>" where cap.name comes from the YAML name field.
    // The kubectl-observe pack's capability YAML has: name: kubectl.get-pods
    // So the full registry key is "kubectl-observe.kubectl.get-pods".
    let cap = registry
        .get_by_full_name("kubectl-observe.kubectl.get-pods")
        .expect("kubectl-observe.kubectl.get-pods should exist");
    assert_eq!(cap.name, "kubectl.get-pods");
    assert!(matches!(cap.risk, RiskLevel::Low));
    match &cap.backend {
        Backend::Subprocess { command, .. } => assert_eq!(command, "kubectl"),
        _ => panic!("expected subprocess backend"),
    }
}

#[test]
fn get_by_pack_and_cap_name() {
    let registry = PackRegistry::load_builtin().unwrap();
    let cap = registry
        .get("kubectl-observe", "kubectl.get-namespaces")
        .expect("kubectl.get-namespaces should exist");
    assert_eq!(cap.name, "kubectl.get-namespaces");
}

#[test]
fn capabilities_kubernetes_tag_filter() {
    let registry = PackRegistry::load_builtin().unwrap();
    let k8s_caps = registry.capabilities(Some("kubernetes"));
    assert!(
        !k8s_caps.is_empty(),
        "should find kubernetes-tagged capabilities"
    );
    // kubectl-observe, helm-observe, argocd-observe are all tagged kubernetes
    for summary in &k8s_caps {
        assert!(
            summary.tags.contains(&"kubernetes".to_string()),
            "{} should have kubernetes tag",
            summary.full_name
        );
    }
}

#[test]
fn capabilities_none_returns_all() {
    let registry = PackRegistry::load_builtin().unwrap();
    let all = registry.capabilities(None);
    assert!(!all.is_empty());
}

#[test]
fn capabilities_readonly_tag_filter() {
    let registry = PackRegistry::load_builtin().unwrap();
    let readonly_caps = registry.capabilities(Some("readonly"));
    assert!(
        !readonly_caps.is_empty(),
        "should find readonly-tagged capabilities"
    );
    for summary in &readonly_caps {
        assert!(summary.tags.contains(&"readonly".to_string()));
    }
}

#[test]
fn evaluate_allow_all_returns_allow() {
    let registry = PackRegistry::load_builtin().unwrap();
    let cap = registry
        .get_by_full_name("kubectl-observe.kubectl.get-pods")
        .unwrap();
    let decision = evaluate_policy(&PolicyBundle::allow_all(), &ctx(), cap);
    assert!(matches!(decision, Decision::Allow));
}

#[test]
fn evaluate_default_deny_returns_deny() {
    let registry = PackRegistry::load_builtin().unwrap();
    let cap = registry
        .get_by_full_name("kubectl-observe.kubectl.get-pods")
        .unwrap();
    let decision = evaluate_policy(&PolicyBundle::default(), &ctx(), cap);
    assert!(matches!(decision, Decision::Deny { .. }));
}

#[test]
fn evaluate_deny_rule_for_high_risk() {
    let _registry = PackRegistry::load_builtin().unwrap();
    // Use any low-risk cap as a stand-in and construct a synthetic cap with High risk.
    use crate::manifest::CapabilityManifest;
    use edgeplaned_sandbox::types::SideEffectClass;

    let cap = CapabilityManifest {
        name: "test.dangerous-op".to_string(),
        version: 1,
        description: Some("synthetic high-risk cap for policy test".to_string()),
        backend: Backend::Builtin {
            name: "noop".to_string(),
        },
        risk: RiskLevel::Critical,
        side_effect_class: SideEffectClass::Mutating,
        sandbox_profile: None,
        input_schema: serde_json::json!({}),
        credentials: vec![],
        tags: vec!["test".to_string()],
    };

    // A deny rule targeting the "critical" risk level.
    let mut policy = PolicyBundle::allow_all();
    policy.rules.push(PolicyRule {
        risk: Some("critical".to_string()),
        action: PolicyAction::Deny,
        reason: Some("critical ops are not allowed".to_string()),
        ..Default::default()
    });

    let decision = evaluate_policy(&policy, &ctx(), &cap);
    assert!(
        matches!(decision, Decision::Deny { .. }),
        "critical risk should be denied by the policy rule"
    );
}

#[test]
fn evaluate_explicit_allow_overrides_default_deny() {
    let registry = PackRegistry::load_builtin().unwrap();
    let cap = registry
        .get_by_full_name("kubectl-observe.kubectl.get-pods")
        .unwrap();

    let mut policy = PolicyBundle::default(); // default: Deny
    policy.rules.push(PolicyRule {
        capability: Some("kubectl.get-pods".to_string()),
        action: PolicyAction::Allow,
        ..Default::default()
    });

    let decision = evaluate_policy(&policy, &ctx(), cap);
    assert!(matches!(decision, Decision::Allow));
}

#[test]
fn get_gcloud_aiplatform_capability() {
    let registry = PackRegistry::load_builtin().unwrap();
    let cap = registry
        .get_by_full_name("gcloud-aiplatform.gcloud.aiplatform.datasets.list")
        .expect("gcloud-aiplatform.gcloud.aiplatform.datasets.list should exist");
    assert_eq!(cap.risk, RiskLevel::Low);
}

#[test]
fn builtin_packs_have_expected_tags() {
    let registry = PackRegistry::load_builtin().unwrap();

    let cloud_caps = registry.capabilities(Some("cloud"));
    assert!(
        !cloud_caps.is_empty(),
        "cloud tag should match aws/gcloud/az/terraform packs"
    );

    let container_caps = registry.capabilities(Some("container"));
    assert!(
        !container_caps.is_empty(),
        "container tag should match docker/podman/incus packs"
    );
}
