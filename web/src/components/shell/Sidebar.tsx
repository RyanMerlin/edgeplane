import { Link, useRouterState } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { useAuthStore } from '@/stores/auth';
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

// ── Sidebar ──────────────────────────────────────────────────────────────────

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const userSubject = useAuthStore((s) => s.userSubject);
  const logout = useAuthStore((s) => s.logout);

  const [showMenu, setShowMenu] = useState(false);
  const [theme, setTheme] = useState('dark');
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
      style={{
        width: 200,
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--surface)',
        borderRight: '1px solid var(--border)',
        flexShrink: 0,
      }}
    >
      {/* Logo */}
      <div
        style={{
          padding: '12px 14px',
          color: 'var(--accent)',
          fontWeight: 700,
          fontSize: 14,
          letterSpacing: '-0.03em',
          borderBottom: '1px solid var(--border)',
        }}
      >
        EdgePlane
      </div>

      {/* Nav groups */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '8px 0' }}>
        {NAV_GROUPS.map((group, gi) => (
          <div key={gi}>
            {group.heading && (
              <div
                style={{
                  padding: '8px 14px 4px',
                  fontSize: 10,
                  color: 'var(--dim)',
                  letterSpacing: '0.08em',
                  fontWeight: 700,
                }}
              >
                {group.heading}
              </div>
            )}
            {group.items.map((item) => {
              const active = isNavItemActive(item.to, pathname);
              return (
                <Link
                  key={item.to}
                  to={item.to}
                  data-testid={`nav-${item.to}`}
                  aria-current={active ? 'page' : undefined}
                  style={{
                    display: 'block',
                    padding: '5px 14px',
                    color: active ? 'var(--text)' : 'var(--muted)',
                    textDecoration: 'none',
                    fontSize: 13,
                    borderLeft: active ? '2px solid var(--accent)' : '2px solid transparent',
                    background: active ? 'var(--surface-2)' : 'transparent',
                  }}
                >
                  {item.label}
                </Link>
              );
            })}
          </div>
        ))}
      </div>

      {/* Footer */}
      <div
        style={{
          marginTop: 'auto',
          borderTop: '1px solid var(--border)',
          padding: '8px 0',
        }}
      >
        {/* Onboarding link */}
        <Link
          to="/onboarding"
          data-testid="nav-onboarding"
          style={{
            display: 'block',
            padding: '5px 14px',
            color: 'var(--muted)',
            textDecoration: 'none',
            fontSize: 13,
          }}
        >
          Onboarding
        </Link>

        {/* Account button + menu */}
        <div ref={menuRef} style={{ position: 'relative' }}>
          <button
            type="button"
            data-testid="account-btn"
            className="avatar"
            onClick={() => setShowMenu((v) => !v)}
            title={userSubject ?? 'User menu'}
            aria-haspopup="true"
            aria-expanded={showMenu}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              width: '100%',
              padding: '5px 14px',
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              color: 'var(--muted)',
              fontSize: 13,
              textAlign: 'left',
            }}
          >
            <span
              style={{
                width: 22,
                height: 22,
                borderRadius: 3,
                background: 'var(--surface-2)',
                border: '1px solid var(--border-2)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: 11,
                fontWeight: 700,
                color: 'var(--accent)',
                flexShrink: 0,
              }}
            >
              {label ?? '⬡'}
            </span>
            <span
              style={{
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                fontSize: 11,
                color: 'var(--muted)',
              }}
            >
              {label ? userSubject : 'Account'}
            </span>
          </button>

          {showMenu && (
            <div
              className="avatar-dropdown"
              role="menu"
              style={{
                position: 'absolute',
                bottom: '100%',
                left: 8,
                right: 8,
                background: 'var(--surface)',
                border: '1px solid var(--border-2)',
                borderRadius: 3,
                zIndex: 100,
                padding: '4px 0',
              }}
            >
              {userSubject && (
                <div
                  className="avatar-subject"
                  style={{
                    padding: '4px 12px',
                    fontSize: 11,
                    color: 'var(--muted)',
                    borderBottom: '1px solid var(--border)',
                    marginBottom: 4,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {userSubject}
                </div>
              )}
              <button
                type="button"
                role="menuitem"
                onClick={toggleTheme}
                style={{
                  display: 'block',
                  width: '100%',
                  padding: '5px 12px',
                  background: 'transparent',
                  border: 'none',
                  cursor: 'pointer',
                  color: 'var(--muted)',
                  fontSize: 12,
                  textAlign: 'left',
                }}
              >
                {theme === 'dark' ? '☀ Light mode' : '☾ Dark mode'}
              </button>
              <button
                type="button"
                role="menuitem"
                data-testid="logout-item"
                className="logout-item"
                onClick={async () => {
                  setShowMenu(false);
                  await logout();
                }}
                style={{
                  display: 'block',
                  width: '100%',
                  padding: '5px 12px',
                  background: 'transparent',
                  border: 'none',
                  cursor: 'pointer',
                  color: 'var(--err)',
                  fontSize: 12,
                  textAlign: 'left',
                }}
              >
                Logout
              </button>
            </div>
          )}
        </div>
      </div>
    </nav>
  );
}

export default Sidebar;
