<script lang="ts">
  import { createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { useAuthState } from '$lib/stores/auth-state.svelte';
  import { queryKeys } from '$lib/queryKeys';
  import { fetchTree, fetchNode } from '$lib/api';
  import AgentTerminal from '$lib/components/AgentTerminal.svelte';

  const queryClient = useQueryClient();
  const auth = useAuthState();

  let searchInput = $state('');
  let selectedNodeType = $state<'domain' | 'mission' | 'task' | null>(null);
  let selectedNodeKey = $state<{ type: 'domain' | 'mission' | 'task'; id: string } | null>(null);
  let selectedDomainData = $state<{ domain: unknown; missions: unknown[]; tasks: unknown[] } | null>(null);

  const treeQuery = createQuery(() => ({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => fetchTree(auth.currentToken || undefined),
    enabled: auth.isLoggedIn
  }));

  const nodeQuery = createQuery(() => ({
    queryKey: selectedNodeKey
      ? queryKeys.explorer.node(selectedNodeKey.type, selectedNodeKey.id)
      : (['__explorer_none__'] as const),
    queryFn: () => fetchNode(selectedNodeKey!.type, selectedNodeKey!.id, auth.currentToken || undefined),
    enabled: auth.isLoggedIn && !!selectedNodeKey
  }));

  let filteredDomains = $derived(
    ((treeQuery.data?.domains ?? []) as unknown[]).filter(
      (d: unknown) => !searchInput || (d as { name?: string }).name?.toLowerCase().includes(searchInput.toLowerCase())
    )
  );

  let lastRefreshed = $derived(
    treeQuery.dataUpdatedAt > 0 ? new Date(treeQuery.dataUpdatedAt).toLocaleTimeString() : ''
  );

  let explorerBusy = $derived(nodeQuery.isFetching && !!selectedNodeKey);

  let selectedNodeData = $derived.by(() => {
    if (selectedNodeType === 'domain') return selectedDomainData;
    return (nodeQuery.data as Record<string, unknown>) ?? null;
  });

  function statusClass(status?: string) {
    const v = String(status ?? '').toLowerCase();
    if (v === 'done' || v === 'completed') return 'status-done';
    if (v === 'blocked') return 'status-blocked';
    if (v === 'in_progress') return 'status-progress';
    return 'status-proposed';
  }

  function statusTagClass(status?: string) {
    const v = String(status ?? '').toLowerCase();
    if (v === 'done' || v === 'completed') return 'ok';
    if (v === 'blocked' || v === 'failed') return 'err';
    if (v === 'in_progress' || v === 'running') return 'accent';
    return 'dim';
  }

  function statusDot(status?: string) {
    const v = String(status ?? '').toLowerCase();
    if (v === 'done' || v === 'completed') return '✓';
    if (v === 'in_progress' || v === 'running') return '⟳';
    if (v === 'blocked' || v === 'failed') return '✗';
    if (v === 'proposed') return '○';
    return '●';
  }

  function taskCountByStatus(tasks: unknown[] = [], status: string) {
    return (tasks as { status?: string }[]).filter(t => String(t.status ?? '').toLowerCase() === status).length;
  }

  function selectExplorerNode(type: 'domain' | 'mission' | 'task', node: unknown) {
    selectedNodeType = type;
    if (type === 'domain') {
      selectedDomainData = {
        domain: node,
        missions: (node as { missions?: unknown[] }).missions ?? [],
        tasks: []
      };
      selectedNodeKey = null;
      return;
    }
    selectedDomainData = null;
    const nodeId =
      type === 'task'
        ? String((node as { public_id?: unknown; id?: unknown }).public_id ?? (node as { id?: unknown }).id ?? '')
        : String((node as { id?: unknown }).id ?? '');
    selectedNodeKey = nodeId ? { type, id: nodeId } : null;
  }

  function selectDomain(d: unknown) { return selectExplorerNode('domain', d); }
  function selectMission(m: unknown) { return selectExplorerNode('mission', m); }
  function selectTask(t: unknown) { return selectExplorerNode('task', t); }

  function refreshTree() {
    queryClient.invalidateQueries({ queryKey: queryKeys.explorer.tree() });
  }

  // Agent terminal drawer — manual node/agent entry MVP. Future work surfaces
  // these as a node type in the tree.
  let terminalOpen = $state(false);
  let terminalNodeId = $state('');
  let terminalAgentId = $state('');
  let activeTerminal = $state<{ nodeId: string; agentId: string } | null>(null);

  function openAgentTerminal() {
    if (!terminalNodeId || !terminalAgentId) return;
    activeTerminal = { nodeId: terminalNodeId, agentId: terminalAgentId };
    terminalOpen = true;
  }

  function closeAgentTerminal() {
    terminalOpen = false;
    activeTerminal = null;
  }
</script>

<div class="explorer-page">

  <!-- top filter bar -->
  <div class="explorer-bar">
    <span class="explorer-bar-title">Explorer</span>
    <input bind:value={searchInput} placeholder="filter domains…" style="width:180px;" />
    <button class="ghost" onclick={refreshTree}>Refresh</button>
    {#if lastRefreshed}<span class="muted" style="font-size:11px;">updated {lastRefreshed}</span>{/if}
  </div>

  <!-- 3-pane layout -->
  <div class="pane-row" style="flex:1; min-height:0;">

    <!-- pane 1: domains -->
    <div class="pane" style="width:220px; flex-shrink:0;">
      <div class="pane-header">
        <span class="pane-title">Domains</span>
        <span class="dim">{filteredDomains.length}</span>
      </div>
      <div class="pane-body">
        {#each filteredDomains as domain}
          <div class="row" role="button" tabindex="0"
            onclick={() => selectDomain(domain)}
            onkeydown={(e) => e.key === 'Enter' && selectDomain(domain)}>
            <span>{statusDot((domain as { status?: string }).status)}</span>
            <span style="flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">{(domain as { name?: string }).name}</span>
            <span class="dim" style="font-size:11px;">{(domain as { missions?: unknown[] }).missions?.length ?? 0}m</span>
          </div>
          {#if (domain as { missions?: unknown[] }).missions?.length}
            {#each (domain as { missions: unknown[] }).missions as mission}
              <div class="row" role="button" tabindex="0" style="padding-left:22px; background:transparent;"
                onclick={() => selectMission(mission)}
                onkeydown={(e) => e.key === 'Enter' && selectMission(mission)}>
                <span class="dim">▸</span>
                <span style="flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:11px;">{(mission as { name?: string }).name}</span>
                <span class="dim" style="font-size:10px;">{(mission as { task_count?: number; recent_tasks?: unknown[] }).task_count ?? (mission as { recent_tasks?: unknown[] }).recent_tasks?.length ?? 0}t</span>
              </div>
            {/each}
          {/if}
        {:else}
          <div class="row"><span class="muted">No domains yet.</span></div>
        {/each}
      </div>
    </div>

    <!-- pane 2: details -->
    <div class="pane" style="flex:1; min-width:0;">
      <div class="pane-header">
        <span class="pane-title">Details</span>
      </div>
      <div class="pane-body" style="padding:10px;">
        {#if explorerBusy}
          <p class="muted">⟳ Loading…</p>
        {:else if selectedNodeData}
          {#if selectedNodeType === 'domain' && (selectedNodeData as { domain?: unknown }).domain}
            {@const d = (selectedNodeData as { domain: { name?: string; status?: string; description?: string; task_count?: number }; missions?: unknown[] })}
            <div style="display:flex; align-items:center; gap:8px; margin-bottom:8px;">
              <strong>{d.domain.name}</strong>
              <span class="tag {statusTagClass(d.domain.status)}">{d.domain.status ?? 'unknown'}</span>
            </div>
            <p class="muted" style="margin:0 0 10px;">{d.domain.description || 'No description.'}</p>
            <div style="display:flex; gap:12px; font-size:11px; color:var(--dim);">
              <span>Missions: <span class="text">{d.missions?.length ?? 0}</span></span>
              <span>Tasks: <span class="text">{d.domain.task_count ?? 0}</span></span>
            </div>
          {:else if selectedNodeType === 'mission' && (selectedNodeData as { mission?: unknown }).mission}
            {@const m = (selectedNodeData as { mission: { name?: string; status?: string; description?: string }; tasks?: unknown[] })}
            <div style="display:flex; align-items:center; gap:8px; margin-bottom:8px;">
              <strong>{m.mission.name}</strong>
              <span class="tag {statusTagClass(m.mission.status)}">{m.mission.status ?? 'unknown'}</span>
            </div>
            <p class="muted" style="margin:0 0 10px;">{m.mission.description || 'No description.'}</p>
            <div style="display:flex; gap:12px; font-size:11px; color:var(--dim); margin-bottom:10px;">
              <span>Tasks: {m.tasks?.length ?? 0}</span>
              <span>In progress: {taskCountByStatus(m.tasks ?? [], 'in_progress')}</span>
              <span>Blocked: {taskCountByStatus(m.tasks ?? [], 'blocked')}</span>
            </div>
            {#each m.tasks ?? [] as task}
              <div class="task-row">
                <div style="display:flex; align-items:center; gap:6px;">
                  <span>{statusDot((task as { status?: string }).status)}</span>
                  <strong style="font-size:12px;">{(task as { title?: string }).title}</strong>
                  <span class="tag {statusTagClass((task as { status?: string }).status)}" style="margin-left:auto;">{(task as { status?: string }).status ?? 'unknown'}</span>
                </div>
                <p class="muted" style="margin:3px 0 0; font-size:11px;">{(task as { description?: string }).description || 'No description.'}</p>
                <button class="ghost" style="margin-top:5px; font-size:11px;" onclick={() => selectTask(task)}>Open Task</button>
              </div>
            {:else}
              <p class="muted">No sub-tasks.</p>
            {/each}
          {:else if selectedNodeType === 'task' && (selectedNodeData as { task?: unknown }).task}
            {@const t = (selectedNodeData as { task: { title?: string; status?: string; description?: string } })}
            <div style="display:flex; align-items:center; gap:8px; margin-bottom:8px;">
              <strong>{t.task.title}</strong>
              <span class="tag {statusTagClass(t.task.status)}">{t.task.status ?? 'unknown'}</span>
            </div>
            <p class="muted" style="margin:0 0 10px;">{t.task.description || 'No description.'}</p>
            <pre style="font-size:11px;">{JSON.stringify(selectedNodeData, null, 2)}</pre>
          {:else}
            <pre style="font-size:11px;">{JSON.stringify(selectedNodeData, null, 2)}</pre>
          {/if}
        {:else}
          <p class="muted">Select a domain or mission.</p>
        {/if}
      </div>
    </div>

    <!-- pane 3: agent terminal -->
    <div class="pane" style="width:320px; flex-shrink:0;">
      <div class="pane-header">
        <span class="pane-title">Agent Terminal</span>
        {#if !terminalOpen}
          <button class="ghost" style="font-size:11px; padding:2px 6px;" onclick={openAgentTerminal} disabled={!terminalNodeId || !terminalAgentId}>Attach</button>
        {:else}
          <button class="ghost" style="font-size:11px; padding:2px 6px;" onclick={closeAgentTerminal}>Detach</button>
        {/if}
      </div>
      <div class="pane-body">
        {#if !terminalOpen}
          <div style="padding:10px; display:flex; flex-direction:column; gap:6px;">
            <input bind:value={terminalNodeId} placeholder="node id (e.g. epyc)" />
            <input bind:value={terminalAgentId} placeholder="agent id" />
          </div>
        {:else if activeTerminal && auth.currentToken}
          <div style="height:100%;">
            <AgentTerminal
              nodeId={activeTerminal.nodeId}
              agentId={activeTerminal.agentId}
              token={auth.currentToken}
            />
          </div>
        {/if}
      </div>
    </div>

  </div>
</div>

<style>
  .explorer-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .explorer-bar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .explorer-bar-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    margin-right: 4px;
  }

  .task-row {
    padding: 7px 0;
    border-bottom: 1px solid var(--border);
  }

  .task-row:last-child {
    border-bottom: none;
  }
</style>
