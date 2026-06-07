/**
 * useMergedAgents — merge logic unit tests.
 *
 * Covers the control-plane + mesh merge that the former AgentsPage exercised
 * end-to-end. The hook's fetch orchestration is thin glue over apiClient; the
 * interesting behavior is mergeAgents(), tested here in isolation.
 */

import { describe, expect, it } from 'vitest';

import { mergeAgents } from './useMergedAgents';

type CpAgent = Parameters<typeof mergeAgents>[0][number];
type MeshAgent = Parameters<typeof mergeAgents>[1][number];

const cp = (o: Partial<CpAgent>): CpAgent => o as CpAgent;
const mesh = (o: Partial<MeshAgent>): MeshAgent => o as MeshAgent;

describe('mergeAgents', () => {
  it('maps a control-plane-only agent with source "controlplane"', () => {
    const result = mergeAgents(
      [
        cp({
          public_id: 'aria-operator-e8820c0d',
          name: 'aria-operator',
          status: 'online',
          capabilities: 'fleet-management',
          metadata: '{"node_id":"excalibur"}',
          updated_at: '2026-05-31T10:00:00Z',
        }),
      ],
      [],
    );
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({
      public_id: 'aria-operator-e8820c0d',
      name: 'aria-operator',
      status: 'online',
      capabilities: 'fleet-management',
      source: 'controlplane',
    });
  });

  it('augments a matching cp agent with mesh data; mesh status wins, source "both"', () => {
    const result = mergeAgents(
      [
        cp({
          public_id: 'aria-work-aa11bb22',
          name: 'aria-work',
          status: 'offline',
          capabilities: 'work',
          metadata: '{}',
        }),
      ],
      [
        mesh({
          public_id: 'aria-work-aa11bb22',
          status: 'online',
          runtime_kind: 'claude_acp',
          node_name: 'excalibur',
          last_heartbeat_at: '2026-05-31T12:00:00Z',
        }),
      ],
    );
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({
      public_id: 'aria-work-aa11bb22',
      status: 'online', // mesh wins
      source: 'both',
      runtime_kind: 'claude_acp',
      node_name: 'excalibur',
      capabilities: 'work', // kept from controlplane
    });
  });

  it('includes mesh-only agents with source "mesh", name defaulting to public_id', () => {
    const result = mergeAgents(
      [],
      [
        mesh({
          public_id: 'ghost-agent-01',
          status: 'online',
          runtime_kind: 'custom',
          node_name: 'kai',
        }),
      ],
    );
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({
      public_id: 'ghost-agent-01',
      name: 'ghost-agent-01',
      source: 'mesh',
      node_name: 'kai',
    });
  });

  it('preserves registration order (cp agents first, then mesh-only)', () => {
    const result = mergeAgents(
      [
        cp({ public_id: 'zeta', name: 'zeta', status: 'online', capabilities: '' }),
        cp({ public_id: 'alpha', name: 'alpha', status: 'online', capabilities: '' }),
      ],
      [mesh({ public_id: 'mesh-only', status: 'online' })],
    );
    expect(result.map((a) => a.public_id)).toEqual(['zeta', 'alpha', 'mesh-only']);
  });
});
