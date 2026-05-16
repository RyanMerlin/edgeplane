use serde::{Deserialize, Serialize};

/// Filesystem access policy for the jailed worker.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsPolicy {
    /// Additional read-only bind mounts (host paths) to expose inside the jail. The CLI binary
    /// and its dynamic libraries are always bound automatically.
    #[serde(default)]
    pub extra_ro_bind: Vec<String>,
    /// Additional read-write bind mounts (host paths). Use sparingly.
    #[serde(default)]
    pub extra_rw_bind: Vec<String>,
    /// If true, expose /tmp from the host (default: isolated tmpfs /tmp).
    #[serde(default)]
    pub share_host_tmp: bool,
}

/// Network egress rules for the jailed worker.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicy {
    /// Explicit host:port allowlist for outbound TCP connections. If empty, all network is denied.
    #[serde(default)]
    pub egress_allowlist: Vec<String>,
}

/// cgroup v2 resource constraints for the jailed worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CgroupLimits {
    /// Maximum RSS in MiB. Default 512.
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u64,
    /// Maximum number of PIDs inside the worker. Default 64.
    #[serde(default = "default_max_pids")]
    pub max_pids: u64,
    /// CPU weight (relative, range 1–10000). Default 100.
    #[serde(default = "default_cpu_weight")]
    pub cpu_weight: u64,
}

impl Default for CgroupLimits {
    fn default() -> Self {
        CgroupLimits {
            memory_mib: default_memory_mib(),
            max_pids: default_max_pids(),
            cpu_weight: default_cpu_weight(),
        }
    }
}

fn default_memory_mib() -> u64 { 512 }
fn default_max_pids() -> u64 { 64 }
fn default_cpu_weight() -> u64 { 100 }

/// Describes the side-effect class of a capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SideEffectClass {
    #[serde(rename = "none")]
    #[default]
    None,
    #[serde(rename = "readOnly", alias = "read_only", alias = "readonly")]
    ReadOnly,
    Additive,
    #[serde(alias = "write_remote", alias = "writeRemote")]
    Mutating,
    Destructive,
}
