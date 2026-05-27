<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import '../app.css';
  import { authStore, bootstrapAuth, loginWithCookieSession, loginWithToken, token, startOidcLogin, logout } from '$lib/auth';
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
  let currentToken = $state<string | null>(get(authStore).token ?? null);
  let initialToken = $state('');

  $effect(() => {
    return authStore.subscribe($auth => {
      isLoggedIn = $auth.loggedIn;
      currentToken = $auth.token ?? null;
    });
  });

  // ── SSE lifecycle ──────────────────────────────────────────────────────────────

  $effect(() => {
    if (!isLoggedIn) {
      stopMatrixStream();
      queryClient.clear();
      return;
    }
    startMatrixStream(currentToken ?? undefined);
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

  function handleToken() {
    if (!initialToken.trim()) { showToast('Enter a Edgeplane token or use OIDC login.'); return; }
    loginWithToken(initialToken.trim());
  }

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
    return `tab ${page.url.pathname.startsWith(`${base}${path}`) ? 'active' : ''}`;
  }
</script>

<QueryClientProvider client={queryClient}>
  <div class="shell">
    <header class="shell-header glass-panel">
      <div>
        <div class="status-chip">Edgeplane</div>
        <p style="margin:0.25rem 0 0;font-size:0.9rem; color: var(--muted);">
          {#if isLoggedIn}Connected{:else}Authenticate to continue{/if}
        </p>
      </div>
      <div class="header-actions">
        <button class="ghost icon-btn" onclick={toggleTheme} title={theme === 'dark' ? 'Switch to light' : 'Switch to dark'}>
          {theme === 'dark' ? '☀' : '☾'}
        </button>
        {#if isLoggedIn}
          <button class="ghost" onclick={logout}>Logout</button>
        {/if}
      </div>
    </header>

    {#if isLoggedIn}
      <nav class="tabs">
        <a href="{base}/fleet/" class={navClass('/fleet')}>Fleet</a>
        <a href="{base}/agents/" class={navClass('/agents')}>Agents</a>
        <a href="{base}/ai/" class={navClass('/ai')}>AI Console</a>
        <a href="{base}/matrix/" class={navClass('/matrix')}>Matrix</a>
        <a href="{base}/explorer/" class={navClass('/explorer')}>Explorer</a>
        <a href="{base}/onboarding/" class={navClass('/onboarding')}>Onboarding</a>
        <a href="{base}/governance/" class={navClass('/governance')}>Governance</a>
      </nav>
      <div class="main-shell">
        {@render children()}
      </div>
    {:else}
      <section class="login">
        <div class="login-card">
          <div class="status-chip">Edgeplane Secure</div>
          <h1>Team Console</h1>
          <p class="muted" style="margin:0;">OIDC is the production login path. Token login is for testing.</p>
          <div class="login-actions">
            <button class="primary" onclick={handleOidc}>Sign in via OIDC</button>
          </div>
          <label>Testing Token<input bind:value={initialToken} type="password" placeholder="EP_TOKEN" /></label>
          <div class="login-actions">
            <button class="ghost" onclick={handleToken}>Continue with token</button>
          </div>
        </div>
      </section>
    {/if}

    {#if $toastStore.visible}
      <div class="toast" role="alert">{$toastStore.message}</div>
    {/if}
  </div>
</QueryClientProvider>
