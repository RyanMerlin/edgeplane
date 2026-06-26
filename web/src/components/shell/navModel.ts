export interface NavItem {
  to: string;
  label: string;
  adminOnly?: boolean;
}
export interface NavGroup {
  heading: string | null;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  { heading: null, items: [{ to: '/', label: 'Dashboard' }] },
  { heading: null, items: [{ to: '/agents', label: 'Agents' }] },
  { heading: null, items: [{ to: '/nodes', label: 'Nodes' }] },
  { heading: null, items: [{ to: '/domains', label: 'Domains' }] },
  { heading: null, items: [{ to: '/feed', label: 'Feed' }] },
  { heading: null, items: [{ to: '/admin', label: 'Admin', adminOnly: true }] },
];

/** Active when pathname equals the item ("/" exact) or is a path-boundary descendant. */
export function isNavItemActive(itemTo: string, pathname: string): boolean {
  if (itemTo === '/') return pathname === '/';
  return pathname === itemTo || pathname.startsWith(`${itemTo}/`);
}
