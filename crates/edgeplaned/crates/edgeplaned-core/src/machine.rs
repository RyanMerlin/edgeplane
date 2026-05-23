/// Auto-detect host machine information for agent enrollment.
///
/// This is called once at daemon startup and included in every agent enrollment
/// so the backend roster can tell other agents what hardware/tools are available.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MachineInfo {
    pub hostname: String,
    pub os: String,
    pub cpu_cores: u32,
    pub ram_gb: f64,
    pub disk_free_gb: f64,
    pub working_dir: String,
    pub installed_tools: Vec<ToolInfo>,
    /// Tailscale IPv4 address (e.g. "100.x.x.x"), None if not installed or not connected.
    #[serde(default)]
    pub tailscale_ip: Option<String>,
    /// Tailscale FQDN (e.g. "epyc.my-tailnet.ts.net"), None if unavailable.
    #[serde(default)]
    pub tailscale_fqdn: Option<String>,
}

impl MachineInfo {
    /// Detect current host info. Non-fatal: missing values default to zero/empty.
    pub fn detect(work_dir: &std::path::Path) -> Self {
        let (tailscale_ip, tailscale_fqdn) = detect_tailscale();
        MachineInfo {
            hostname: hostname(),
            os: os_string(),
            cpu_cores: cpu_cores(),
            ram_gb: ram_gb(),
            disk_free_gb: disk_free_gb(work_dir),
            working_dir: work_dir.display().to_string(),
            installed_tools: detect_tools(),
            tailscale_ip,
            tailscale_fqdn,
        }
    }
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into()))
}

fn os_string() -> String {
    // Try /etc/os-release first (Linux), fall back to uname.
    if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
        let pretty = contents
            .lines()
            .find(|l| l.starts_with("PRETTY_NAME="))
            .and_then(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string());
        if let Some(name) = pretty {
            // Append kernel version.
            let kernel = uname_r();
            return if kernel.is_empty() { name } else { format!("{name} ({kernel})") };
        }
    }
    // macOS / fallback
    let sysname = std::process::Command::new("uname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let release = uname_r();
    if release.is_empty() { sysname } else { format!("{sysname} {release}") }
}

fn uname_r() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn cpu_cores() -> u32 {
    // Try nproc, then /proc/cpuinfo, then sysctl (macOS).
    if let Ok(out) = std::process::Command::new("nproc").output() {
        if let Ok(s) = String::from_utf8(out.stdout) {
            if let Ok(n) = s.trim().parse::<u32>() {
                return n;
            }
        }
    }
    if let Ok(contents) = std::fs::read_to_string("/proc/cpuinfo") {
        let count = contents.lines().filter(|l| l.starts_with("processor")).count();
        if count > 0 {
            return count as u32;
        }
    }
    0
}

fn ram_gb() -> f64 {
    // Linux: /proc/meminfo
    if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
        for line in contents.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                return (kb as f64) / 1_048_576.0; // kB → GB
            }
        }
    }
    // macOS: sysctl
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
    {
        if let Ok(s) = String::from_utf8(out.stdout) {
            if let Ok(bytes) = s.trim().parse::<u64>() {
                return (bytes as f64) / 1_073_741_824.0;
            }
        }
    }
    0.0
}

fn disk_free_gb(path: &std::path::Path) -> f64 {
    // Use `df -B1` for byte-level output; take the available column.
    let out = std::process::Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(path)
        .output();
    if let Ok(o) = out {
        if let Ok(s) = String::from_utf8(o.stdout) {
            // Output is header + value
            let bytes: u64 = s
                .lines()
                .nth(1)
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(0);
            return (bytes as f64) / 1_073_741_824.0;
        }
    }
    0.0
}

/// Detect Tailscale IPv4 address and FQDN. Both are optional — returns (None, None)
/// if Tailscale is not installed, not connected, or any command fails.
fn detect_tailscale() -> (Option<String>, Option<String>) {
    // Step 1: get IPv4 address via `tailscale ip --4`
    let ip = std::process::Command::new("tailscale")
        .args(["ip", "--4"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty());

    // Step 2: get FQDN via `tailscale status --json` → .Self.DNSName (strip trailing dot)
    let fqdn = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| {
            v.get("Self")
                .and_then(|s| s.get("DNSName"))
                .and_then(|n| n.as_str())
                .map(|s| s.trim_end_matches('.').to_string())
        })
        .filter(|s| !s.is_empty());

    (ip, fqdn)
}

/// Probe a set of well-known CLIs and capture their versions.
fn detect_tools() -> Vec<ToolInfo> {
    let probes: &[(&str, &[&str])] = &[
        ("claude",  &["--version"]),
        ("codex",   &["--version"]),
        ("gemini",  &["version"]),
        ("git",     &["--version"]),
        ("cargo",   &["--version"]),
        ("rustc",   &["--version"]),
        ("python3", &["--version"]),
        ("node",    &["--version"]),
        ("docker",     &["--version"]),
        ("kubectl",    &["version", "--client", "--short"]),
        ("tailscale",  &["version"]),
    ];

    probes
        .iter()
        .filter_map(|(name, args)| {
            let out = std::process::Command::new(name)
                .args(*args)
                .output()
                .ok()?;
            // Some tools write version to stderr (e.g. git on some systems).
            let raw = if out.stdout.is_empty() { out.stderr } else { out.stdout };
            let version = String::from_utf8_lossy(&raw)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if version.is_empty() {
                return None;
            }
            Some(ToolInfo {
                name: name.to_string(),
                version,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_does_not_panic() {
        let dir = std::path::Path::new(".");
        let info = MachineInfo::detect(dir);
        // Hostname should be non-empty on any CI machine.
        assert!(!info.hostname.is_empty(), "hostname should be detectable");
        // OS should be non-empty.
        assert!(!info.os.is_empty(), "os should be detectable");
    }

    #[test]
    fn serializes_to_json() {
        let info = MachineInfo {
            hostname: "test-host".into(),
            os: "Linux 6.0".into(),
            cpu_cores: 8,
            ram_gb: 16.0,
            disk_free_gb: 100.0,
            working_dir: "/tmp/work".into(),
            installed_tools: vec![ToolInfo {
                name: "git".into(),
                version: "git version 2.47.0".into(),
            }],
            tailscale_ip: None,
            tailscale_fqdn: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("test-host"));
        assert!(json.contains("git"));
    }

    #[test]
    fn serializes_tailscale_fields() {
        let info = MachineInfo {
            hostname: "epyc".into(),
            os: "Linux".into(),
            cpu_cores: 4,
            ram_gb: 8.0,
            disk_free_gb: 50.0,
            working_dir: "/tmp".into(),
            installed_tools: vec![],
            tailscale_ip: Some("100.64.0.1".into()),
            tailscale_fqdn: Some("epyc.example.ts.net".into()),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("100.64.0.1"));
        assert!(json.contains("epyc.example.ts.net"));
    }
}
