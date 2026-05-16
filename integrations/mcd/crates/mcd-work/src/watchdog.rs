use chrono::{DateTime, Utc};
use tokio::sync::watch;

/// The policy applied when the daemon loses backend connectivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflinePolicy {
    /// Kill all mutable work immediately.
    Strict,
    /// Allow read/monitor ops; block writes and outbound calls.
    SafeReadonly,
    /// Continue whitelisted service agents until an autonomy TTL expires,
    /// then fall back to Strict.
    Autonomous { max_ttl_secs: u64 },
}

/// The current connectivity state from the watchdog's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityState {
    Connected,
    Degraded { since: DateTime<Utc> },
    Offline { since: DateTime<Utc> },
}

/// Watchdog monitors task lease deadlines and backend connectivity.
///
/// When a lease expires without renewal (because the backend is unreachable),
/// the watchdog sends a `WatchdogEvent::LeaseExpired` to the supervision loop,
/// which then enforces the configured offline policy.
pub struct Watchdog {
    policy: OfflinePolicy,
    grace_secs: u64,
    state_tx: watch::Sender<ConnectivityState>,
    pub state_rx: watch::Receiver<ConnectivityState>,
}

impl Watchdog {
    pub fn new(policy: OfflinePolicy, grace_secs: u64) -> Self {
        let (tx, rx) = watch::channel(ConnectivityState::Connected);
        Watchdog {
            policy,
            grace_secs,
            state_tx: tx,
            state_rx: rx,
        }
    }

    pub fn record_heartbeat_success(&self) {
        let _ = self.state_tx.send(ConnectivityState::Connected);
    }

    pub fn record_heartbeat_failure(&self) {
        let current = *self.state_rx.borrow();
        match current {
            ConnectivityState::Connected => {
                let _ = self
                    .state_tx
                    .send(ConnectivityState::Degraded { since: Utc::now() });
            }
            ConnectivityState::Degraded { since } => {
                let elapsed = (Utc::now() - since).num_seconds().unsigned_abs();
                if elapsed >= self.grace_secs {
                    let _ = self
                        .state_tx
                        .send(ConnectivityState::Offline { since });
                }
            }
            ConnectivityState::Offline { .. } => {}
        }
    }

    pub fn policy(&self) -> OfflinePolicy {
        self.policy
    }

    pub fn is_offline(&self) -> bool {
        matches!(*self.state_rx.borrow(), ConnectivityState::Offline { .. })
    }

    pub fn state(&self) -> ConnectivityState {
        *self.state_rx.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_connected() {
        let wd = Watchdog::new(OfflinePolicy::Strict, 30);
        assert_eq!(wd.state(), ConnectivityState::Connected);
        assert!(!wd.is_offline());
    }

    #[test]
    fn single_failure_transitions_to_degraded() {
        let wd = Watchdog::new(OfflinePolicy::Strict, 30);
        wd.record_heartbeat_failure();
        assert!(matches!(wd.state(), ConnectivityState::Degraded { .. }));
        assert!(!wd.is_offline());
    }

    #[test]
    fn success_after_failure_resets_to_connected() {
        let wd = Watchdog::new(OfflinePolicy::Strict, 30);
        wd.record_heartbeat_failure();
        wd.record_heartbeat_success();
        assert_eq!(wd.state(), ConnectivityState::Connected);
    }

    #[test]
    fn failure_past_grace_transitions_to_offline() {
        // grace = 0 means any degraded failure immediately goes offline
        let wd = Watchdog::new(OfflinePolicy::Strict, 0);
        wd.record_heartbeat_failure(); // → Degraded
        wd.record_heartbeat_failure(); // elapsed >= 0 → Offline
        assert!(wd.is_offline());
    }

    #[test]
    fn success_from_offline_resets_to_connected() {
        let wd = Watchdog::new(OfflinePolicy::Strict, 0);
        wd.record_heartbeat_failure();
        wd.record_heartbeat_failure();
        assert!(wd.is_offline());
        wd.record_heartbeat_success();
        assert_eq!(wd.state(), ConnectivityState::Connected);
        assert!(!wd.is_offline());
    }

    #[test]
    fn repeated_failures_while_offline_stay_offline() {
        let wd = Watchdog::new(OfflinePolicy::Strict, 0);
        wd.record_heartbeat_failure();
        wd.record_heartbeat_failure(); // → Offline
        wd.record_heartbeat_failure(); // stays Offline
        wd.record_heartbeat_failure();
        assert!(wd.is_offline());
    }

    #[test]
    fn policy_accessor() {
        let wd = Watchdog::new(OfflinePolicy::SafeReadonly, 60);
        assert_eq!(wd.policy(), OfflinePolicy::SafeReadonly);
    }

    #[test]
    fn autonomous_policy_stored_correctly() {
        let wd = Watchdog::new(OfflinePolicy::Autonomous { max_ttl_secs: 300 }, 30);
        assert!(matches!(wd.policy(), OfflinePolicy::Autonomous { max_ttl_secs: 300 }));
    }
}
