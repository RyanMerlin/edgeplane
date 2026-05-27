<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { useAuthState } from '$lib/stores/auth-state.svelte';
	import { resolveFleetAgents, profileName, type FleetAgent } from '$lib/api/fleet';
	import AgentTerminal from '$lib/components/AgentTerminal.svelte';

	const auth = useAuthState();

	const FLEET_PROFILES = ['operator', 'engineer', 'merlinlabs', 'publisher', 'work', 'research'];

	let agents = $state<FleetAgent[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let activeProfile = $state<string | null>(null);
	let viewMode = $state<'terminal' | 'conversation'>('terminal');

	function agentForProfile(profile: string): FleetAgent | undefined {
		return agents.find((a) => profileName(a.agentId) === profile);
	}

	let activeAgent = $derived(activeProfile ? agentForProfile(activeProfile) : undefined);

	async function load(force = false): Promise<void> {
		loading = true;
		error = null;
		try {
			agents = await resolveFleetAgents(force);
			if (!activeProfile && agents.length > 0) {
				const first = FLEET_PROFILES.find((p) => agentForProfile(p));
				activeProfile = first ?? profileName(agents[0].agentId);
			}
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			loading = false;
		}
	}

	function statusColor(status: string): string {
		switch (status) {
			case 'online':
			case 'active':
				return '#4ade80';
			case 'working':
				return '#ddc05a';
			case 'error':
				return '#f87171';
			default:
				return '#6e7785';
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (!(e.ctrlKey || e.metaKey)) return;
		const num = parseInt(e.key);
		if (num >= 1 && num <= FLEET_PROFILES.length) {
			e.preventDefault();
			const profile = FLEET_PROFILES[num - 1];
			if (agentForProfile(profile)) {
				activeProfile = profile;
			}
		}
	}

	onMount(() => {
		if (auth.isLoggedIn) load();
		document.addEventListener('keydown', handleKeydown);
	});

	onDestroy(() => {
		document.removeEventListener('keydown', handleKeydown);
	});

	$effect(() => {
		if (auth.isLoggedIn && agents.length === 0) load();
	});
</script>

<div class="page">
	{#if !auth.isLoggedIn}
		<p class="muted">Sign in to view the fleet.</p>
	{:else if loading && agents.length === 0}
		<p class="muted">Loading fleet…</p>
	{:else if error}
		<div class="err">{error}</div>
	{:else}
		<div class="session-tabs">
			{#each FLEET_PROFILES as profile, i}
				{@const agent = agentForProfile(profile)}
				<button
					class="session-tab"
					class:active={activeProfile === profile}
					class:unavailable={!agent}
					onclick={() => agent && (activeProfile = profile)}
					title={agent
						? `${profile} (${agent.status}) — Ctrl+${i + 1}`
						: `${profile} — not registered`}
				>
					<span
						class="dot"
						style="background:{agent ? statusColor(agent.status) : '#333'}"
					></span>
					{profile}
				</button>
			{/each}
			<button class="refresh-btn" onclick={() => load(true)} title="Refresh agents">
				↻
			</button>
		</div>

		<div class="view-tabs">
			<button
				class="view-tab"
				class:active={viewMode === 'terminal'}
				onclick={() => (viewMode = 'terminal')}
			>
				Terminal
			</button>
			<button
				class="view-tab"
				class:active={viewMode === 'conversation'}
				disabled
				title="Available when agent runs on ACP"
			>
				Conversation
			</button>
		</div>

		<div class="view-container">
			{#if activeAgent && auth.currentToken}
				{#if viewMode === 'terminal'}
					{#key activeAgent.agentId}
						<AgentTerminal
							nodeId={activeAgent.nodeId}
							agentId={activeAgent.agentId}
							token={auth.currentToken}
						/>
					{/key}
				{:else}
					<div class="placeholder">
						<p>ACP conversation pane not yet available for ZellijHosted agents.</p>
					</div>
				{/if}
			{:else if activeProfile && !activeAgent}
				<div class="placeholder">
					<p><strong>{activeProfile}</strong> is not registered in EdgePlane.</p>
					<p class="muted">Check that edgeplaned has imported this profile.</p>
				</div>
			{:else}
				<div class="placeholder">
					<p class="muted">Select a profile to attach.</p>
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		padding: 0.75rem;
		gap: 0;
	}

	.session-tabs {
		display: flex;
		gap: 0;
		padding: 0 0.25rem;
		border-bottom: 1px solid var(--panel-border, rgba(255, 255, 255, 0.08));
	}

	.session-tab {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		padding: 0.5rem 0.75rem;
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--muted, #9aa7c4);
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
		font-size: 13px;
		cursor: pointer;
		transition: color 0.15s, border-color 0.15s;
	}

	.session-tab:hover:not(.unavailable) {
		color: var(--text, #e9edf5);
	}

	.session-tab.active {
		color: var(--text, #e9edf5);
		border-bottom-color: var(--accent, #d94a2b);
	}

	.session-tab.unavailable {
		opacity: 0.35;
		cursor: not-allowed;
	}

	.dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		flex-shrink: 0;
	}

	.refresh-btn {
		margin-left: auto;
		background: none;
		border: none;
		color: var(--muted, #9aa7c4);
		font-size: 16px;
		cursor: pointer;
		padding: 0.5rem;
		line-height: 1;
	}

	.refresh-btn:hover {
		color: var(--text, #e9edf5);
	}

	.view-tabs {
		display: flex;
		gap: 0;
		padding: 0 0.25rem;
		margin-top: 0.5rem;
	}

	.view-tab {
		padding: 0.35rem 0.65rem;
		background: none;
		border: 1px solid var(--panel-border, rgba(255, 255, 255, 0.08));
		color: var(--muted, #9aa7c4);
		font-size: 12px;
		cursor: pointer;
		transition: color 0.15s, background 0.15s;
	}

	.view-tab:first-child {
		border-radius: 4px 0 0 4px;
	}

	.view-tab:last-child {
		border-radius: 0 4px 4px 0;
		border-left: none;
	}

	.view-tab.active {
		background: var(--accent-dim, rgba(217, 74, 43, 0.18));
		color: var(--text, #e9edf5);
		border-color: var(--accent, #d94a2b);
	}

	.view-tab:disabled {
		opacity: 0.35;
		cursor: not-allowed;
	}

	.view-container {
		flex: 1;
		min-height: 0;
		margin-top: 0.5rem;
		border-radius: 6px;
		overflow: hidden;
	}

	.placeholder {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		height: 100%;
		color: var(--muted, #9aa7c4);
		font-size: 14px;
		text-align: center;
		gap: 0.25rem;
	}

	.placeholder p {
		margin: 0;
	}

	.muted {
		color: var(--muted, #9aa7c4);
	}

	.err {
		padding: 0.75rem;
		background: rgba(248, 113, 113, 0.1);
		border: 1px solid rgba(248, 113, 113, 0.3);
		border-radius: 4px;
		color: #f87171;
	}
</style>
