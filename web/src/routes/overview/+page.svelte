<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { useAuthState } from '$lib/stores/auth-state.svelte';
	import { api } from '$lib/api/client';
	import { fetchTree } from '$lib/api';
	import { matrixEvents, matrixStatus } from '$lib/telemetry';
	import { queryKeys } from '$lib/queryKeys';

	// ── Types ─────────────────────────────────────────────────────

	type AgentRow = {
		id: number;
		public_id: string;
		name: string;
		status: string;
		capabilities?: string;
		runtime?: string | null;
		node_id?: string | null;
		domain_name?: string | null;
		created_at?: string;
		updated_at?: string;
	};

	type DomainRow = {
		id: string;
		name: string;
		status?: string;
		description?: string;
		missions?: MissionRow[];
	};

	type MissionRow = {
		id: string;
		name: string;
		status?: string;
		domain_id?: string;
	};

	type MatrixEvent = {
		id?: string;
		event?: string;
		type?: string;
		agent_id?: string;
		status?: string;
		payload: unknown;
		receivedAt: number;
	};

	// ── Auth ──────────────────────────────────────────────────────

	const auth = useAuthState();

	// ── Queries ───────────────────────────────────────────────────

	const agentsQuery = createQuery(() => ({
		queryKey: ['agents'],
		queryFn: () => api.get<AgentRow[]>('/agents'),
		enabled: auth.isLoggedIn,
		refetchInterval: 15_000
	}));

	const treeQuery = createQuery(() => ({
		queryKey: queryKeys.explorer.tree(),
		queryFn: () => fetchTree(auth.currentToken || undefined),
		enabled: auth.isLoggedIn,
		refetchInterval: 30_000
	}));

	// ── Derived state ─────────────────────────────────────────────

	let agents = $derived((agentsQuery.data ?? []) as AgentRow[]);
	let domains = $derived(
		((treeQuery.data?.domains ?? []) as unknown[]).map((d) => d as DomainRow)
	);

	// Metrics derived from agent list
	let totalAgents = $derived(agents.length);
	let activeAgents = $derived(agents.filter((a) => a.status === 'online').length);
	let idleAgents = $derived(
		agents.filter((a) => a.status === 'online').length -
		agents.filter((a) => a.status === 'online' && (a.domain_name ?? null) !== null).length
	);
	let offlineAgents = $derived(agents.filter((a) => a.status === 'offline').length);
	let degradedAgents = $derived(agents.filter((a) => a.status === 'degraded').length);

	let totalDomains = $derived(domains.length);

	// Events from SSE store — take latest 40
	let recentEvents = $derived($matrixEvents.slice(0, 40) as MatrixEvent[]);

	// ── Helpers ───────────────────────────────────────────────────

	function statusDot(status: string): { glyph: string; cls: string } {
		switch (status) {
			case 'online':   return { glyph: '⟳', cls: 'accent' };
			case 'offline':  return { glyph: '○', cls: 'dim' };
			case 'degraded': return { glyph: '▲', cls: 'warn' };
			case 'error':    return { glyph: '✗', cls: 'err' };
			default:         return { glyph: '●', cls: 'muted' };
		}
	}

	function runtimeColor(runtime?: string | null): string {
		switch ((runtime ?? '').toLowerCase()) {
			case 'claude':  return 'ok';
			case 'goose':   return 'warn';
			case 'openai':  return 'accent';
			case 'gemini':  return 'purple';
			default:        return 'muted';
		}
	}

	function agentRowClass(status: string): string {
		if (status === 'offline')  return 'agent-row offline';
		if (status === 'degraded') return 'agent-row degraded';
		return 'agent-row';
	}

	function evTypeClass(ev: MatrixEvent): { cls: string; label: string } {
		const t = String(ev.type ?? ev.event ?? '');
		if (t.includes('error') || t.includes('fail')) return { cls: 'ev-type-err', label: t };
		if (t.includes('done') || t.includes('finished') || t.includes('completed')) return { cls: 'ev-type-done', label: t };
		if (t.includes('governance') || t.includes('approval')) return { cls: 'ev-type-gov', label: t };
		if (t.includes('warn')) return { cls: 'ev-type-warn', label: t };
		if (t.includes('started') || t.includes('claimed') || t.includes('heartbeat')) return { cls: 'ev-type-info', label: t };
		return { cls: 'ev-type-ok', label: t };
	}

	function evRowClass(ev: MatrixEvent): string {
		const t = String(ev.type ?? ev.event ?? '');
		if (t.includes('error') || t.includes('fail')) return 'ev-row alert-err';
		if (t.includes('governance') || t.includes('approval')) return 'ev-row alert-gov';
		if (t.includes('warn')) return 'ev-row alert-warn';
		return 'ev-row';
	}

	function evTime(ts: number): string {
		return new Date(ts).toLocaleTimeString('en-US', { hour12: false });
	}

	function evBody(ev: MatrixEvent): string {
		const agentPart = ev.agent_id ? ` · ${ev.agent_id}` : '';
		const payload = ev.payload;
		let detail = '';
		if (payload && typeof payload === 'object') {
			const p = payload as Record<string, unknown>;
			detail = String(p.message ?? p.description ?? p.title ?? p.task_id ?? '');
		} else if (typeof payload === 'string') {
			detail = payload as string;
		}
		return `${agentPart}${detail ? ` · ${detail}` : ''}`;
	}

	function domainStatusDot(status?: string): string {
		switch (status) {
			case 'active':   return '◈';
			case 'archived': return '◉';
			default:         return '◈';
		}
	}

	function domainDotClass(status?: string): string {
		return status === 'active' ? 'accent' : 'muted';
	}

	// selected mission slug for left pane highlight
	let selectedDomainId = $state<string | null>(null);
