export interface Crumb { label: string; to: string | undefined; }

const SECTION_LABEL: Record<string, string> = {
  '/': 'Dashboard',
  '/agents': 'Agents',
  '/domains': 'Domains',
  '/feed': 'Feed',
  '/governance': 'Governance',
  '/onboarding': 'Onboarding',
};

/** Build a crumb trail for the current path. `params` supplies detail-id labels. */
export function buildCrumbs(pathname: string, params: Record<string, string>): Crumb[] {
  if (pathname.startsWith('/agents/') && params.agentId) {
    return [
      { label: 'Agents', to: '/agents' },
      { label: params.agentId, to: undefined },
    ];
  }
  const label = SECTION_LABEL[pathname];
  if (label) return [{ label, to: undefined }];
  return [];
}
