import { api } from '@/lib/api/http';
import { useAuthStore } from '@/stores/auth';
import { useToastStore } from '@/stores/toast';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Link, Outlet, createRootRoute, useNavigate, useRouter } from '@tanstack/react-router';
import { useCallback, useEffect, useRef, useState } from 'react';

// ── QueryClient singleton ─────────────────────────────────────────────────────

const queryClient = new QueryClient({
  defaultOptions: { queries: { refetchOnWindowFocus: true, retry: 1 } },
});

// ── Root route ────────────────────────────────────────────────────────────────

export const Route = createRootRoute({
  component: RootLayout,
});

// ── Helpers ───────────────────────────────────────────────────────────────────

function initials(subject: string | null): string {
  if (!subject) return 'U';
  const atIdx = subject.indexOf('@');
  const local = atIdx > 0 ? subject.slice(0, atIdx) : subject;
  const parts = local.split(/[._\-\s]+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }
  return local.slice(0, 2).toUpperCase();
}

function applyTheme(next: string) {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = next;
    localStorage.setItem('edgeplane:theme', next);
  }
}

// ── Components ────────────────────────────────────────────────────────────────

function NavLink({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link
      to={to}
      className="nav-tab"
      activeProps={{ className: 'nav-tab active' }}
      activeOptions={{ exact: to === '/' }}
    >
      {children}
    </Link>
  );
}

