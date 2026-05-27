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
  let userSubject = $state<string | null>(null);

  $effect(() => {
    return authStore.subscribe($auth => {
      isLoggedIn = $auth.loggedIn;
    });
  });

  // ── User info ─────────────────────────────────────────────────────────────────

  async function fetchUserInfo() {
    try {
      const res = await fetch('/api/auth/me', { credentials: 'include' });
      if (res.ok) {
        const data = await res.json();
        userSubject = data.subject ?? null;
      }
    } catch {
      // ignore — avatar will fall back to 'U'
    }
  }

  function initials(subject: string | null): string {
    if (!subject) return 'U';
    // If it looks like an email, use first letter of local part
    const atIdx = subject.indexOf('@');
    const local = atIdx > 0 ? subject.slice(0, atIdx) : subject;
    // Split on dots, dashes, underscores — take first letter of up to 2 parts
    const parts = local.split(/[._\-\s]+/).filter(Boolean);
    if (parts.length >= 2) {
      return (parts[0][0] + parts[1][0]).toUpperCase();
    }
    return local.slice(0, 2).toUpperCase();
  }

  let avatarInitials = $derived(initials(userSubject));

  // ── SSE lifecycle — start once per login, stop on logout ─────────────────────

  let streamRunning = false;

  $effect(() => {
    if (!isLoggedIn) {
      if (streamRunning) {
        stopMatrixStream();
        streamRunning = false;
      }
      queryClient.clear();
      userSubject = null;
      return;
    }
    if (!streamRunning) {
      streamRunning = true;
      startMatrixStream();
      fetchUserInfo();
    }
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

  // ── Avatar menu ───────────────────────────────────────────────────────────────

  let showAvatarMenu = $state(false);

  function toggleAvatarMenu() { showAvatarMenu = !showAvatarMenu; }

  function handleAvatarKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleAvatarMenu(); }
    if (e.key === 'Escape') showAvatarMenu = false;
  }

  function handleDocClick(e: MouseEvent) {
    const target = e.target as Element | null;
    if (showAvatarMenu && target && !target.closest('.avatar-menu')) {
      showAvatarMenu = false;
    }
  }

  // ── Auth actions ──────────────────────────────────────────────────────────────

  function handleOidc() { startOidcLogin(window.location.pathname); }

  async function handleLogout() {
    showAvatarMenu = false;
    streamRunning = false;
    await logout();
  }

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
    document.addEventListener('click', handleDocClick);
  });

  onDestroy(() => {
    stopMatrixStream();
    if (typeof document !== 'undefined') {
      document.removeEventListener('click', handleDocClick);
    }
  });

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

        <!-- right actions -->
        <div class="topbar-right">
          <button class="icon-btn ghost" onclick={toggleTheme} title={theme === 'dark' ? 'Light mode' : 'Dark mode'}>
            {theme === 'dark' ? '☀' : '☾'}
          </button>

          <!-- User avatar with dropdown -->
          <div class="avatar-menu">
            <button
              class="avatar"
              onclick={toggleAvatarMenu}
              onkeydown={handleAvatarKeydown}
              title={userSubject ?? 'User menu'}
              aria-haspopup="true"
              aria-expanded={showAvatarMenu}
            >
              {avatarInitials}
            </button>
            {#if showAvatarMenu}
              <div class="avatar-dropdown" role="menu">
                {#if userSubject}
                  <div class="avatar-subject">{userSubject}</div>
                {/if}
                <button role="menuitem" onclick={() => { showAvatarMenu = false; }}>Profile</button>
                <button role="menuitem" onclick={() => { showAvatarMenu = false; }}>Settings</button>
                <button role="menuitem" class="logout-item" onclick={handleLogout}>Logout</button>
              </div>
            {/if}
          </div>
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

<style>
  /* Avatar menu styles — scoped to layout */
  .avatar-menu {
    position: relative;
    display: flex;
    align-items: center;
  }

  .avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--accent, #ddc05a);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
    border: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    letter-spacing: 0.03em;
    transition: opacity 0.15s;
    flex-shrink: 0;
  }

  .avatar:hover {
    opacity: 0.85;
  }

  .avatar-dropdown {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    min-width: 160px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
    z-index: 1000;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .avatar-subject {
    padding: 6px 12px;
    font-size: 10px;
    color: var(--muted);
    border-bottom: 1px solid var(--border);
    word-break: break-all;
  }

  .avatar-dropdown button {
    background: none;
    border: none;
    color: var(--text);
    font-size: 12px;
    padding: 7px 12px;
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
  }

  .avatar-dropdown button:hover {
    background: var(--surface-2);
  }

  .logout-item {
    color: var(--err, #f87171) !important;
    border-top: 1px solid var(--border);
  }
</style>