</script>

<!-- Metrics strip -->
<div id="metrics">
	<div class="metric-cell">
		<div class="metric-lbl">Agents</div>
		<div class="metric-val" style="color:var(--text)">{totalAgents}</div>
		<div class="metric-sub">
			{#if offlineAgents > 0}
				<span class="err">{offlineAgents} offline</span>
			{:else if degradedAgents > 0}
				<span class="warn">▲ {degradedAgents} degraded</span>
			{:else}
				<span class="ok">all healthy</span>
			{/if}
		</div>
	</div>
	<div class="metric-cell">
		<div class="metric-lbl">Active</div>
		<div class="metric-val accent">{activeAgents}</div>
		<div class="metric-sub">⟳ running</div>
	</div>
	<div class="metric-cell">
		<div class="metric-lbl">Idle</div>
		<div class="metric-val muted">{idleAgents < 0 ? 0 : idleAgents}</div>
		<div class="metric-sub">online · no task</div>
	</div>
	<div class="metric-cell">
		<div class="metric-lbl">Domains</div>
		<div class="metric-val" style="color:var(--purple)">{totalDomains}</div>
		<div class="metric-sub">org boundaries</div>
	</div>
	<div class="metric-cell">
		<div class="metric-lbl">Events</div>
		<div class="metric-val" style="color:var(--text)">{recentEvents.length}</div>
		<div class="metric-sub dim">buffered · live</div>
	</div>
	<div class="metric-cell">
		<div class="metric-lbl">Stream</div>
		<div class="metric-val" class:ok={$matrixStatus.connected} class:err={!$matrixStatus.connected}>
			{$matrixStatus.connected ? '●' : '○'}
		</div>
		<div class="metric-sub">
			{#if $matrixStatus.connected}
				<span class="ok live">live</span>
			{:else}
				<span class="muted">disconnected</span>
			{/if}
		</div>
	</div>
</div>

<!-- Three-pane content area -->
<div id="content">

	<!-- Left: Domains / Missions pane -->
	<div id="pane-missions">
		<div class="ph">
			<span class="t">Domains</span>
			<span class="c">{totalDomains}</span>
		</div>
		<div class="pane-scroll">
			{#if treeQuery.isLoading}
				<div class="empty-state muted">Loading…</div>
			{:else if treeQuery.isError}
				<div class="empty-state err">Failed to load</div>
			{:else if domains.length === 0}
				<div class="empty-state dim">No domains yet.</div>
			{:else}
				{#each domains as domain (domain.id)}
					<!-- svelte-ignore a11y_click_events_have_key_events -->
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="m-row"
						class:sel={selectedDomainId === domain.id}
						onclick={() => { selectedDomainId = selectedDomainId === domain.id ? null : domain.id; }}
					>
						<div class="m-name">
							<span class={domainDotClass(domain.status)}>{domainStatusDot(domain.status)}</span>
							{domain.name}
						</div>
						{#if domain.missions?.length}
							<div class="m-meta">{domain.missions.length} missions</div>
						{:else}
							<div class="m-meta dim">no missions</div>
						{/if}
					</div>
					{#if selectedDomainId === domain.id && domain.missions?.length}
						<div class="slbl">Missions</div>
						{#each domain.missions as mission (mission.id)}
							<div class="m-sub-row">
								<span class="accent">↳</span> {mission.name}
								{#if mission.status}
									<span class="dim">· {mission.status}</span>
								{/if}
							</div>
						{/each}
					{/if}
				{/each}
			{/if}
		</div>
	</div>

	<!-- Center: Agent table -->
	<div id="pane-agents">
		<div class="ph">
			<span class="t">Agents</span>
			<span class="c">{totalAgents} total · {activeAgents} active</span>
		</div>
		<div class="agent-thead">
			<span></span>
			<span>Agent</span>
			<span>Node</span>
			<span>Runtime</span>
			<span>Domain</span>
			<span>Status</span>
			<span style="text-align:right">Updated</span>
		</div>
		<div class="agent-tbody">
			{#if agentsQuery.isLoading}
				<div class="empty-state muted">Loading…</div>
			{:else if agentsQuery.isError}
				<div class="empty-state err">Failed to load agents</div>
			{:else if agents.length === 0}
				<div class="empty-state dim">No agents registered.</div>
			{:else}
				{#each agents as agent (agent.id)}
					{@const dot = statusDot(agent.status)}
					{@const rtCls = runtimeColor(agent.runtime)}
					<div class={agentRowClass(agent.status)}>
						<span class={dot.cls}>{dot.glyph}</span>
						<span class="ar-name" class:accent={agent.status === 'online'}>{agent.name}</span>
						<span class="ar-node">{agent.node_id ?? '—'}</span>
						<span class="ar-runtime {rtCls}">{agent.runtime ?? '—'}</span>
						<span class="ar-mission">{agent.domain_name ?? '—'}</span>
						<span class="ar-task" class:idle={agent.status !== 'online'}>{agent.status}</span>
						<span class="ar-uptime">
							{#if agent.updated_at}
								{new Date(agent.updated_at).toLocaleTimeString('en-US', { hour12: false })}
							{:else}
								—
							{/if}
						</span>
					</div>
				{/each}
			{/if}
		</div>
	</div>

	<!-- Right: Events pane -->
	<div id="pane-right">
		<div id="pane-events">
			<div class="ph">
				<span class="t">Recent Events</span>
				<span style="display:flex;align-items:center;gap:5px">
					{#if $matrixStatus.connected}
						<span class="ok live">●</span>
						<span class="c ok" style="font-size:11px">LIVE</span>
					{:else}
						<span class="dim">○</span>
						<span class="c dim" style="font-size:11px">offline</span>
					{/if}
				</span>
			</div>
			<div class="events-scroll">
				{#if recentEvents.length === 0}
					<div class="empty-state dim">No events yet.</div>
				{:else}
					{#each recentEvents as ev (ev.receivedAt)}
						{@const typeInfo = evTypeClass(ev)}
						<div class={evRowClass(ev)}>
							<span class="ev-time">{evTime(ev.receivedAt)}</span>
							<span class="ev-body">
								<span class={typeInfo.cls}>{typeInfo.label || 'event'}</span>{evBody(ev)}
							</span>
						</div>
					{/each}
				{/if}
			</div>
		</div>
	</div>

</div>

<style>
	/* ── Metrics strip ───────────────────────────────────────────── */
	#metrics {
		display: flex;
		flex-shrink: 0;
		background: var(--base);
		border-bottom: 1px solid var(--border);
	}

	.metric-cell {
		flex: 1;
		padding: 8px 14px;
		border-right: 1px solid var(--border);
	}

	.metric-cell:last-child {
		border-right: none;
	}

	.metric-val {
		font-size: 22px;
		font-weight: 700;
		line-height: 1;
		margin-bottom: 2px;
	}

	.metric-lbl {
		font-size: 10px;
		color: var(--dim);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		margin-bottom: 2px;
	}

	.metric-sub {
		font-size: 11px;
		color: var(--dim);
	}

	/* ── Layout: three-pane content ──────────────────────────────── */
	#content {
		flex: 1;
		display: flex;
		overflow: hidden;
		min-height: 0;
	}

	/* ── Left: missions/domains pane ─────────────────────────────── */
	#pane-missions {
		width: 240px;
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		border-right: 1px solid var(--border);
	}

	.pane-scroll {
		flex: 1;
		overflow-y: auto;
	}

	.m-row {
		padding: 8px 10px;
		border-bottom: 1px solid var(--surface);
		cursor: pointer;
	}

	.m-row:hover {
		background: var(--surface);
	}

	.m-row.sel {
		background: var(--surface-2);
		border-left: 2px solid var(--accent);
	}

	.m-name {
		font-size: 12px;
		font-weight: 700;
		display: flex;
		align-items: center;
		gap: 6px;
		margin-bottom: 3px;
	}

	.m-meta {
		font-size: 11px;
		color: var(--dim);
		padding-left: 14px;
	}

	.m-sub-row {
		padding: 3px 10px 3px 20px;
		font-size: 11px;
		color: var(--muted);
		border-bottom: 1px solid var(--surface);
		line-height: 1.7;
	}

	/* ── Center: agent table ─────────────────────────────────────── */
	#pane-agents {
		flex: 1;
		display: flex;
		flex-direction: column;
		overflow: hidden;
		border-right: 1px solid var(--border);
	}

	.agent-thead {
		display: grid;
		grid-template-columns: 16px 160px 90px 70px 120px 1fr 72px;
		gap: 0 6px;
		padding: 3px 10px;
		background: var(--surface);
		border-bottom: 1px solid var(--border-mid);
		color: var(--dim);
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		flex-shrink: 0;
	}

	.agent-tbody {
		flex: 1;
		overflow-y: auto;
	}

	.agent-row {
		display: grid;
		grid-template-columns: 16px 160px 90px 70px 120px 1fr 72px;
		gap: 0 6px;
		padding: 4px 10px;
		border-bottom: 1px solid var(--surface);
		align-items: center;
		cursor: pointer;
		font-size: 12px;
	}

	.agent-row:hover {
		background: var(--surface);
	}

	.agent-row.offline {
		opacity: 0.45;
	}

	.agent-row.degraded {
		opacity: 0.7;
	}

	.ar-name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.ar-node {
		color: var(--muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.ar-runtime {
		font-size: 11px;
	}

	.ar-mission {
		font-size: 11px;
		color: var(--muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.ar-task {
		font-size: 11px;
		color: var(--accent);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.ar-task.idle {
		color: var(--dim);
	}

	.ar-uptime {
		font-size: 11px;
		color: var(--dim);
		text-align: right;
	}

	/* ── Right: events pane ──────────────────────────────────────── */
	#pane-right {
		width: 320px;
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	#pane-events {
		flex: 1;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.events-scroll {
		flex: 1;
		overflow-y: auto;
	}

	.ev-row {
		display: grid;
		grid-template-columns: 58px 1fr;
		gap: 0 6px;
		padding: 3px 10px;
		border-bottom: 1px solid var(--surface);
		font-size: 11px;
		cursor: pointer;
	}

	.ev-row:hover {
		background: var(--surface);
	}

	.ev-row.alert-err {
		border-left: 2px solid var(--err);
		background: var(--err-bg);
	}

	.ev-row.alert-gov {
		border-left: 2px solid var(--purple);
		background: var(--purple-bg);
	}

	.ev-row.alert-warn {
		border-left: 2px solid var(--warn);
		background: var(--warn-bg);
	}

	.ev-time {
		color: var(--dim);
	}

	.ev-body {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--muted);
	}

	.ev-type-err  { color: var(--err); }
	.ev-type-ok   { color: var(--ok); }
	.ev-type-gov  { color: var(--purple); }
	.ev-type-info { color: var(--accent); }
	.ev-type-done { color: var(--ok); font-weight: 700; }
	.ev-type-warn { color: var(--warn); font-weight: 700; }

	/* ── Shared: empty states ────────────────────────────────────── */
	.empty-state {
		padding: 12px 10px;
		font-size: 11px;
	}
</style>
