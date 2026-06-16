import { describe, expect, it } from 'vitest';
import { NAV_GROUPS, isNavItemActive } from './navModel';

describe('navModel', () => {
  it('exposes Dashboard, Agents, Nodes, Domains, Feed, Admin', () => {
    const tos = NAV_GROUPS.flatMap((g) => g.items).map((i) => i.to);
    expect(tos).toEqual(['/', '/agents', '/nodes', '/domains', '/feed', '/admin']);
  });
  it('matches "/" only exactly', () => {
    expect(isNavItemActive('/', '/')).toBe(true);
    expect(isNavItemActive('/', '/agents')).toBe(false);
  });
  it('matches a section by prefix, including detail routes', () => {
    expect(isNavItemActive('/agents', '/agents')).toBe(true);
    expect(isNavItemActive('/agents', '/agents/aria-operator-bb05ea7a')).toBe(true);
    expect(isNavItemActive('/domains', '/domains/apollo')).toBe(true);
    expect(isNavItemActive('/nodes', '/nodes/excalibur-abc')).toBe(true);
    expect(isNavItemActive('/agents', '/agents-foo')).toBe(false);
  });
});
