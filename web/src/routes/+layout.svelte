<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import '../app.css';
  import { authStore, bootstrapAuth, loginWithCookieSession, startOidcLogin, logout } from '$lib/auth';
  import { base } from '$app/paths';
  import { exchangeOidcGrant } from '$lib/api';
  import { startMatrixStream, stopMatrixStream } from '$lib/telemetry';
  import { toastStore, showToast } from '$lib/stores/toast';
  import { QueryClient, QueryClientProvider } from '@tanstack/svelte-query';
  import { page } from '$app/state';

  let { children } = $props();

  const queryClient = new QueryClient({
    defaultOptions: { queries: { refetchOnWindowFocus: true, retry: 1 } }
  });

  // ── Auth state ────────────────────────────────────────────────────────────────

  let isLoggedIn = $state(get(authStore).loggedIn);

  $effect(() => {
    return authStore.subscribe($auth => {
      isLoggedIn = $auth.loggedIn;
    });
  });

  // ── SSE lifecycle ──────────────────────────────────────────────────────────────

  $effect(() => {
    if (!isLoggedIn) {
      stopMatrixStream();
      queryClient.clear();
      return;
    }
    startMatrixStream();
    return () => { stopMatrixStream(); };
  });

  // ── Theme ─────────────────────────────────────────────────────────────────────

  let theme = $state('dark');

  function applyTheme(next: string) {
    theme = next;
    if (typeof document !== 'undefined') {
      document.documentElement.dataset.theme = next;
      localStorage.setItem('edgeplane:theme', next);
    }
  }

  function toggleTheme() { applyTheme(theme === 'dark' ? 'light' : 'dark'); }

  // ── Auth actions ──────────────────────────────────────────────────────────────

  function handleOidc() { startOidcLogin(window.location.pathname); }

  // ── Mount ─────────────────────────────────────────────────────────────────────

  onMount(() => {
    const saved = localStorage.getItem('edgeplane:theme');
    applyTheme(saved === 'light' ? 'light' : 'dark');

    const params = new URLSearchParams(window.location.search);
    const hashParams = new URLSearchParams(window.location.hash.replace(/^#/, ''));
    const grant = hashParams.get('oidc_grant') || params.get('oidc_grant');
    if (grant) {
      exchangeOidcGrant(grant)
        .then(() => {
          loginWithCookieSession();
          hashParams.delete('oidc_grant');
          params.delete('oidc_grant');
          const query = params.toString();
          const hash = hashParams.toString();
          window.history.replaceState(
            {}, '',
            `${window.location.pathname}${query ? `?${query}` : ''}${hash ? `#${hash}` : ''}`
          );
        })
        .catch(err => { showToast(err instanceof Error ? err.message : 'OIDC login failed'); });
    }

    bootstrapAuth();
  });

  onDestroy(() => { stopMatrixStream(); });

  // ── Nav helpers ───────────────────────────────────────────────────────────────

  function navClass(path: string) {
    const active = page.url.pathname === `${base}${path}` ||
      (path !== '/' && page.url.pathname.startsWith(`${base}${path}`));
    return `nav-tab${active ? ' active' : ''}`;
  }
</script>

<QueryClientProvider client={queryClient}>
  {#if isLoggedIn}
    <div class="app-shell">

      <!-- 34px topbar -->
      <div class="topbar">
        <span class="topbar-logo">EdgePlane</span>

        <!-- flat underline nav -->
        <a href="{base}/" class={navClass('/')}>Overview</a>
        <a href="{base}/ai/" class={navClass('/ai/')}>Console</a>
        <a href="{base}/agents/" class={navClass('/agents/')}>Agents</a>
        <a href="{base}/explorer/" class={navClass('/explorer/')}>Explorer</a>
        <a href="{base}/feed/" class={navClass('/feed/')}>Feed</a>
        <a href="{base}/governance/" class={navClass('/governance/')}>Governance</a>
        <a href="{base}/onboarding/" class={navClass('/onboarding/')}>Onboarding</a>

        <!-- right actions -->
        <div class="topbar-right">
          <button class="icon-btn ghost" onclick={toggleTheme} title={theme === 'dark' ? 'Light mode' : 'Dark mode'}>
            {theme === 'dark' ? '☀' : '☾'}
          </button>
          <button class="ghost" onclick={logout}>Logout</button>
        </div>
      </div>

      <!-- flex:1 content -->
      <div class="app-content">
        {@render children()}
      </div>

      <!-- 24px statusbar -->
      <div class="statusbar">
        <div class="statusbar-left">
          <span class="ok">●</span>
          <span>Connected</span>
        </div>
        <div class="statusbar-right">
          <span>EdgePlane</span>
        </div>
      </div>

    </div>
  {:else}
    <!-- Login view — no topbar, no statusbar -->
    <div class="login-view">
      <div class="login-card">

        <div class="login-card-head">
          <div>
            <div class="login-card-logo">edgeplane</div>
            <div class="login-card-sub">Fleet operations console</div>
          </div>
          <div class="login-card-status">
            <span class="ok">●</span>
            <span>online</span>
          </div>
        </div>

        <div class="login-card-body">

          <div class="login-section">
            <p class="login-desc">Authenticate via your organization's identity provider. All sessions are CSRF-protected and expire after inactivity.</p>
            <button class="primary" style="width:100%; display:flex; align-items:center; justify-content:center; gap:6px; padding:7px 12px;" onclick={handleOidc}>
              <span>⬡</span>
              Sign in with OIDC
            </button>
          </div>

        </div>

        <div class="login-card-foot">
          <div class="login-foot-badges">
            <span class="login-foot-badge"><span class="ok">✓</span> TLS</span>
            <span class="login-foot-badge"><span class="ok">✓</span> CSRF</span>
            <span class="login-foot-badge"><span class="ok">✓</span> Cookie</span>
          </div>
          <span>Secure session · httpOnly</span>
        </div>

      </div>
    </div>
  {/if}

  {#if $toastStore.visible}
    <div class="toast" role="alert">{$toastStore.message}</div>
  {/if}
</QueryClientProvider>
