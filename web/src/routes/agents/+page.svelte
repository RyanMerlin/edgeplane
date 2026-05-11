<script lang="ts">
	import { useAuthState } from '$lib/stores/auth-state.svelte';
	import { api } from '$lib/api/client';

	type AgentRow = {
		id: number;
		public_id: string;
		name: string;
		status: string;
		capabilities?: string;
	};

	const auth = useAuthState();

	let agents = $state<AgentRow[]>([]);
	let loading = $state(false);
	let error = $state<string | null>(null);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			agents = await api.get<AgentRow[]>('/agents');
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
				</tr>
			</thead>
			<tbody>
				{#each agents as a (a.id)}
					<tr>
						<td><span class="status-{a.status}">●</span> {a.status}</td>
						<td>
							<a href={`/agents/${encodeURIComponent(a.public_id)}/`}>{a.public_id}</a>
						</td>
						<td>{a.name}</td>
						<td class="caps">{a.capabilities ?? ''}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</div>

<style>
	.page { padding: 1rem; display: flex; flex-direction: column; gap: 1rem; }
	h1 { margin: 0; }
	.muted { color: var(--muted, #6e7785); margin: 0; }
	table { width: 100%; border-collapse: collapse; font-size: 14px; }
	th, td { text-align: left; padding: 0.5rem 0.75rem; border-bottom: 1px solid var(--border, #1c2230); }
	th { color: var(--muted, #6e7785); font-weight: 600; font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; }
	tr:hover td { background: rgba(255, 174, 87, 0.04); }
	a { color: var(--accent, #ffae57); text-decoration: none; }
	a:hover { text-decoration: underline; }
	.caps { color: var(--muted, #6e7785); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
	.status-online { color: #4ade80; }
	.status-offline { color: #6e7785; }
	.status-error { color: #f87171; }
	.err { padding: 0.75rem; background: rgba(248, 113, 113, 0.1); border: 1px solid rgba(248, 113, 113, 0.3); border-radius: 4px; color: #f87171; }
</style>
