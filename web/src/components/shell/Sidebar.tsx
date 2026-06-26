import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useAuthStore } from '@/stores/auth';
import { useQuery } from '@tanstack/react-query';
import { Link, useRouterState } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { NAV_GROUPS, isNavItemActive } from './navModel';

type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];

// ── Helpers ─────────────────────────────────────────────────────────────────

export function avatarLabel(email: string | null): string | null {
  if (!email) return null;
  if (/^[0-9a-f]{24,}$/i.test(email)) return null;
  const atIdx = email.indexOf('@');
  const local = atIdx > 0 ? email.slice(0, atIdx) : email;
  const parts = local.split(/[._\-\s]+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  if (parts.length === 1 && /^[a-zA-Z]+$/.test(parts[0])) return parts[0][0].toUpperCase();
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
  '/nodes': '▦',
  '/domains': '▤',
  '/feed': '≋',
  '/admin': '⚙',
};

// ── Inline Domains tree ──────────────────────────────────────────────────────

function SidebarDomainsSection({ pathname }: { pathname: string }) {
  const isActive = isNavItemActive('/domains', pathname);
  const { data: tree } = useQuery({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree', {})),
    refetchInterval: 30_000,
  });

  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  const toggle = (id: string) =>
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div>
      {/* DOMAINS section label */}
      <Link
        to="/domains"
        data-testid="nav-/domains"
        aria-current={pathname === '/domains' ? 'page' : undefined}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 9,
          height: 28,
          padding: '0 8px',
          borderRadius: 6,
          color: isActive ? 'var(--text)' : 'var(--text-2)',
          fontSize: 13,
          fontWeight: 510,
          textDecoration: 'none',
          background: pathname === '/domains' ? 'var(--raised-2)' : 'transparent',
          userSelect: 'none',
        }}
      >
        <span
          style={{
            width: 15,
            height: 15,
            flexShrink: 0,
            color: isActive ? 'var(--accent)' : 'var(--dim)',
            display: 'grid',
            placeItems: 'center',
            fontSize: 13,
          }}
        >
          {NAV_ICON['/domains']}
        </span>
        Domains
      </Link>

      {/* Domain tree items */}
      {tree?.domains.map((domain: ExplorerDomainNode) => {
        const domainActive = pathname.startsWith(`/domains/${domain.id}`);
        const expanded = expandedIds.has(domain.id);
        return (
          <div key={domain.id}>
            <div style={{ display: 'flex', alignItems: 'center' }}>
              <button
                type="button"
                aria-label={`${expanded ? 'Collapse' : 'Expand'} ${domain.name}`}
                aria-expanded={expanded}
                onClick={() => toggle(domain.id)}
                style={{
                  background: 'none',
                  border: 'none',
                  color: 'var(--dim)',
                  fontSize: 10,
                  cursor: 'pointer',
                  padding: '0 2px 0 20px',
                  flexShrink: 0,
                }}
              >
                {expanded ? '▾' : '▸'}
              </button>
              <Link
                to="/domains/$domainId"
                params={{ domainId: domain.id }}
                style={{
                  flex: 1,
                  display: 'flex',
                  alignItems: 'center',
                  height: 26,
                  padding: '0 8px 0 2px',
                  borderRadius: 5,
                  fontSize: 12,
                  color: domainActive ? 'var(--text)' : 'var(--text-2)',
                  textDecoration: 'none',
                  fontWeight: domainActive ? 510 : 400,
                  background: domainActive ? 'var(--raised-2)' : 'transparent',
                }}
              >
                {domain.name}
              </Link>
            </div>
            {expanded &&
              domain.missions.map((m: ExplorerMissionNode) => {
                const mActive = pathname.includes(`/missions/${m.id}`);
                return (
                  <Link
                    key={m.id}
                    to="/domains/$domainId/missions/$missionId"
                    params={{ domainId: domain.id, missionId: m.id }}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      height: 24,
                      padding: '0 8px 0 38px',
                      borderRadius: 5,
                      fontSize: 11,
                      color: mActive ? 'var(--text)' : 'var(--text-2)',
                      textDecoration: 'none',
                      background: mActive ? 'var(--raised-2)' : 'transparent',
                    }}
                  >
                    <span
                      style={{
                        width: 6,
                        height: 6,
                        borderRadius: '50%',
                        background: 'var(--dim)',
                        marginRight: 6,
                        flexShrink: 0,
                      }}
                    />
                    {m.name}
                  </Link>
                );
              })}
          </div>
        );
      })}
    </div>
  );
}

