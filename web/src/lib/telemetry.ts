import { browser } from '$app/environment';
import { writable } from 'svelte/store';

type RateLimit = {
  limit: number;
  remaining: number;
  reset_at: string;
};

type MatrixEvent = {
  id?: string;
  event?: string;
  type?: string;
  domain_id?: string;
  mission_id?: string;
  agent_id?: string;
  status?: string;
  payload: any;
  rate_limit?: RateLimit;
  receivedAt: number;
};

export const matrixEvents = writable<MatrixEvent[]>([]);
export const matrixStatus = writable({
  connected: false,
  lastError: null as string | null,
  rateLimit: null as RateLimit | null,
  lastEventId: null as string | null
});

let eventSource: EventSource | null = null;
let reconnectTimeout = 1000;
let pausedUntil = 0;
let messagesReceived = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let streamStarted = false;

function clearReconnectTimer() {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
}

export function startMatrixStream() {
  if (!browser) return;
  // Prevent concurrent starts — only one stream at a time
  if (streamStarted && eventSource) return;
  if (Date.now() < pausedUntil) return;

  clearReconnectTimer();

  if (eventSource) {
    eventSource.close();
    eventSource = null;
  }

  streamStarted = true;

  try {
    eventSource = new EventSource('/api/events/stream', { withCredentials: true });

    eventSource.onopen = () => {
      matrixStatus.update((state) => ({ ...state, connected: true, lastError: null }));
      // Reset backoff only after a successful open
      reconnectTimeout = 1000;
    };

    eventSource.onmessage = (message) => {
      messagesReceived++;
      try {
        const payload = JSON.parse(message.data);
        const rateLimit = payload.rate_limit;
        if (rateLimit) {
          if (rateLimit.remaining <= 0) {
            pausedUntil = Date.parse(rateLimit.reset_at) + 1000;
          }
          matrixStatus.update((state) => ({ ...state, rateLimit }));
        }
      } catch {
        // ignore parse errors on the outer message handler
      }
    };

    eventSource.addEventListener('message', (message: MessageEvent) => {
      messagesReceived++;
      try {
        const payload = JSON.parse(message.data);
        matrixEvents.update((list) => {
          const next = [
            {
              ...payload,
              receivedAt: Date.now(),
              payload: payload.payload ?? payload
            },
            ...list
          ].slice(0, 60);
          return next;
        });
      } catch {
        // ignore malformed events
      }
    });

    eventSource.onerror = () => {
      matrixStatus.update((state) => ({
        ...state,
        connected: false,
        lastError: 'Connection lost'
      }));
      eventSource?.close();
      eventSource = null;
      streamStarted = false;

      // If we never received a single message, this endpoint may not be
      // reachable. Use a longer initial backoff to prevent rapid flicker.
      const baseTimeout = messagesReceived === 0 ? 5000 : 1000;
      reconnectTimeout = Math.min(Math.max(reconnectTimeout * 1.5, baseTimeout), 30000);

      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        startMatrixStream();
      }, reconnectTimeout);
    };
  } catch (err) {
    streamStarted = false;
    matrixStatus.update((state) => ({
      ...state,
      connected: false,
      lastError: err instanceof Error ? err.message : 'unknown'
    }));
  }
}

export function stopMatrixStream() {
  clearReconnectTimer();
  streamStarted = false;
  messagesReceived = 0;
  reconnectTimeout = 1000;
  eventSource?.close();
  eventSource = null;
  matrixStatus.set({
    connected: false,
    lastError: 'stream stopped',
    rateLimit: null,
    lastEventId: null
  });
}