function TopBar({
  theme,
  onToggleTheme,
  userSubject,
  onLogout,
}: {
  theme: string;
  onToggleTheme: () => void;
  userSubject: string | null;
  onLogout: () => Promise<void>;
}) {
  const [showMenu, setShowMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const navigate = useNavigate();

  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (showMenu && menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
    }
    document.addEventListener('click', handleClick);
    return () => document.removeEventListener('click', handleClick);
  }, [showMenu]);

  return (
    <div className="topbar">
      <span className="topbar-logo">EdgePlane</span>

      <NavLink to="/">Fleet</NavLink>
      <NavLink to="/ai">Console</NavLink>
      <NavLink to="/explorer">Explorer</NavLink>
      <NavLink to="/feed">Feed</NavLink>
      <NavLink to="/governance">Governance</NavLink>

      <div className="topbar-right">
        <button
          type="button"
          className="icon-btn ghost"
          onClick={onToggleTheme}
          title={theme === 'dark' ? 'Light mode' : 'Dark mode'}
        >
          {theme === 'dark' ? '☀' : '☾'}
        </button>

        <div className="avatar-menu" ref={menuRef}>
          <button
            type="button"
            className="avatar"
            onClick={() => setShowMenu((v) => !v)}
            title={userSubject ?? 'User menu'}
            aria-haspopup="true"
            aria-expanded={showMenu}
          >
            {initials(userSubject)}
          </button>
          {showMenu && (
            <div className="avatar-dropdown" role="menu">
              {userSubject && <div className="avatar-subject">{userSubject}</div>}
              <button type="button" role="menuitem" onClick={() => setShowMenu(false)}>
                Profile
              </button>
              <button type="button" role="menuitem" onClick={() => setShowMenu(false)}>
                Settings
              </button>
              <button
                type="button"
                role="menuitem"
                data-testid="menu-onboarding"
                onClick={() => {
                  setShowMenu(false);
                  navigate({ to: '/onboarding' });
                }}
              >
                Onboarding
              </button>
              <button
                type="button"
                role="menuitem"
                className="logout-item"
                onClick={async () => {
                  setShowMenu(false);
                  await onLogout();
                }}
              >
                Logout
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function StatusBar() {
  return (
    <div className="statusbar">
      <div className="statusbar-left">
        <span className="ok">●</span>
        <span>Connected</span>
      </div>
      <div className="statusbar-right">
        <span>EdgePlane</span>
      </div>
    </div>
  );
}

function LoginScreen() {
  const startOidcLogin = useAuthStore((s) => s.startOidcLogin);
  return (
    <div className="login-view">
      <div className="login-card">
        <div className="login-card-head">
          <div>
            <div className="login-card-logo">edgeplane</div>
            <div className="login-card-sub">Fleet operations console</div>
          </div>
          <div className="login-card-status">
            <span className="ok">●</span>
            <span>online</span>
          </div>
        </div>

        <div className="login-card-body">
          <div className="login-section">
            <p className="login-desc">
              Authenticate via your organization's identity provider. All sessions are
              CSRF-protected and expire after inactivity.
            </p>
            <button
              type="button"
              className="primary"
              style={{
                width: '100%',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                gap: '6px',
                padding: '7px 12px',
              }}
              onClick={() => startOidcLogin(window.location.pathname)}
            >
              <span>⬡</span>
              Sign in with OIDC
            </button>
          </div>
        </div>

        <div className="login-card-foot">
          <div className="login-foot-badges">
            <span className="login-foot-badge">
              <span className="ok">✓</span> TLS
            </span>
            <span className="login-foot-badge">
              <span className="ok">✓</span> CSRF
            </span>
            <span className="login-foot-badge">
              <span className="ok">✓</span> Cookie
            </span>
          </div>
          <span>Secure session · httpOnly</span>
        </div>
      </div>
    </div>
  );
}

// ── Root layout ───────────────────────────────────────────────────────────────

function RootLayout() {
  const loggedIn = useAuthStore((s) => s.loggedIn);
  const bootstrapped = useAuthStore((s) => s.bootstrapped);
  const userSubject = useAuthStore((s) => s.userSubject);
  const bootstrap = useAuthStore((s) => s.bootstrap);
  const loginWithCookieSession = useAuthStore((s) => s.loginWithCookieSession);
  const logout = useAuthStore((s) => s.logout);
  const setUserSubject = useAuthStore((s) => s.setUserSubject);

  const [theme, setTheme] = useState('dark');
  const [toast, setToast] = useState<string | null>(null);
  const storeToast = useToastStore((s) => s.message);

  const router = useRouter();

  // Stable refs so the mount effect doesn't depend on Zustand function identity
  const bootstrapRef = useRef(bootstrap);
  const loginRef = useRef(loginWithCookieSession);
  const setSubjectRef = useRef(setUserSubject);
  bootstrapRef.current = bootstrap;
  loginRef.current = loginWithCookieSession;
  setSubjectRef.current = setUserSubject;

  // ── Theme init ────────────────────────────────────────────────────────────
  useEffect(() => {
    const saved = localStorage.getItem('edgeplane:theme');
    const initial = saved === 'light' ? 'light' : 'dark';
    setTheme(initial);
    applyTheme(initial);
  }, []);

  const toggleTheme = useCallback(() => {
    setTheme((prev) => {
      const next = prev === 'dark' ? 'light' : 'dark';
      applyTheme(next);
      return next;
    });
  }, []);

  // ── OIDC grant exchange + bootstrap ──────────────────────────────────────
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const hashParams = new URLSearchParams(window.location.hash.replace(/^#/, ''));
    const grant = hashParams.get('oidc_grant') ?? params.get('oidc_grant');

    if (grant) {
      // OIDC return: the session cookie is established by the exchange below,
      // so probing GET /auth/me first would 401 (no session yet) and flash the
      // login screen before the exchange resolves. Exchange only; a successful
      // exchange already flips loggedIn+bootstrapped (loginWithCookieSession),
      // so we go straight to the dashboard with no intermediate splash. Only
      // fall back to a bootstrap probe if the exchange itself fails.
      api
        .post<{ subject?: string }>('/auth/oidc/exchange', { grant })
        .then((data) => {
          loginRef.current();
          if (data?.subject) setSubjectRef.current(data.subject);
          hashParams.delete('oidc_grant');
          params.delete('oidc_grant');
          const query = params.toString();
          const hash = hashParams.toString();
          window.history.replaceState(
            {},
            '',
            `${window.location.pathname}${query ? `?${query}` : ''}${hash ? `#${hash}` : ''}`,
          );
          router.invalidate();
        })
        .catch((err: unknown) => {
          const msg = err instanceof Error ? err.message : 'OIDC login failed';
          setToast(msg);
          setTimeout(() => setToast(null), 4000);
          bootstrapRef.current();
        });
      return;
    }

    bootstrapRef.current();
  }, [router]);

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <QueryClientProvider client={queryClient}>
      {!bootstrapped ? (
        // Auth state not yet known — show a minimal dark splash instead of
        // flashing the login screen before GET /api/auth/me resolves.
        <div className="login-view">
          <div className="login-card-logo" style={{ opacity: 0.4 }}>
            edgeplane
          </div>
        </div>
      ) : loggedIn ? (
        <div className="app-shell">
          <TopBar
            theme={theme}
            onToggleTheme={toggleTheme}
            userSubject={userSubject}
            onLogout={logout}
          />
          <div className="app-content">
            <Outlet />
          </div>
          <StatusBar />
        </div>
      ) : (
        <LoginScreen />
      )}
      {(toast ?? storeToast) && (
        <div className="toast" role="alert">
          {toast ?? storeToast}
        </div>
      )}
    </QueryClientProvider>
  );
}
