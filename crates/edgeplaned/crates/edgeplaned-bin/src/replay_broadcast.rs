//! Bounded replay buffer fronting a `tokio::sync::broadcast::Sender`.
//!
//! New viewers that attach mid-session need to see what the agent was
//! doing before they arrived — without this, reconnecting to a running
//! ACP session shows an empty pane until the next event ticks in.
//! Phase C exit criterion #6 of the tmux-retirement plan
//! (docs/plans/2026-05-11-retire-tmux-via-acp-persistent-sessions.md)
//! makes this explicit.
//!
//! ## Linearisation
//!
//! `send` and `subscribe_with_replay` are atomic w.r.t. each other —
//! both take the same lock briefly. A notification is either:
//!   1. In the replay snapshot a new viewer just received, OR
//!   2. Delivered live on the broadcast channel after they subscribed,
//! never both. This avoids the classic race where a subscriber both
//! replays an item and then sees it again live.
//!
//! ## Capacity
//!
//! The buffer is a fixed-size ring. Older entries are dropped when full.
//! Tuned to "enough to make a refresh look continuous" rather than
//! "complete history since process start"; full history requires the
//! agent (claude-agent-acp) to expose session resume, which is a
//! separate concern.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

/// Combined replay buffer + broadcast channel. Cheap to clone (`Arc`
/// inside). `T` must be `Clone` because both the buffer copy and the
/// broadcast send retain values.
pub struct ReplayBroadcast<T: Clone + Send + 'static> {
    inner: Arc<Mutex<Inner<T>>>,
}

struct Inner<T: Clone + Send + 'static> {
    buf: VecDeque<T>,
    capacity: usize,
    tx: broadcast::Sender<T>,
}

impl<T: Clone + Send + 'static> Clone for ReplayBroadcast<T> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<T: Clone + Send + 'static> ReplayBroadcast<T> {
    /// `replay_capacity` items are kept in the snapshot ring. `bc_capacity`
    /// is the size of the underlying broadcast channel; slow subscribers
    /// see `RecvError::Lagged` when they fall behind that many items.
    pub fn new(replay_capacity: usize, bc_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(bc_capacity);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                buf: VecDeque::with_capacity(replay_capacity),
                capacity: replay_capacity,
                tx,
            })),
        }
    }

    /// Push an item: append to the replay ring (evict oldest if full)
    /// and broadcast to live subscribers. Atomic — a `subscribe_with_replay`
    /// observing the snapshot afterwards will not also receive this item
    /// on its receiver.
    pub fn send(&self, item: T) {
        let mut inner = self.inner.lock().expect("replay_broadcast lock");
        if inner.capacity > 0 {
            if inner.buf.len() >= inner.capacity {
                inner.buf.pop_front();
            }
            inner.buf.push_back(item.clone());
        }
        // `send` returns Err iff there are zero subscribers; that's fine
        // — drop silently. Future viewers still get the item via replay.
        let _ = inner.tx.send(item);
    }

    /// Take a snapshot of the replay buffer and subscribe to live
    /// updates. Returns `(snapshot, receiver)` — caller drains snapshot
    /// first, then forwards receiver items, with no overlap.
    pub fn subscribe_with_replay(&self) -> (Vec<T>, broadcast::Receiver<T>) {
        let inner = self.inner.lock().expect("replay_broadcast lock");
        let rx = inner.tx.subscribe();
        let snapshot: Vec<T> = inner.buf.iter().cloned().collect();
        (snapshot, rx)
    }

    /// Subscribe without taking a replay snapshot. Used by callers that
    /// already know they only want live updates (e.g. the supervisor's
    /// own loop that's already authoritative for replay state).
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        let inner = self.inner.lock().expect("replay_broadcast lock");
        inner.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replay_returns_recent_items() {
        let rb: ReplayBroadcast<i32> = ReplayBroadcast::new(4, 16);
        rb.send(1);
        rb.send(2);
        rb.send(3);
        let (snap, _rx) = rb.subscribe_with_replay();
        assert_eq!(snap, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn replay_buffer_evicts_oldest() {
        let rb: ReplayBroadcast<i32> = ReplayBroadcast::new(2, 16);
        rb.send(1);
        rb.send(2);
        rb.send(3); // evicts 1
        rb.send(4); // evicts 2
        let (snap, _rx) = rb.subscribe_with_replay();
        assert_eq!(snap, vec![3, 4]);
    }

    #[tokio::test]
    async fn live_subscribers_do_not_see_pre_subscribe_items() {
        let rb: ReplayBroadcast<i32> = ReplayBroadcast::new(8, 16);
        rb.send(1); // before subscribe
        let mut rx = rb.subscribe();
        rb.send(2); // after subscribe
        rb.send(3);
        assert_eq!(rx.recv().await.unwrap(), 2);
        assert_eq!(rx.recv().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn replay_and_live_have_no_overlap() {
        // The whole point: an item is in the snapshot OR on the
        // receiver, never both. Take a snapshot, then send more items
        // and confirm those (and only those) come down the receiver.
        let rb: ReplayBroadcast<i32> = ReplayBroadcast::new(8, 16);
        rb.send(1);
        rb.send(2);
        let (snap, mut rx) = rb.subscribe_with_replay();
        assert_eq!(snap, vec![1, 2]);
        rb.send(3);
        rb.send(4);
        assert_eq!(rx.recv().await.unwrap(), 3);
        assert_eq!(rx.recv().await.unwrap(), 4);
    }

    #[tokio::test]
    async fn zero_capacity_means_replay_disabled_but_live_still_works() {
        let rb: ReplayBroadcast<i32> = ReplayBroadcast::new(0, 16);
        let mut rx = rb.subscribe();
        rb.send(1);
        rb.send(2);
        assert_eq!(rx.recv().await.unwrap(), 1);
        assert_eq!(rx.recv().await.unwrap(), 2);
        let (snap, _rx) = rb.subscribe_with_replay();
        assert!(snap.is_empty(), "zero-capacity buffer should never store");
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let rb: ReplayBroadcast<i32> = ReplayBroadcast::new(4, 16);
        let cloned = rb.clone();
        rb.send(1);
        let (snap, _) = cloned.subscribe_with_replay();
        assert_eq!(snap, vec![1]);
    }
}
