/**
 * Typed API client — wraps openapi-fetch with CSRF + credentials middleware.
 *
 * CSRF logic ported from src/lib/api/http.ts:
 *   - Reads `ep_csrf_token` cookie
 *   - Injects `X-CSRF-Token` header on mutations (POST/PUT/PATCH/DELETE)
 *   - Always sends `credentials: 'include'`
 *
 * Base URL is `''` (same-origin); Vite dev proxy forwards `/api → localhost:8008`.
 */

import createClient, { type Middleware } from 'openapi-fetch';
import type { paths } from './schema.gen';

// ── CSRF helper ───────────────────────────────────────────────────────────────

function readCookie(name: string): string | null {
  if (typeof document === 'undefined') return null;
  const needle = `${name}=`;
  for (const part of document.cookie.split(';')) {
    const item = part.trim();
    if (item.startsWith(needle)) return decodeURIComponent(item.slice(needle.length));
  }
  return null;
}

// ── Middleware ────────────────────────────────────────────────────────────────

const csrfMiddleware: Middleware = {
  async onRequest({ request }) {
    const method = request.method.toUpperCase();
    if (['POST', 'PUT', 'PATCH', 'DELETE'].includes(method)) {
      const csrf = readCookie('ep_csrf_token');
      if (csrf) {
        request.headers.set('X-CSRF-Token', csrf);
      }
    }
    return request;
  },
};

// ── Client singleton ──────────────────────────────────────────────────────────

export const apiClient = createClient<paths>({
  baseUrl: '',
  credentials: 'include',
});

apiClient.use(csrfMiddleware);

// ── Typed helpers (thin wrappers for query functions) ─────────────────────────

export type ApiClientError = {
  status: number;
  message: string;
};

/**
 * Unwrap an openapi-fetch response, throwing on error.
 * The typed client returns `{ data, error, response }` — callers want the data
 * directly (compatible with TanStack Query's `queryFn` signature).
 */
export async function unwrap<T>(
  promise: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  const { data, error, response } = await promise;
  if (error !== undefined || !response.ok) {
    const status = response.status;
    let message = `Request failed: ${status}`;
    if (error && typeof error === 'object' && 'error' in error) {
      message = String((error as { error: unknown }).error);
    } else if (typeof error === 'string') {
      message = error;
    }
    const err: ApiClientError & Error = Object.assign(new Error(message), { status, message });
    throw err;
  }
  // data is guaranteed non-undefined when response.ok and no error
  return data as T;
}
