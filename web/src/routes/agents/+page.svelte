<script lang="ts">
	import { useAuthState } from '$lib/stores/auth-state.svelte';
	import { api } from '$lib/api/client';
	import { resolveFleetAgents, type FleetAgent } from '$lib/api/fleet';
	import { base } from '$app/paths';

	type AgentRow = {
		id: number;
		public_id: string;
		name: string;
		status: string;
		capabilities?: string;
	};

	type MergedAgent = {
		public_id: string;
		name: string;
		status: string;
		capabilities?: string;
		source: 'controlplane' | 'mesh' | 'both';
	};

	const auth = useAuthState();

	let agents = $state<MergedAgent[]>([]);
	let loading = $state(false);
	let error = $state<string | null>(null);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [cpAgents, meshAgents] = await Promise.all([
				api.get<AgentRow[]>('/agents').catch((): AgentRow[] => []),
				resolveFleetAgents().catch((): FleetAgent[] => [])
			]);

			// Build a map keyed by public_id — mesh data is preferred for status
			const byId = new Map<string, MergedAgent>();

			for (const a of cpAgents) {
				byId.set(a.public_id, {
					public_id: a.public_id,
					name: a.name,
					status: a.status,
					capabilities: a.capabilities,
					source: 'controlplane'
				});
			}

			for (const a of meshAgents) {
				const existing = byId.get(a.agentId);
				if (existing) {
					// Prefer mesh status; keep controlplane capabilities
					existing.status = a.status;
					existing.source = 'both';
				} else {
					byId.set(a.agentId, {
						public_id: a.agentId,
						name: a.name,
						status: a.status,
						capabilities: a.runtimeKind,
						source: 'mesh'
					});
				}
			}

			agents = Array.from(byId.values()).sort((a, b) => a.name.localeCompare(b.name));
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (auth.isLoggedIn) load();
	});
</script>

<div class="page">
	<header>
		<h1>Agents</h1>
		<p class="muted">Click a row to open the live ACP conversation pane.</p>
	</header>

	{#if !auth.isLoggedIn}
		<p>Sign in to view agents.</p>
	{:else if loading}
		<p class="muted">Loading…</p>
	{:else if error}
		<div class="err">{error}</div>
	{:else if agents.length === 0}
		<p class="muted">No agents registered.</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>Status</th>
					<th>Public ID</th>
					<th>Name</th>
					<th>Capabilities</th>
					<th>Source</th>
				</tr>
			</thead>
			<tbody>
				{#each agents as a (a.public_id)}
					<tr>
						<td><span class="status-{a.status}">●</span> {a.status}</td>
						<td>
							<a href={`${base}/agents/${encodeURIComponent(a.public_id)}/`}>{a.public_id}</a>
						</td>
						<td>{a.name}</td>
						<td class="caps">{a.capabilities ?? ''}</td>
						<td class="source source-{a.source}">{a.source}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</div>

<style>
	.page { padding: 10px 12px; display: flex; flex-direction: column; gap: 8px; height: 100%; overflow: hidden; }
	h1 { margin: 0; font-size: 14px; }
	.muted { color: var(--muted); margin: 0; }
	table { width: 100%; border-collapse: collapse; font-size: 12px; }
	th, td { text-align: left; padding: 4px 10px; border-bottom: 1px solid var(--border); }
	th { color: var(--dim); font-weight: 400; font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em; background: var(--surface); }
	tr:hover td { background: var(--surface-2); }
	a { color: var(--accent); text-decoration: none; }
	a:hover { text-decoration: underline; }
	.caps { color: var(--muted); font-size: 11px; }
	.source { font-size: 10px; color: var(--dim); }
	.source-mesh { color: var(--accent); }
	.source-both { color: var(--ok); }
	.status-online { color: var(--ok); }
	.status-active { color: var(--ok); }
	.status-offline { color: var(--dim); }
	.status-error { color: var(--err); }
	.err { padding: 8px 10px; background: var(--err-bg); border: 1px solid var(--err-border); border-radius: 2px; color: var(--err); font-size: 12px; }
</style>
