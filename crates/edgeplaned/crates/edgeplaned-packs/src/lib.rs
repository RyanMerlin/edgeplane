pub mod builtin;
pub mod error;
pub mod manifest;
pub mod policy;
pub mod registry;

pub use error::{PacksError, Result};
pub use manifest::{Backend, CapabilityManifest, PackManifest, RiskLevel, SandboxProfile};
pub use policy::{
    evaluate_policy, Decision, ExecutionContext, PolicyAction, PolicyBundle, PolicyRule,
};
pub use registry::{CapabilitySummary, PackRegistry};

#[cfg(test)]
mod tests;
