use crate::error::{PacksError, Result};
use crate::manifest::{CapabilityManifest, PackManifest};
use crate::registry::PackRegistry;

/// Mapping from pack name to derived tags.
fn pack_tags(pack_name: &str) -> Vec<String> {
    let tags: &[&str] = match pack_name {
        "kubectl-observe"    => &["kubernetes", "observe"],
        "gh-readonly"        => &["git", "github", "readonly"],
        "git-readonly"       => &["git", "readonly"],
        "gcloud-readonly"    => &["cloud", "gcloud", "readonly"],
        "docker-observe"     => &["container", "docker", "observe"],
        "helm-observe"       => &["kubernetes", "helm", "observe"],
        "argocd-observe"     => &["kubernetes", "argocd", "observe"],
        "aws-readonly"       => &["cloud", "aws", "readonly"],
        "az-readonly"        => &["cloud", "azure", "readonly"],
        "incus-readonly"     => &["container", "incus", "readonly"],
        "podman-observe"     => &["container", "podman", "observe"],
        "terraform-observe"  => &["cloud", "terraform", "observe"],
        "gcloud-aiplatform"  => &["cloud", "gcloud", "ai"],
        "base"               => &["base", "system"],
        _                    => &[],
    };
    tags.iter().map(|s| s.to_string()).collect()
}

macro_rules! pack_entry {
    ($pack:literal, [$($cap:literal),+ $(,)?]) => {
        PackEntry {
            name: $pack,
            pack_yaml: include_str!(concat!("../packs/", $pack, "/pack.yaml")),
            // Each tuple is (filename, content). Only `capabilities/` YAML files are loaded;
            // `profiles/` and `workflows/` directories are intentionally not loaded in this phase.
            capabilities: &[
                $(
                    (concat!($cap), include_str!(concat!("../packs/", $pack, "/capabilities/", $cap))),
                )+
            ],
        }
    };
}

struct PackEntry {
    name: &'static str,
    pack_yaml: &'static str,
    /// Each entry is `(filename, yaml_content)` so parse errors can name the exact file.
    capabilities: &'static [(&'static str, &'static str)],
}

static BUILTIN_PACKS: &[PackEntry] = &[
    pack_entry!("argocd-observe", [
        "argocd.app-diff.yaml",
        "argocd.app-get.yaml",
        "argocd.app-history.yaml",
        "argocd.app-list.yaml",
    ]),
    pack_entry!("aws-readonly", [
        "aws.ec2-instances.yaml",
        "aws.ecr-repos.yaml",
        "aws.ecs-clusters.yaml",
        "aws.lambda-list.yaml",
        "aws.s3-buckets.yaml",
        "aws.whoami.yaml",
    ]),
    pack_entry!("az-readonly", [
        "az.account-list.yaml",
        "az.acr-list.yaml",
        "az.aks-list.yaml",
        "az.group-list.yaml",
        "az.storage-accounts.yaml",
        "az.vm-list.yaml",
    ]),
    pack_entry!("base", [
        "system.date.yaml",
        "system.echo.yaml",
    ]),
    pack_entry!("docker-observe", [
        "docker.images.yaml",
        "docker.inspect.yaml",
        "docker.logs.yaml",
        "docker.network-list.yaml",
        "docker.ps.yaml",
        "docker.stats.yaml",
    ]),
    pack_entry!("gcloud-aiplatform", [
        "datasets-list.yaml",
        "endpoints-describe.yaml",
        "endpoints-list.yaml",
        "jobs-list.yaml",
        "models-describe.yaml",
        "models-list.yaml",
    ]),
    pack_entry!("gcloud-readonly", [
        "gcloud.compute-instances-list.yaml",
        "gcloud.container-clusters-list.yaml",
        "gcloud.functions-list.yaml",
        "gcloud.iam-service-accounts-list.yaml",
        "gcloud.list-projects.yaml",
        "gcloud.storage-buckets-list.yaml",
    ]),
    pack_entry!("gh-readonly", [
        "gh.issue-list.yaml",
        "gh.list-repos.yaml",
        "gh.pr-list.yaml",
        "gh.pr-view.yaml",
        "gh.workflow-list.yaml",
    ]),
    pack_entry!("git-readonly", [
        "git.branch-list.yaml",
        "git.diff.yaml",
        "git.log.yaml",
        "git.status.yaml",
    ]),
    pack_entry!("helm-observe", [
        "helm.get-values.yaml",
        "helm.history.yaml",
        "helm.list.yaml",
        "helm.status.yaml",
    ]),
    pack_entry!("incus-readonly", [
        "incus.image-list.yaml",
        "incus.info.yaml",
        "incus.list.yaml",
        "incus.snapshot-list.yaml",
    ]),
    pack_entry!("kubectl-observe", [
        "kubectl.describe-pod.yaml",
        "kubectl.get-deployments.yaml",
        "kubectl.get-events.yaml",
        "kubectl.get-namespaces.yaml",
        "kubectl.get-nodes.yaml",
        "kubectl.get-pods.yaml",
        "kubectl.get-services.yaml",
        "kubectl.logs.yaml",
    ]),
    pack_entry!("podman-observe", [
        "podman.images.yaml",
        "podman.inspect.yaml",
        "podman.logs.yaml",
        "podman.pod-list.yaml",
        "podman.ps.yaml",
    ]),
    pack_entry!("terraform-observe", [
        "terraform.output.yaml",
        "terraform.plan.yaml",
        "terraform.show.yaml",
        "terraform.state-list.yaml",
        "terraform.validate.yaml",
    ]),
];

pub fn load_builtin_registry() -> Result<PackRegistry> {
    let mut registry = PackRegistry::new();

    for entry in BUILTIN_PACKS {
        // Validate pack manifest parses (not stored in registry currently).
        let _pack: PackManifest = serde_yaml::from_str(entry.pack_yaml).map_err(|e| {
            PacksError::YamlParse {
                file: format!("{}/pack.yaml", entry.name),
                source: e,
            }
        })?;

        let tags = pack_tags(entry.name);

        for (cap_filename, cap_yaml) in entry.capabilities {
            let cap: CapabilityManifest =
                serde_yaml::from_str(cap_yaml).map_err(|e| PacksError::YamlParse {
                    file: format!("{}/capabilities/{}", entry.name, cap_filename),
                    source: e,
                })?;

            tracing::trace!(pack = entry.name, cap = %cap.name, "loaded builtin capability");
            registry.insert(entry.name, cap, tags.clone());
        }
    }

    Ok(registry)
}
