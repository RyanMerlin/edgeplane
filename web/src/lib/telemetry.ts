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

function buildUrl(token?: string) {
  const base = '/api/events/stream';
  if (!token) return base;
  return `${base}?ep_token=${encodeURIComponent(token)}`;
}

export function startMatrixStream(token?: string) {
  if (!browser) return;
  if (Date.now() < pausedUntil) return;
  if (eventSource) {
    eventSource.close();
  }

  try {
    eventSource = new EventSource(buildUrl(token));
    matrixStatus.update((state) => ({ ...state, connected: true, lastError: null }));
    reconnectTimeout = 1000;

    eventSource.onmessage = (message) => {
      const payload = JSON.parse(message.data);
      const rateLimit = payload.rate_limit;
      if (rateLimit) {
        if (rateLimit.remaining <= 0) {
          pausedUntil = Date.parse(rateLimit.reset_at) + 1000;
        }
        matrixStatus.update((state) => ({ ...state, rateLimit }));
      }
    };

    eventSource.addEventListener('message', (message: MessageEvent) => {
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
    });

    eventSource.onerror = () => {
      matrixStatus.update((state) => ({
        ...state,
        connected: false,
        lastError: 'Connection lost'
      }));
      eventSource?.close();
      eventSource = null;
      reconnectTimeout = Math.min(reconnectTimeout * 1.5, 30000);
      setTimeout(() => startMatrixStream(token), reconnectTimeout);
    };
  } catch (err) {
    matrixStatus.update((state) => ({
      ...state,
      connected: false,
      lastError: err instanceof Error ? err.message : 'unknown'
    }));
  }
}

export function stopMatrixStream() {
  eventSource?.close();
  eventSource = null;
  matrixStatus.set({
    connected: false,
    lastError: 'stream stopped',
    rateLimit: null,
    lastEventId: null
  });
}
