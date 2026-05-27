<script lang="ts">
	import { page } from '$app/state';
	import { useAuthState } from '$lib/stores/auth-state.svelte';
	import { api } from '$lib/api/client';
	import AgentConversation from '$lib/components/AgentConversation.svelte';

	const auth = useAuthState();

	// `agentId` is the public_id (e.g. `aria-operator-e8820c0d`) — what
	// edgeplaned polls. It can also be the numeric row id; AgentIdent handles
	// both at the controlplane.
	const agentId = $derived(page.params.agentId);

	// Resolve which node hosts this agent. Scans `/runtime/nodes` and each
	// node's agent list for a row whose public_id matches. The CLI does the
	// same lookup in `edgeplane agent attach`.
	type RuntimeNode = { id: string; node_name?: string };
	type MeshAgent = {
		id: string;
		public_id?: string;
		agent_public_id?: string;
	};

	let nodeId = $state<string | null>(null);
	let nodeError = $state<string | null>(null);

	async function resolveNode(): Promise<void> {
		nodeError = null;
		try {
			const nodes = await api.get<RuntimeNode[]>('/runtime/nodes');
			for (const n of nodes) {
				const agents = await api.get<MeshAgent[]>(`/runtime/nodes/${n.id}/agents`).catch(() => []);
				const hit = agents.find(
					(a) =>
						a.public_id === agentId ||
						a.agent_public_id === agentId ||
						a.id === agentId
				);
				if (hit) {
					nodeId = n.id;
					return;
				}
			}
			nodeError = `agent ${agentId} not found on any registered node`;
		} catch (err) {
			nodeError = err instanceof Error ? err.message : String(err);
		}
	}

	$effect(() => {
		if (auth.isLoggedIn && agentId) {
			nodeId = null;
			resolveNode();
		}
	});
</script>

<div class="page">
	<header class="page-header">
		<h1>{agentId}</h1>
		<p class="muted">Persistent ACP session</p>
	</header>

	{#if !auth.isLoggedIn}
		<p>Sign in to view this agent.</p>
	{:else if nodeError}
		<div class="err">
			<p><strong>Couldn't locate {agentId}</strong></p>
			<p>{nodeError}</p>
		</div>
	{:else if !nodeId}
		<p class="muted">Locating node…</p>
	{:else if auth.currentToken}
		<div class="conv-wrap">
			<AgentConversation
				{nodeId}
				{agentId}
				token={auth.currentToken}
			/>
		</div>
	{:else}
		<p class="muted">No token available — refreshing auth…</p>
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		gap: 6px;
		padding: 10px 12px;
	}
	.page-header h1 {
		margin: 0;
		font-size: 13px;
		font-family: inherit;
		color: var(--accent);
	}
	.muted {
		margin: 0;
		color: var(--muted);
		font-size: 12px;
	}
	.conv-wrap {
		flex: 1;
		min-height: 0;
	}
	.err {
		padding: 8px 10px;
		background: var(--err-bg);
		border: 1px solid var(--err-border);
		border-radius: 2px;
		color: var(--err);
		font-size: 12px;
	}
	.err p { margin: 3px 0; }
</style>
