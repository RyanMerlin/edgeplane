/**
 * useMergedAgents — shared control-plane + mesh agent fetch/merge.
 *
 * Single source of truth for the fleet agent list, consumed by both the Fleet
 * dashboard (conversation tabs) and the Agents table. Previously this fetch +
 * merge logic was duplicated verbatim across routes/index.tsx and
 * routes/agents.tsx; it now lives here.
 *
 * Data sources:
 *   - GET /api/agents                          (control-plane agents)
 *   - GET /api/runtime/nodes                   (runtime nodes for the mesh merge)
 *   - GET /api/runtime/nodes/{node_id}/agents  (NodeMeshAgent list per node)
 *
 * Merge strategy (mirrors the original Svelte fleet page):
 *   - Control-plane agents are the primary source (name, capabilities, status).
 *   - Mesh agents augment with live status and add mesh-only agents if missing.
 *   - Keyed by public_id; mesh status wins on conflict.
 *   - Registration order is preserved (cp agents first, then mesh-only). Ordering
 *     for display (e.g. the alphabetical Agents table) is the consumer's concern.
 *
 * Cadence: refetchInterval 30s for both queries.
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';

// ── Generated schema types ─────────────────────────────────────────────────────

type Agent = components['schemas']['Agent'];
type NodeMeshAgent = components['schemas']['NodeMeshAgent'];
type RuntimeNode = components['schemas']['RuntimeNode'];

// ── Merged agent row ───────────────────────────────────────────────────────────

/** Source of truth for this row. */
export type AgentSource = 'controlplane' | 'mesh' | 'both';

export type MergedAgent = {
  public_id: string;
  name: string;
  status: string;
  capabilities: string;
  source: AgentSource;
  /** Present when source includes controlplane */
  metadata?: string;
  /** Domain name when joined server-side (control-plane only) */
  domain_name?: string | null;
  updated_at?: string;
  /** runtime_kind from mesh layer */
  runtime_kind?: string;
  /** Node this agent is enrolled on — node_name from the mesh topology layer */
  node_name?: string | null;
  /** last_heartbeat_at from mesh layer */
  last_heartbeat_at?: string | null;
};

/** A mesh agent tagged with the node it was fetched from (per-node loop). */
type MeshAgentWithNode = NodeMeshAgent & { node_name?: string | null };

// ── Helpers ────────────────────────────────────────────────────────────────────

function parseCapabilities(raw: string | undefined | unknown): string {
  if (!raw) return '';
  if (typeof raw === 'string') return raw;
  if (Array.isArray(raw)) return raw.join(', ');
  return String(raw);
}

// ── Merge logic ────────────────────────────────────────────────────────────────

/** Merge control-plane + mesh agents (same strategy as the Svelte fleet page). */
export function mergeAgents(cpAgents: Agent[], meshAgents: MeshAgentWithNode[]): MergedAgent[] {
  const byId = new Map<string, MergedAgent>();

  for (const a of cpAgents) {
    byId.set(a.public_id, {
      public_id: a.public_id,
      name: a.name,
      status: a.status,
      capabilities: a.capabilities,
      source: 'controlplane',
      metadata: a.metadata,
      updated_at: a.updated_at,
    });
  }

  for (const a of meshAgents) {
    const pid = a.public_id ?? a.agent_public_id ?? a.id;
    const existing = byId.get(pid);
    if (existing) {
      // Mesh status wins; keep controlplane capabilities/metadata. The mesh
      // layer is the source of truth for the live node + runtime_kind.
      existing.status = a.status;
      existing.source = 'both';
      existing.runtime_kind = a.runtime_kind;
      existing.node_name = a.node_name;
      existing.last_heartbeat_at = a.last_heartbeat_at;
    } else {
      byId.set(pid, {
        public_id: pid,
        name: pid,
        status: a.status,
        capabilities: parseCapabilities(a.capabilities),
        source: 'mesh',
        runtime_kind: a.runtime_kind,
        node_name: a.node_name,
        last_heartbeat_at: a.last_heartbeat_at,
        domain_name: a.domain_name,
      });
    }
  }

  return Array.from(byId.values());
}

// ── Query functions ────────────────────────────────────────────────────────────

async function fetchCpAgents(): Promise<Agent[]> {
  return unwrap(apiClient.GET('/api/agents'));
}

async function fetchMeshAgents(): Promise<MeshAgentWithNode[]> {
  const nodes = await unwrap(apiClient.GET('/api/runtime/nodes'));
  const allMesh: MeshAgentWithNode[] = [];
  await Promise.all(
    nodes.map(async (node: RuntimeNode) => {
      const agents = await unwrap(
        apiClient.GET('/api/runtime/nodes/{node_id}/agents', {
          params: { path: { node_id: node.id } },
        }),
      ).catch(() => [] as NodeMeshAgent[]);
      // Tag each mesh row with the node it was fetched from so the Node column
      // resolves from live topology even when the agent's metadata is empty.
      for (const a of agents) {
        allMesh.push({ ...a, node_name: node.node_name });
      }
    }),
  );
  return allMesh;
}

// ── Hook ─────────────────────────────────────────────────────────────────────

export interface UseMergedAgentsResult {
  agents: MergedAgent[];
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
}

/**
 * Fetch + merge control-plane and mesh agents on a 30s cadence.
 *
 * `isLoading`/`isError` are AND-ed across both queries so the UI shows data as
 * soon as either source resolves and only errors when both fail.
 */
export function useMergedAgents(): UseMergedAgentsResult {
  const cpQuery = useQuery({
    queryKey: queryKeys.agents.list(),
    queryFn: fetchCpAgents,
    refetchInterval: 30_000,
  });

  const meshQuery = useQuery({
    queryKey: [...queryKeys.agents.all, 'mesh'] as const,
    queryFn: fetchMeshAgents,
    refetchInterval: 30_000,
  });

  const agents: MergedAgent[] =
    cpQuery.data !== undefined || meshQuery.data !== undefined
      ? mergeAgents(cpQuery.data ?? [], meshQuery.data ?? [])
      : [];

  return {
    agents,
    isLoading: cpQuery.isLoading && meshQuery.isLoading,
    isError: cpQuery.isError && meshQuery.isError,
    error: (cpQuery.error as Error) ?? (meshQuery.error as Error) ?? null,
  };
}
