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

<div class="glass-panel">
  <div class="grid">
    <div>
      <h3>Domain Tree</h3>
      <button class="ghost" onclick={refreshTree}>Refresh</button>
      {#if lastRefreshed}<small class="muted">Updated {lastRefreshed}</small>{/if}
    </div>
    <input bind:value={searchInput} placeholder="Filter domains..." style="max-width:220px" />
  </div>
  <div class="grid">
    <section class="glass-panel">
      <h4>Domains {filteredDomains.length > 0 ? `(${filteredDomains.length})` : ''}</h4>
      <ul class="explorer-list">
        {#each filteredDomains as domain}
          <li>
            <button class="ghost explorer-node-btn" onclick={() => selectDomain(domain)}>
              <span>{(domain as { name?: string }).name}</span>
              <span class={`status-badge ${statusClass((domain as { status?: string }).status)}`}>{(domain as { status?: string }).status ?? 'unknown'}</span>
            </button>
            {#if (domain as { missions?: unknown[] }).missions?.length}
              <ul class="explorer-sublist">
                {#each (domain as { missions: unknown[] }).missions as mission}
                  <li>
                    <button class="ghost explorer-subnode-btn" onclick={() => selectMission(mission)}>
                      <span>{(mission as { name?: string }).name}</span>
                      <span class="muted">{(mission as { task_count?: number; recent_tasks?: unknown[] }).task_count ?? (mission as { recent_tasks?: unknown[] }).recent_tasks?.length ?? 0} tasks</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {:else}
          <li class="muted">No domains yet.</li>
        {/each}
      </ul>
    </section>
    <section class="glass-panel">
      <h4>Details</h4>
      {#if explorerBusy}
        <p class="muted">Loading node details...</p>
      {:else if selectedNodeData}
        {#if selectedNodeType === 'domain' && (selectedNodeData as { domain?: unknown }).domain}
          {@const d = (selectedNodeData as { domain: { name?: string; status?: string; description?: string; task_count?: number }; missions?: unknown[] })}
          <div class="explorer-detail-header">
            <h4>{d.domain.name}</h4>
            <span class={`status-badge ${statusClass(d.domain.status)}`}>{d.domain.status ?? 'unknown'}</span>
          </div>
          <p class="muted">{d.domain.description || 'No domain description.'}</p>
          <div class="detail-metrics">
            <div class="status-chip">Missions: {d.missions?.length ?? 0}</div>
            <div class="status-chip">Tasks: {d.domain.task_count ?? 0}</div>
          </div>
        {:else if selectedNodeType === 'mission' && (selectedNodeData as { mission?: unknown }).mission}
          {@const m = (selectedNodeData as { mission: { name?: string; status?: string; description?: string }; tasks?: unknown[] })}
          <div class="explorer-detail-header">
            <h4>{m.mission.name}</h4>
            <span class={`status-badge ${statusClass(m.mission.status)}`}>{m.mission.status ?? 'unknown'}</span>
          </div>
          <p class="muted">{m.mission.description || 'No mission description.'}</p>
          <div class="detail-metrics">
            <div class="status-chip">Sub-tasks: {m.tasks?.length ?? 0}</div>
            <div class="status-chip">In Progress: {taskCountByStatus(m.tasks ?? [], 'in_progress')}</div>
            <div class="status-chip">Blocked: {taskCountByStatus(m.tasks ?? [], 'blocked')}</div>
          </div>
          <div class="task-cards">
            {#each m.tasks ?? [] as task}
              <article class="task-card">
                <div class="explorer-detail-header">
                  <strong>{(task as { title?: string }).title}</strong>
                  <span class={`status-badge ${statusClass((task as { status?: string }).status)}`}>{(task as { status?: string }).status ?? 'unknown'}</span>
                </div>
                <p class="muted">{(task as { description?: string }).description || 'No description.'}</p>
                <button class="ghost" onclick={() => selectTask(task)}>Open Task</button>
              </article>
            {:else}
              <p class="muted">No sub-tasks for this mission yet.</p>
            {/each}
          </div>
        {:else if selectedNodeType === 'task' && (selectedNodeData as { task?: unknown }).task}
          {@const t = (selectedNodeData as { task: { title?: string; status?: string; description?: string } })}
          <div class="explorer-detail-header">
            <h4>{t.task.title}</h4>
            <span class={`status-badge ${statusClass(t.task.status)}`}>{t.task.status ?? 'unknown'}</span>
          </div>
          <p class="muted">{t.task.description || 'No task description.'}</p>
          <pre>{JSON.stringify(selectedNodeData, null, 2)}</pre>
        {:else}
          <pre>{JSON.stringify(selectedNodeData, null, 2)}</pre>
        {/if}
      {:else}
        <p class="muted">Choose a domain or mission to inspect.</p>
      {/if}
    </section>
  </div>

  <section class="glass-panel agent-terminal-panel">
    <div class="grid">
      <h4>Agent Terminal</h4>
      {#if !terminalOpen}
        <div class="agent-attach-form">
          <input bind:value={terminalNodeId} placeholder="node id (e.g. vail-epyc)" />
          <input bind:value={terminalAgentId} placeholder="agent id" />
          <button class="primary" onclick={openAgentTerminal} disabled={!terminalNodeId || !terminalAgentId}>
            Attach
          </button>
        </div>
      {:else}
        <button class="ghost" onclick={closeAgentTerminal}>Detach</button>
      {/if}
    </div>
    {#if terminalOpen && activeTerminal && auth.currentToken}
      <div class="agent-terminal-host">
        <AgentTerminal
          nodeId={activeTerminal.nodeId}
          agentId={activeTerminal.agentId}
          token={auth.currentToken}
        />
      </div>
    {/if}
  </section>
</div>

<style>
  .agent-terminal-panel {
    margin-top: 1rem;
  }
  .agent-attach-form {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
  }
  .agent-terminal-host {
    margin-top: 0.75rem;
    height: 480px;
    min-height: 0;
  }
</style>
