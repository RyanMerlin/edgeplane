use crate::error::{Result, SandboxError};

pub fn apply_sandbox(allowed_executables: &[String]) -> Result<()> {
    use landlock::{AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr};
    if allowed_executables.is_empty() { return Ok(()); }
    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::Execute)
        .map_err(|e| SandboxError::Sandbox(format!("landlock: {e}")))?
        .create()
        .map_err(|e| SandboxError::Sandbox(format!("landlock create: {e}")))?;
    for path in allowed_executables {
        let fd = PathFd::new(path).map_err(|e| SandboxError::Sandbox(format!("landlock path {path}: {e}")))?;
        ruleset = ruleset.add_rule(PathBeneath::new(fd, AccessFs::Execute))
            .map_err(|e| SandboxError::Sandbox(format!("landlock rule: {e}")))?;
    }
    ruleset.restrict_self().map_err(|e| SandboxError::Sandbox(format!("landlock restrict: {e}")))?;
    Ok(())
}

pub fn sandbox_enforced() -> bool { true }
