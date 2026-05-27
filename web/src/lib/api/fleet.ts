/**
 * Fleet resolution helper.
 *
 * Resolves all registered agents to their node_id + agent_id mappings
 * for the fleet dashboard. Uses the same /runtime/nodes endpoint chain
 * as the agent detail page but fetches all agents in one pass.
 */

import { api } from './client';

export interface FleetAgent {
	nodeId: string;
	agentId: string;
	name: string;
	status: string;
	runtimeKind: string;
}

type RuntimeNode = { id: string; node_name?: string };
type MeshAgent = {
	id: string;
	public_id?: string;
	agent_public_id?: string;
	name?: string;
	status?: string;
	runtime_kind?: string;
};

let cached: FleetAgent[] | null = null;
let cacheTime = 0;
const CACHE_TTL_MS = 30_000;

export async function resolveFleetAgents(force = false): Promise<FleetAgent[]> {
	const now = Date.now();
	if (!force && cached && now - cacheTime < CACHE_TTL_MS) return cached;

	const nodes = await api.get<RuntimeNode[]>('/runtime/nodes');
	const result: FleetAgent[] = [];

	for (const node of nodes) {
		const agents = await api.get<MeshAgent[]>(
			`/runtime/nodes/${node.id}/agents`
		).catch(() => [] as MeshAgent[]);

		for (const a of agents) {
			result.push({
				nodeId: node.id,
				agentId: a.public_id ?? a.agent_public_id ?? a.id,
				name: a.name ?? a.public_id ?? a.id,
				status: a.status ?? 'unknown',
				runtimeKind: a.runtime_kind ?? 'unknown',
			});
		}
	}

	cached = result;
	cacheTime = now;
	return result;
}

/** Extract a short profile name from agent IDs like "aria-operator-e8820c0d". */
export function profileName(agentId: string): string {
	const parts = agentId.split('-');
	if (parts.length >= 3 && parts[0] === 'aria') {
		return parts.slice(1, -1).join('-');
	}
	return agentId;
}
