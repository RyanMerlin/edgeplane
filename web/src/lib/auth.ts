import { derived, writable } from 'svelte/store';

const cookieSessionStore = writable<boolean>(false);

const authStore = derived(cookieSessionStore, ($cookieSession) => ({
  token: null as null,
  loggedIn: $cookieSession
}));

export function loginWithCookieSession() {
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

export { authStore };
