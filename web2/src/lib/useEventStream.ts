/**
 * useEventStream — thin hook wrapper over the Zustand event-stream store.
 *
 * Responsibilities:
 *   1. Ensure the singleton EventSource is connected on first mount.
 *   2. Guard against React 19 StrictMode double-mount: connect() is idempotent
 *      on the store (checks `status !== 'closed'`), so the second mount's
 *      connect() call is a no-op.
 *   3. Return the slice of store state that consumers need.
 *
 * Disconnect policy: mirrors the Svelte behavior — the stream runs for the
 * lifetime of the app, not just while a single component is mounted. We do NOT
 * disconnect on unmount. This is intentional: the feed and matrix pages share
 * the same underlying connection; navigating away from feed and then back
 * should not cause a reconnect cycle.
 *
 * If you need explicit lifecycle management (e.g., tests), call
 * `useEventStreamStore.getState().disconnect()` directly.
 */

import { useEventStreamStore } from '@/stores/eventStream';
import { useEffect } from 'react';

export function useEventStream() {
  const connect = useEventStreamStore((s) => s.connect);
  const events = useEventStreamStore((s) => s.events);
  const status = useEventStreamStore((s) => s.status);
  const lastError = useEventStreamStore((s) => s.lastError);
  const rateLimit = useEventStreamStore((s) => s.rateLimit);
  const reconnectCount = useEventStreamStore((s) => s.reconnectCount);
  const reconnectDelay = useEventStreamStore((s) => s.reconnectDelay);
  const messagesReceived = useEventStreamStore((s) => s.messagesReceived);
  const clearEvents = useEventStreamStore((s) => s.clearEvents);

  useEffect(() => {
    // connect() is idempotent — safe to call on every mount including StrictMode's
    // second mount. The store guards with `if (status !== 'closed') return`.
    connect();
    // Intentionally no cleanup: the stream outlives individual component mounts.
  }, [connect]);

  return {
    events,
    status,
    lastError,
    rateLimit,
    reconnectCount,
    reconnectDelay,
    messagesReceived,
    clearEvents,
    isLive: status === 'open',
  };
}
