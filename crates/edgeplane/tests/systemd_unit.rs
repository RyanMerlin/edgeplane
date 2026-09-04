//! Invariants for the shipped system unit. Guards the calibrated-hardening
//! contract (Axis 2 spec §5.1) so a future edit can't silently re-root the
//! daemon or break the unprivileged-userns sandbox jail.

const UNIT: &str = include_str!("../systemd/edgeplaned.service");

/// Active (non-comment, non-blank) directive lines.
fn active(unit: &str) -> Vec<&str> {
    unit.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

#[test]
fn runs_edgeplaned_as_dedicated_nonroot_user() {
    let dirs = active(UNIT);
    assert!(dirs.contains(&"ExecStart=/usr/local/bin/edgeplaned run"));
    assert!(dirs.contains(&"User=edgeplane"));
    assert!(!dirs.iter().any(|l| l.starts_with("User=root")));
}

#[test]
fn uses_ep_home_single_root() {
    let dirs = active(UNIT);
    assert!(dirs.contains(&"Environment=EP_HOME=/var/lib/edgeplane"));
    assert!(dirs.contains(&"StateDirectory=edgeplane"));
}

#[test]
fn has_calibrated_hardening() {
    let dirs = active(UNIT);
    for d in [
        "NoNewPrivileges=yes",
        "ProtectSystem=strict",
        "ProtectHome=yes",
    ] {
        assert!(dirs.contains(&d), "missing {d}");
    }
}

#[test]
fn preserves_userns_jail() {
    // These would break unshare(CLONE_NEWUSER|...). They may appear in COMMENTS
    // (documenting why they're forbidden) but never as active directives.
    let dirs = active(UNIT);
    for forbidden in ["RestrictNamespaces", "PrivateUsers", "SystemCallFilter"] {
        assert!(
            !dirs.iter().any(|l| l.starts_with(forbidden)),
            "forbidden active directive: {forbidden}"
        );
    }
}