// ── Sidebar ──────────────────────────────────────────────────────────────────

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const userSubject = useAuthStore((s) => s.userSubject);
  const userEmail = useAuthStore((s) => s.userEmail);
  const userName = useAuthStore((s) => s.userName);
  const isAdmin = useAuthStore((s) => s.isAdmin);
  const logout = useAuthStore((s) => s.logout);

  const [showMenu, setShowMenu] = useState(false);
  const [_theme, setTheme] = useState('dark');
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const saved = localStorage.getItem('edgeplane:theme');
    const initial = saved === 'light' ? 'light' : 'dark';
    setTheme(initial);
    applyTheme(initial);
  }, []);

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

  const label = avatarLabel(userName ?? userEmail);

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
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px 10px' }}>
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

      {/* Search */}
      <button
        type="button"
        aria-label="Search"
        onClick={() => {}}
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
      <div style={{ display: 'flex', flexDirection: 'column', gap: 1, flex: 1, overflowY: 'auto' }}>
        {NAV_GROUPS.flatMap((g) => g.items).map((item) => {
          if (item.to === '/domains') {
            return <SidebarDomainsSection key="/domains" pathname={pathname} />;
          }
          if (item.adminOnly && !isAdmin) return null;
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
                if (!active) (e.currentTarget as HTMLElement).style.background = 'var(--raised)';
              }}
              onMouseLeave={(e) => {
                if (!active) (e.currentTarget as HTMLElement).style.background = 'transparent';
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
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'stretch',
            }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === 'Escape') setShowMenu(false);
            }}
          >
            <button
              type="button"
              role="menuitem"
              data-testid="menu-preferences"
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'flex-start',
                gap: 8,
                padding: '6px 8px',
                borderRadius: 5,
                fontSize: 13,
                color: 'var(--text-2)',
                cursor: 'pointer',
                width: '100%',
                background: 'transparent',
                border: 'none',
                fontFamily: 'var(--font)',
              }}
            >
              Preferences
            </button>
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
                width: '100%',
                boxSizing: 'border-box',
              }}
            >
              Onboarding
            </Link>
            <button
              type="button"
              role="menuitem"
              data-testid="theme-item"
              onClick={toggleTheme}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'flex-start',
                gap: 8,
                padding: '6px 8px',
                borderRadius: 5,
                fontSize: 13,
                color: 'var(--text-2)',
                cursor: 'pointer',
                width: '100%',
                background: 'transparent',
                border: 'none',
                fontFamily: 'var(--font)',
              }}
            >
              ☾ Theme
            </button>
            <button
              type="button"
              role="menuitem"
              data-testid="logout-item"
              onClick={async () => {
                setShowMenu(false);
                await logout();
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'flex-start',
                gap: 8,
                padding: '6px 8px',
                borderRadius: 5,
                fontSize: 13,
                color: 'var(--err)',
                cursor: 'pointer',
                width: '100%',
                background: 'transparent',
                border: 'none',
                fontFamily: 'var(--font)',
              }}
            >
              Logout
            </button>
          </div>
        )}
        <button
          type="button"
          data-testid="account-btn"
          aria-haspopup="menu"
          aria-expanded={showMenu}
          onClick={() => setShowMenu((v) => !v)}
          title={userName ?? userEmail ?? userSubject ?? 'User menu'}
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
          }}
        >
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
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {userName ?? userEmail ?? 'Account'}
          </span>
          <span style={{ marginLeft: 'auto', color: 'var(--dim)', fontSize: 11 }}>⌄</span>
        </button>
      </div>
    </nav>
  );
}

export default Sidebar;
