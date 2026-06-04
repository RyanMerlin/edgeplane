// Auth store — ported from web/src/lib/auth.ts (Svelte) → Zustand.
// Vestigial `token` field dropped (was always null in the Svelte version).
// Auth is cookie+CSRF only; this store tracks the logged-in state derived
// from a GET /api/auth/me probe.

import { create } from 'zustand';

interface AuthState {
  loggedIn: boolean;
  /** False until the initial GET /api/auth/me probe resolves. Gates the UI so
   *  we don't flash the login screen before auth state is known. */
  bootstrapped: boolean;
  userSubject: string | null;
  bootstrap: () => Promise<void>;
  loginWithCookieSession: () => void;
  logout: () => Promise<void>;
  startOidcLogin: (redirect?: string) => void;
  setUserSubject: (subject: string | null) => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  loggedIn: false,
  bootstrapped: false,
  userSubject: null,

  setUserSubject: (subject) => set({ userSubject: subject }),

  loginWithCookieSession: () => set({ loggedIn: true, bootstrapped: true }),

  bootstrap: async () => {
    try {
      const res = await fetch('/api/auth/me', { credentials: 'include' });
      if (res.ok) {
        const data = (await res.json()) as { subject?: string };
        set({ loggedIn: true, userSubject: data.subject ?? null, bootstrapped: true });
        return;
      }
    } catch {
      // Ignore — keep logged out.
    }
    set({ loggedIn: false, userSubject: null, bootstrapped: true });
  },

  logout: async () => {
    try {
      await fetch('/api/auth/sessions/current', {
        method: 'DELETE',
        credentials: 'include',
      });
    } catch {
      // Local logout still proceeds.
    }
    set({ loggedIn: false, userSubject: null });
  },

  startOidcLogin: (redirect = typeof window !== 'undefined' ? window.location.href : '/') => {
    let path = '/';
    try {
      const parsed = new URL(redirect, window.location.origin);
      path = `${parsed.pathname}${parsed.search}${parsed.hash}` || '/';
    } catch {
      path = '/';
    }
    const url = `/api/auth/oidc/start?redirect=${encodeURIComponent(path)}`;
    window.location.assign(url);
  },
}));
