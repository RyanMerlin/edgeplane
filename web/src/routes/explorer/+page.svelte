<script lang="ts">
  import { createQuery, useQueryClient } from '@tanstack/svelte-query';
  import { useAuthState } from '$lib/stores/auth-state.svelte';
  import { queryKeys } from '$lib/queryKeys';
  import { fetchTree, fetchNode } from '$lib/api';

  const queryClient = useQueryClient();
  const auth = useAuthState();

  let searchInput = $state('');
  let selectedNodeType = $state<'mission' | 'kluster' | 'task' | null>(null);
  let selectedNodeKey = $state<{ type: 'mission' | 'kluster' | 'task'; id: string } | null>(null);
  let selectedMissionData = $state<{ mission: unknown; klusters: unknown[]; tasks: unknown[] } | null>(null);

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

  let filteredMissions = $derived(
    ((treeQuery.data?.missions ?? []) as unknown[]).filter(
      (m: unknown) => !searchInput || (m as { name?: string }).name?.toLowerCase().includes(searchInput.toLowerCase())
    )
  );

  let lastRefreshed = $derived(
    treeQuery.dataUpdatedAt > 0 ? new Date(treeQuery.dataUpdatedAt).toLocaleTimeString() : ''
  );

  let explorerBusy = $derived(nodeQuery.isFetching && !!selectedNodeKey);

  let selectedNodeData = $derived.by(() => {
    if (selectedNodeType === 'mission') return selectedMissionData;
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

  function selectExplorerNode(type: 'mission' | 'kluster' | 'task', node: unknown) {
    selectedNodeType = type;
    if (type === 'mission') {
      selectedMissionData = {
        mission: node,
        klusters: (node as { klusters?: unknown[] }).klusters ?? [],
        tasks: []
      };
      selectedNodeKey = null;
      return;
    }
    selectedMissionData = null;
    const nodeId =
      type === 'task'
        ? String((node as { public_id?: unknown; id?: unknown }).public_id ?? (node as { id?: unknown }).id ?? '')
        : String((node as { id?: unknown }).id ?? '');
    selectedNodeKey = nodeId ? { type, id: nodeId } : null;
  }

  function selectMission(m: unknown) { return selectExplorerNode('mission', m); }
  function selectKluster(k: unknown) { return selectExplorerNode('kluster', k); }
  function selectTask(t: unknown) { return selectExplorerNode('task', t); }

  function refreshTree() {
    queryClient.invalidateQueries({ queryKey: queryKeys.explorer.tree() });
  }
</script>

<div class="glass-panel">
  <div class="grid">
    <div>
      <h3>Mission Tree</h3>
      <button class="ghost" onclick={refreshTree}>Refresh</button>
      {#if lastRefreshed}<small class="muted">Updated {lastRefreshed}</small>{/if}
    </div>
    <input bind:value={searchInput} placeholder="Filter missions..." style="max-width:220px" />
  </div>
  <div class="grid">
    <section class="glass-panel">
      <h4>Missions {filteredMissions.length > 0 ? `(${filteredMissions.length})` : ''}</h4>
      <ul class="explorer-list">
        {#each filteredMissions as mission}
          <li>
            <button class="ghost explorer-node-btn" onclick={() => selectMission(mission)}>
              <span>{(mission as { name?: string }).name}</span>
              <span class={`status-badge ${statusClass((mission as { status?: string }).status)}`}>{(mission as { status?: string }).status ?? 'unknown'}</span>
            </button>
            {#if (mission as { klusters?: unknown[] }).klusters?.length}
              <ul class="explorer-sublist">
                {#each (mission as { klusters: unknown[] }).klusters as kluster}
                  <li>
                    <button class="ghost explorer-subnode-btn" onclick={() => selectKluster(kluster)}>
                      <span>{(kluster as { name?: string }).name}</span>
                      <span class="muted">{(kluster as { task_count?: number; recent_tasks?: unknown[] }).task_count ?? (kluster as { recent_tasks?: unknown[] }).recent_tasks?.length ?? 0} tasks</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {:else}
          <li class="muted">No missions yet.</li>
        {/each}
      </ul>
    </section>
    <section class="glass-panel">
      <h4>Details</h4>
      {#if explorerBusy}
        <p class="muted">Loading node details...</p>
      {:else if selectedNodeData}
        {#if selectedNodeType === 'mission' && (selectedNodeData as { mission?: unknown }).mission}
          {@const m = (selectedNodeData as { mission: { name?: string; status?: string; description?: string; task_count?: number }; klusters?: unknown[] })}
          <div class="explorer-detail-header">
            <h4>{m.mission.name}</h4>
            <span class={`status-badge ${statusClass(m.mission.status)}`}>{m.mission.status ?? 'unknown'}</span>
          </div>
          <p class="muted">{m.mission.description || 'No mission description.'}</p>
          <div class="detail-metrics">
            <div class="status-chip">Klusters: {m.klusters?.length ?? 0}</div>
            <div class="status-chip">Tasks: {m.mission.task_count ?? 0}</div>
          </div>
        {:else if selectedNodeType === 'kluster' && (selectedNodeData as { kluster?: unknown }).kluster}
          {@const k = (selectedNodeData as { kluster: { name?: string; status?: string; description?: string }; tasks?: unknown[] })}
          <div class="explorer-detail-header">
            <h4>{k.kluster.name}</h4>
            <span class={`status-badge ${statusClass(k.kluster.status)}`}>{k.kluster.status ?? 'unknown'}</span>
          </div>
          <p class="muted">{k.kluster.description || 'No kluster description.'}</p>
          <div class="detail-metrics">
            <div class="status-chip">Sub-tasks: {k.tasks?.length ?? 0}</div>
            <div class="status-chip">In Progress: {taskCountByStatus(k.tasks ?? [], 'in_progress')}</div>
            <div class="status-chip">Blocked: {taskCountByStatus(k.tasks ?? [], 'blocked')}</div>
          </div>
          <div class="task-cards">
            {#each k.tasks ?? [] as task}
              <article class="task-card">
                <div class="explorer-detail-header">
                  <strong>{(task as { title?: string }).title}</strong>
                  <span class={`status-badge ${statusClass((task as { status?: string }).status)}`}>{(task as { status?: string }).status ?? 'unknown'}</span>
                </div>
                <p class="muted">{(task as { description?: string }).description || 'No description.'}</p>
                <button class="ghost" onclick={() => selectTask(task)}>Open Task</button>
              </article>
            {:else}
              <p class="muted">No sub-tasks for this kluster yet.</p>
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
        <p class="muted">Choose a mission or kluster to inspect.</p>
      {/if}
    </section>
  </div>
</div>
