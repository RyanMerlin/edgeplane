import { useAuthStore } from '@/stores/auth';
import { Link, useRouterState } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { NAV_GROUPS, isNavItemActive } from './navModel';

// ── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Returns 2-letter UPPER initials when the subject looks like an email or a
 * name with separators (dot, dash, underscore, space). Returns null for opaque
 * ids/hashes so the caller can render a neutral glyph instead.
 */
export function avatarLabel(subject: string | null): string | null {
  if (!subject) return null;

  // Opaque hash: 24+ hex chars with no separators → treat as opaque
  if (/^[0-9a-f]{24,}$/i.test(subject)) return null;

  // Email: extract the local part
  const atIdx = subject.indexOf('@');
  const local = atIdx > 0 ? subject.slice(0, atIdx) : subject;

  // Split on word separators
  const parts = local.split(/[._\-\s]+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }

  // Single word — only return initials if it looks like a real word (has no digits)
  if (parts.length === 1 && /^[a-zA-Z]+$/.test(parts[0]) && parts[0].length >= 2) {
    return parts[0].slice(0, 2).toUpperCase();
  }

  return null;
}

function applyTheme(next: string) {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = next;
    localStorage.setItem('edgeplane:theme', next);
  }
}

// ── Nav icon map ─────────────────────────────────────────────────────────────
const NAV_ICON: Record<string, string> = {
  '/': '◇',
  '/agents': '◉',
  '/domains': '▤',
  '/feed': '≋',
  '/governance': '⚖',
};

