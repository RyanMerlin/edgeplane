import { describe, expect, it } from 'vitest';
import { buildCrumbs } from './breadcrumbs';

describe('buildCrumbs', () => {
  it('root → single Dashboard crumb (no link)', () => {
    expect(buildCrumbs('/', {})).toEqual([{ label: 'Dashboard', to: undefined }]);
  });
  it('agents list → Agents (current)', () => {
    expect(buildCrumbs('/agents', {})).toEqual([{ label: 'Agents', to: undefined }]);
  });
  it('agent detail → Agents (link) › id (current)', () => {
    expect(buildCrumbs('/agents/aria-operator-bb05ea7a', { agentId: 'aria-operator-bb05ea7a' })).toEqual([
      { label: 'Agents', to: '/agents' },
      { label: 'aria-operator-bb05ea7a', to: undefined },
    ]);
  });
  it('domains → Domains (current)', () => {
    expect(buildCrumbs('/domains', {})).toEqual([{ label: 'Domains', to: undefined }]);
  });
  it('feed → Feed (current)', () => {
    expect(buildCrumbs('/feed', {})).toEqual([{ label: 'Feed', to: undefined }]);
  });
});
