import { derived, writable } from 'svelte/store';

const tokenStore = writable<string | null>(null);
const cookieSessionStore = writable<boolean>(false);

const authStore = derived([tokenStore, cookieSessionStore], ([$token, $cookieSession]) => ({
  token: $token,
  loggedIn: Boolean($token) || $cookieSession
}));

export function loginWithToken(value: string) {
  tokenStore.set(value);
  cookieSessionStore.set(false);
}

export function loginWithCookieSession() {
  tokenStore.set(null);
  cookieSessionStore.set(true);
}

export async function bootstrapAuth() {
  try {
    const res = await fetch('/api/auth/me', { credentials: 'include' });
    if (res.ok) {
      cookieSessionStore.set(true);
      return;
    }
  } catch {
    // Ignore and keep logged out.
  }
  cookieSessionStore.set(false);
}

export async function logout() {
  try {
    await fetch('/api/auth/sessions/current', {
      method: 'DELETE',
      credentials: 'include'
    });
  } catch {
    // Local logout still proceeds.
  }
  tokenStore.set(null);
  cookieSessionStore.set(false);
}

export function startOidcLogin(redirect = window?.location?.href) {
  const path = (() => {
    try {
      const parsed = new URL(redirect, window.location.origin);
      return `${parsed.pathname}${parsed.search}${parsed.hash}`;
    } catch {
      return '/';
    }
  })();
  const url = `/api/auth/oidc/start?redirect=${encodeURIComponent(path || '/')}`;
  window.location.assign(url);
}

export const token = derived(tokenStore, ($token) => $token);
export { authStore };