// ── Sidebar ──────────────────────────────────────────────────────────────────

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const userSubject = useAuthStore((s) => s.userSubject);
  const logout = useAuthStore((s) => s.logout);

  const [showMenu, setShowMenu] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [_theme, setTheme] = useState('dark');
  const menuRef = useRef<HTMLDivElement>(null);

  // Theme init — read from localStorage on mount
  useEffect(() => {
    const saved = localStorage.getItem('edgeplane:theme');
    const initial = saved === 'light' ? 'light' : 'dark';
    setTheme(initial);
    applyTheme(initial);
  }, []);

  // Close menu on outside click
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (showMenu && menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
        setShowSettings(false);
      }
    }
    document.addEventListener('click', handleClick);
    return () => document.removeEventListener('click', handleClick);
  }, [showMenu]);

  const toggleTheme = () => {
    setTheme((prev) => {
      const next = prev === 'dark' ? 'light' : 'dark';
      applyTheme(next);
      return next;
    });
  };

  const label = avatarLabel(userSubject);

  return (
    <nav
      data-testid="sidebar"
      style={{
        width: 'var(--sidebar, 232px)',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--frame)',
        borderRight: '1px solid var(--border)',
        flexShrink: 0,
        padding: '10px 8px 8px',
        gap: 2,
        boxSizing: 'border-box',
      }}
    >
      {/* Brand row */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '4px 8px 10px',
        }}
      >
        <span style={{ color: 'var(--accent)', fontSize: 16, lineHeight: 1 }}>⬡</span>
        <span
          style={{
            fontSize: '13.5px',
            fontWeight: 590,
            color: 'var(--text)',
            letterSpacing: '-0.01em',
          }}
        >
          EdgePlane
        </span>
      </div>

      {/* Search row — static affordance; ⌘K palette is a later phase */}
      <button
        type="button"
        aria-label="Search"
        onClick={() => {
          /* ⌘K palette — Phase C */
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          height: 30,
          padding: '0 8px',
          marginBottom: 8,
          background: 'var(--input)',
          border: '1px solid var(--border-subtle)',
          borderRadius: 6,
          color: 'var(--dim)',
          fontSize: 13,
          cursor: 'pointer',
          width: '100%',
          textAlign: 'left',
          fontFamily: 'var(--font)',
        }}
      >
        Search…
        <kbd
          style={{
            marginLeft: 'auto',
            fontSize: 11,
            color: 'var(--dim)',
            fontFamily: 'var(--mono)',
            background: 'none',
            border: 'none',
            padding: 0,
          }}
        >
          ⌘K
        </kbd>
      </button>

      {/* Nav items */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 1, flex: 1 }}>
        {NAV_GROUPS.flatMap((g) => g.items).map((item) => {
          const active = isNavItemActive(item.to, pathname);
          const icon = NAV_ICON[item.to] ?? '·';
          return (
            <Link
              key={item.to}
              to={item.to}
              data-testid={`nav-${item.to}`}
              aria-current={active ? 'page' : undefined}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 9,
                height: 28,
                padding: '0 8px',
                borderRadius: 6,
                color: active ? 'var(--text)' : 'var(--text-2)',
                fontSize: 13,
                fontWeight: 510,
                textDecoration: 'none',
                background: active ? 'var(--raised-2)' : 'transparent',
                cursor: 'pointer',
                userSelect: 'none',
                transition: 'background .12s ease, color .12s ease',
              }}
              onMouseEnter={(e) => {
                if (!active) {
                  (e.currentTarget as HTMLElement).style.background = 'var(--raised)';
                }
              }}
              onMouseLeave={(e) => {
                if (!active) {
                  (e.currentTarget as HTMLElement).style.background = 'transparent';
                }
              }}
            >
              <span
                style={{
                  width: 15,
                  height: 15,
                  flexShrink: 0,
                  color: active ? 'var(--accent)' : 'var(--dim)',
                  display: 'grid',
                  placeItems: 'center',
                  fontSize: 13,
                  transition: 'color .12s ease',
                }}
              >
                {icon}
              </span>
              {item.label}
            </Link>
          );
        })}
      </div>

      {/* Bottom account control */}
      <div ref={menuRef} style={{ marginTop: 'auto', position: 'relative' }}>
        {/* Account popover menu — appears above the control */}
        {showMenu && (
          <div
            role="menu"
            data-testid="account-menu"
            style={{
              position: 'absolute',
              bottom: 42,
              left: 4,
              right: 4,
              background: 'var(--frame)',
              border: '1px solid var(--border)',
              borderRadius: 8,
              padding: 4,
              zIndex: 20,
              boxShadow: '0 8px 28px rgba(0,0,0,0.5)',
            }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === 'Escape') setShowMenu(false);
            }}
          >
            {/* Subject line */}
            {userSubject && (
              <div
                style={{
                  padding: '6px 8px',
                  fontSize: 11,
                  color: 'var(--dim)',
                  fontFamily: 'var(--mono)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  borderBottom: '1px solid var(--border-subtle)',
                  marginBottom: 4,
                }}
              >
                {userSubject}
              </div>
            )}

            {/* Settings row with submenu toggle */}
            <button
              type="button"
              role="menuitem"
              data-testid="settings-item"
              onClick={(e) => {
                e.stopPropagation();
                setShowSettings((v) => !v);
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '6px 8px',
                borderRadius: 5,
                fontSize: 13,
                color: 'var(--text-2)',
                cursor: 'pointer',
                width: '100%',
                background: 'transparent',
                border: 'none',
                textAlign: 'left',
                fontFamily: 'var(--font)',
                transition: 'background .12s ease',
              }}
            >
              ⚙ Settings
              <span style={{ marginLeft: 'auto', color: 'var(--dim)', fontSize: 11 }}>›</span>
            </button>

            {/* Settings submenu */}
            {showSettings && (
              <div
                data-testid="settings-submenu"
                style={{
                  margin: '2px 0 2px 10px',
                  paddingLeft: 8,
                  borderLeft: '1px solid var(--border-subtle)',
                }}
              >
                <Link
                  to="/onboarding"
                  data-testid="menu-onboarding"
                  role="menuitem"
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '6px 8px',
                    borderRadius: 5,
                    fontSize: 13,
                    color: 'var(--text-2)',
                    cursor: 'pointer',
                    textDecoration: 'none',
                    transition: 'background .12s ease',
                  }}
                >
                  Onboarding
                </Link>
                <button
                  type="button"
                  role="menuitem"
                  data-testid="menu-preferences"
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '6px 8px',
                    borderRadius: 5,
                    fontSize: 13,
                    color: 'var(--text-2)',
                    cursor: 'pointer',
                    width: '100%',
                    background: 'transparent',
                    border: 'none',
                    textAlign: 'left',
                    fontFamily: 'var(--font)',
                    transition: 'background .12s ease',
                  }}
                >
                  Preferences
                </button>
              </div>
            )}

            {/* Theme toggle */}
            <button
              type="button"
              role="menuitem"
              data-testid="theme-item"
              onClick={toggleTheme}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '6px 8px',
                borderRadius: 5,
                fontSize: 13,
                color: 'var(--text-2)',
                cursor: 'pointer',
                width: '100%',
                background: 'transparent',
                border: 'none',
                textAlign: 'left',
                fontFamily: 'var(--font)',
                transition: 'background .12s ease',
              }}
            >
              ☾ Theme
            </button>

            {/* Logout */}
            <button
              type="button"
              role="menuitem"
              data-testid="logout-item"
              onClick={async () => {
                setShowMenu(false);
                setShowSettings(false);
                await logout();
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '6px 8px',
                borderRadius: 5,
                fontSize: 13,
                color: 'var(--err)',
                cursor: 'pointer',
                width: '100%',
                background: 'transparent',
                border: 'none',
                textAlign: 'left',
                fontFamily: 'var(--font)',
                transition: 'background .12s ease',
              }}
            >
              Logout
            </button>
          </div>
        )}

        {/* Account button row */}
        <button
          type="button"
          data-testid="account-btn"
          aria-haspopup="menu"
          aria-expanded={showMenu}
          onClick={() => {
            setShowMenu((v) => !v);
            if (showMenu) setShowSettings(false);
          }}
          title={userSubject ?? 'User menu'}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 9,
            height: 34,
            padding: '0 8px',
            width: '100%',
            background: 'transparent',
            border: 'none',
            borderRadius: 6,
            cursor: 'pointer',
            color: 'var(--text-2)',
            fontFamily: 'var(--font)',
            fontSize: 13,
            textAlign: 'left',
            transition: 'background .12s ease',
          }}
        >
          {/* Avatar */}
          <span
            style={{
              width: 22,
              height: 22,
              borderRadius: 5,
              background: 'var(--accent-dim)',
              color: 'var(--accent)',
              display: 'grid',
              placeItems: 'center',
              fontSize: 11,
              fontWeight: 590,
              flexShrink: 0,
            }}
          >
            {label ?? '⬡'}
          </span>
          {/* Name */}
          <span
            style={{
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {label ? userSubject : 'Account'}
          </span>
          {/* Chevron */}
          <span style={{ marginLeft: 'auto', color: 'var(--dim)', fontSize: 11 }}>⌄</span>
        </button>
      </div>
    </nav>
  );
}

export default Sidebar;
