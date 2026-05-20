use std::collections::HashMap;
use crate::error::Result;
use crate::manifest::{CapabilityManifest, RiskLevel};

/// Summary view of a single capability for listing.
#[derive(Debug, Clone)]
pub struct CapabilitySummary {
    pub full_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub risk: RiskLevel,
}

/// Registry of all installed packs and their capabilities.
///
/// Capabilities are keyed by full name `"<pack>.<cap>"`, e.g. `"kubectl-observe.list-pods"`.
pub struct PackRegistry {
    /// Maps full capability name → capability manifest.
    capabilities: HashMap<String, CapabilityManifest>,
    /// Maps pack name → list of full capability names in that pack.
    packs: HashMap<String, Vec<String>>,
}

impl PackRegistry {
    pub(crate) fn new() -> Self {
        PackRegistry {
            capabilities: HashMap::new(),
            packs: HashMap::new(),
        }
    }

    /// Load the registry from the built-in embedded pack manifests.
    pub fn load_builtin() -> Result<Self> {
        crate::builtin::load_builtin_registry()
    }

    /// Insert a capability into the registry.
    ///
    /// `pack_name` is the directory/pack name (e.g. `"kubectl-observe"`).
    /// `cap` has its `name` field set to the cap's own name (e.g. `"kubectl.get-pods"`).
    pub(crate) fn insert(&mut self, pack_name: &str, mut cap: CapabilityManifest, tags: Vec<String>) {
        cap.tags = tags;
        let full_name = format!("{}.{}", pack_name, cap.name);
        self.packs
            .entry(pack_name.to_string())
            .or_default()
            .push(full_name.clone());
        self.capabilities.insert(full_name, cap);
    }

    /// Return all capabilities, optionally filtered by tag.
    pub fn capabilities(&self, tag: Option<&str>) -> Vec<CapabilitySummary> {
        let mut summaries: Vec<CapabilitySummary> = self
            .capabilities
            .iter()
            .filter(|(_, cap)| {
                if let Some(t) = tag {
                    cap.tags.iter().any(|tag| tag == t)
                } else {
                    true
                }
            })
            .map(|(full_name, cap)| CapabilitySummary {
                full_name: full_name.clone(),
                description: cap.description.clone().unwrap_or_default(),
                tags: cap.tags.clone(),
                risk: cap.risk.clone(),
            })
            .collect();
        summaries.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        summaries
    }

    /// Look up a capability by pack name and capability name.
    pub fn get(&self, pack_name: &str, cap_name: &str) -> Option<&CapabilityManifest> {
        let full_name = format!("{}.{}", pack_name, cap_name);
        self.capabilities.get(&full_name)
    }

    /// Look up a capability by full name (`"pack-name.cap-name"`).
    pub fn get_by_full_name(&self, full_name: &str) -> Option<&CapabilityManifest> {
        self.capabilities.get(full_name)
    }

    /// Number of packs loaded.
    pub fn pack_count(&self) -> usize {
        self.packs.len()
    }

    /// Number of capabilities loaded.
    pub fn capability_count(&self) -> usize {
        self.capabilities.len()
    }
}
