<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { get, derived } from 'svelte/store';
  import {
    authStore, bootstrapAuth, loginWithCookieSession, loginWithToken, token, startOidcLogin
  } from '$lib/auth';
  import {
    fetchTree, fetchPolicy, fetchGovernanceEvents, fetchNode,
    createAiSession, listAiSessions, getAiSession,
    sendAiTurn, approveAiAction, rejectAiAction, exchangeOidcGrant,
    type AiSession, type AiEvent
  } from '$lib/api';
  import { queryKeys } from '$lib/queryKeys';
  import { matrixEvents, matrixStatus, startMatrixStream, stopMatrixStream } from '$lib/telemetry';
  import { createQuery, createMutation, useQueryClient } from '@tanstack/svelte-query';

  // ── Types ────────────────────────────────────────────────────────────────────

  type UiEntry = {
    key: string;
    kind: 'user' | 'assistant' | 'event';
    title: string;
    body: string;
    eventType?: string;
    payload?: Record<string, unknown>;
    createdAt: string;
  };
  type TabName = 'ai' | 'matrix' | 'explorer' | 'onboarding' | 'governance';
  const TAB_STORAGE_KEY = 'mc.ui.selected_tab';
  const TAB_NAMES: TabName[] = ['ai', 'matrix', 'explorer', 'onboarding', 'governance'];

  // ── Query client ─────────────────────────────────────────────────────────────

  const queryClient = useQueryClient();

  // ── Auth state (bridged from stores → $state for TanStack Query reactivity) ─

  let isLoggedIn = $state(get(authStore).loggedIn);
  let currentToken = $state<string | null>(get(authStore).token ?? null);

  $effect(() => {
    return authStore.subscribe($auth => {
      isLoggedIn = $auth.loggedIn;
      currentToken = $auth.token ?? null;
    });
  });

  // ── UI state ─────────────────────────────────────────────────────────────────

  let initialToken = $state('');
  let selectedTab = $state<TabName>('ai');
  let searchInput = $state('');
  let statusMessage = $state('');
  let toastVisible = $state(false);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // AI console
  let activeSessionId = $state<string | null>(null);
  let sessionAutoCreated = $state(false);
  let aiInput = $state('');
  let aiError = $state('');
  let pinToBottom = $state(true);
  let showEventDebug = $state(false);
  let terminalEl = $state<HTMLDivElement | null>(null);

  // Explorer
  let selectedNodeType = $state<'mission' | 'kluster' | 'task' | null>(null);
  let selectedNodeKey = $state<{ type: 'mission' | 'kluster' | 'task'; id: string } | null>(null);
  let selectedMissionData = $state<{ mission: unknown; klusters: unknown[]; tasks: unknown[] } | null>(null);

  // Onboarding
  let onboardingEndpoint = $state('');
  let onboardingManifest = $state('');
  let manifestUrl = $state('');

  // ── Queries ──────────────────────────────────────────────────────────────────

  const sessionsQuery = createQuery(() => ({
    queryKey: queryKeys.ai.sessions(),
    queryFn: () => listAiSessions(currentToken || undefined),
    enabled: isLoggedIn
  }));

  const activeSessionQuery = createQuery(() => ({
    queryKey: activeSessionId
      ? queryKeys.ai.session(activeSessionId)
      : (['__ai_none__'] as const),
    queryFn: () => getAiSession(activeSessionId!, currentToken || undefined, 0),
    enabled: isLoggedIn && !!activeSessionId,
    refetchInterval: isLoggedIn && !!activeSessionId ? 2500 : false
  }));

  const treeQuery = createQuery(() => ({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => fetchTree(currentToken || undefined),
    enabled: isLoggedIn
  }));

  const policyQuery = createQuery(() => ({
    queryKey: queryKeys.governance.policy(),
    queryFn: () => fetchPolicy(currentToken || undefined),
    enabled: isLoggedIn
  }));

  const policyEventsQuery = createQuery(() => ({
    queryKey: queryKeys.governance.events(),
    queryFn: () => fetchGovernanceEvents(currentToken || undefined),
    enabled: isLoggedIn
  }));

  const nodeQuery = createQuery(() => ({
    queryKey: selectedNodeKey
      ? queryKeys.explorer.node(selectedNodeKey.type, selectedNodeKey.id)
      : (['__explorer_none__'] as const),
    queryFn: () => fetchNode(selectedNodeKey!.type, selectedNodeKey!.id, currentToken || undefined),
    enabled: isLoggedIn && !!selectedNodeKey
  }));

  // ── Mutations ─────────────────────────────────────────────────────────────────

  const createSessionMutation = createMutation(() => ({
    mutationFn: () => createAiSession(currentToken || undefined, 'AI Console Session'),
    onSuccess: (session: AiSession) => {
      activeSessionId = session.id;
      queryClient.invalidateQueries({ queryKey: queryKeys.ai.sessions() });
    },
    onError: (err: unknown) => {
      aiError = err instanceof Error ? err.message : 'Failed to create AI session';
    }
  }));

  const sendMutation = createMutation(() => ({
    mutationFn: ({ message }: { message: string }) =>
      sendAiTurn(activeSessionId!, message, currentToken || undefined),
    onSuccess: (session: AiSession) => {
      queryClient.setQueryData(queryKeys.ai.session(activeSessionId!), session);
      aiInput = '';
      pinToBottom = true;
    },
    onError: (err: unknown) => {
      aiError = err instanceof Error ? err.message : 'Send failed';
    }
  }));

  const approveMutation = createMutation(() => ({
    mutationFn: (actionId: string) =>
      approveAiAction(activeSessionId!, actionId, currentToken || undefined),
    onSuccess: (session: AiSession) => {
      queryClient.setQueryData(queryKeys.ai.session(activeSessionId!), session);
      pinToBottom = true;
    },
    onError: (err: unknown) => {
      aiError = err instanceof Error ? err.message : 'Approval failed';
    }
  }));

  const rejectMutation = createMutation(() => ({
    mutationFn: (actionId: string) =>
      rejectAiAction(activeSessionId!, actionId, currentToken || undefined),
    onSuccess: (session: AiSession) => {
      queryClient.setQueryData(queryKeys.ai.session(activeSessionId!), session);
    },
    onError: (err: unknown) => {
      aiError = err instanceof Error ? err.message : 'Reject failed';
    }
  }));

  // ── Derived state ─────────────────────────────────────────────────────────────

  let aiBusy = $derived(
    sendMutation.isPending ||
    approveMutation.isPending ||
    rejectMutation.isPending ||
    createSessionMutation.isPending
  );

  let transcript = $derived(buildTranscript(activeSessionQuery.data ?? null));

  let pendingActions = $derived(
    (activeSessionQuery.data?.pending_actions ?? []).filter(a => a.status === 'pending')
  );

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

  // Svelte store derivations (matrix stream data)
  const lastEvent = derived(matrixEvents, $evts => ($evts.length ? $evts[0] : null));
  const eventChunks = derived(matrixEvents, $evts =>
    $evts.map(event => ({
      label: event.type ?? 'matrix',
      status: event.status,
      detail: event.payload,
      time: new Date(event.receivedAt).toLocaleTimeString()
    }))
  );

  // ── Effects ───────────────────────────────────────────────────────────────────

  // SSE and cache management on auth change
  $effect(() => {
    if (!isLoggedIn) {
      stopMatrixStream();
      queryClient.clear();
      activeSessionId = null;
      sessionAutoCreated = false;
      return;
    }
    startMatrixStream(currentToken ?? undefined);
    return () => { stopMatrixStream(); };
  });

  // Auto-select first session or auto-create one if none exist
  $effect(() => {
    const sessions = sessionsQuery.data;
    if (sessions === undefined) return;
    if (sessions.length > 0) {
      if (!activeSessionId) activeSessionId = sessions[0].id;
    } else if (!sessionAutoCreated && !createSessionMutation.isPending) {
      sessionAutoCreated = true;
      createSessionMutation.mutate();
    }
  });

  // Auto-scroll when session data updates
  $effect(() => {
    if (activeSessionQuery.data) {
      maybeScrollToBottom();
    }
  });

  // ── Functions ─────────────────────────────────────────────────────────────────

  function handleToken() {
    if (!initialToken.trim()) { showToast('Enter a MissionControl token or use OIDC login.'); return; }
    loginWithToken(initialToken.trim());
  }

  function handleOidc() { startOidcLogin(window.location.pathname); }

  function isTabName(value: string | null): value is TabName {
    return value !== null && TAB_NAMES.includes(value as TabName);
  }

  function setSelectedTab(tab: TabName) {
    selectedTab = tab;
    if (typeof window !== 'undefined') window.localStorage.setItem(TAB_STORAGE_KEY, tab);
  }

  function showToast(msg: string) {
    statusMessage = msg;
    toastVisible = true;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { toastVisible = false; }, 4000);
  }

  function summarizeEvent(event: AiEvent): { title: string; body: string } {
    const payload = event.payload ?? {};
    if (event.event_type === 'tool_call')
      return { title: 'Tool call', body: `${payload.tool ?? 'unknown'} ${JSON.stringify(payload.args ?? {})}` };
    if (event.event_type === 'tool_result') {
      const ok = Boolean((payload.result as { ok?: boolean })?.ok);
      if (ok) return { title: 'Tool result', body: `${payload.tool ?? 'tool'} completed` };
      return { title: 'Tool issue', body: `I could not complete ${payload.tool ?? 'that tool'} this time. Expand details for the technical error.` };
    }
    if (event.event_type === 'approval_required')
      return { title: 'Approval required', body: `${payload.tool ?? 'action'} is waiting for approval` };
    if (event.event_type === 'approval_outcome')
      return { title: 'Approval outcome', body: `${payload.action_id ?? 'action'} ${payload.status ?? ''}` };
    if (event.event_type === 'view_rendered')
      return { title: 'View prepared', body: `${(payload.view as { title?: string })?.title ?? 'Custom view'} ready` };
    if (event.event_type === 'planner_result')
      return { title: 'Planner result', body: String(payload.assistant_text ?? 'Plan generated') };
    if (event.event_type === 'session_started')
      return { title: 'Session started', body: String(payload.title ?? 'AI session') };
    return { title: event.event_type, body: 'Event captured' };
  }

  function buildTranscript(session: AiSession | null): UiEntry[] {
    if (!session) return [];
    const entries: UiEntry[] = [];
    for (const t of session.turns ?? []) {
      const text = String((t.content ?? {}).text ?? '').trim() || JSON.stringify(t.content ?? {});
      entries.push({
        key: `turn-${t.id}`,
        kind: t.role === 'assistant' ? 'assistant' : 'user',
        title: t.role === 'assistant' ? 'MissionControl' : 'You',
        body: text,
        payload: t.content,
        createdAt: t.created_at
      });
    }
    for (const e of session.events ?? []) {
      if (e.event_type === 'user_message') continue;
      if (!showEventDebug && (e.event_type === 'planner_result' || e.event_type === 'session_started')) continue;
      const s = summarizeEvent(e);
      entries.push({
        key: `event-${e.id}`,
        kind: 'event',
        title: s.title,
        body: s.body,
        eventType: e.event_type,
        payload: e.payload,
        createdAt: e.created_at
      });
    }
    return entries.sort((a, b) => new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime());
  }

  async function maybeScrollToBottom(force = false) {
    await tick();
    if (!terminalEl) return;
    if (force || pinToBottom) terminalEl.scrollTop = terminalEl.scrollHeight;
  }

  function onTranscriptScroll() {
    if (!terminalEl) return;
    const delta = terminalEl.scrollHeight - terminalEl.scrollTop - terminalEl.clientHeight;
    pinToBottom = delta < 48;
  }

  function sendAiMessage() {
    const message = aiInput.trim();
    if (!message || !activeSessionId || aiBusy) return;
    sendMutation.mutate({ message });
  }

  function newAiSession() {
    if (aiBusy) return;
    sessionAutoCreated = false;
    createSessionMutation.mutate();
  }

  function approve(actionId: string) {
    if (!activeSessionId || aiBusy) return;
    approveMutation.mutate(actionId);
  }

  function reject(actionId: string) {
    if (!activeSessionId || aiBusy) return;
    rejectMutation.mutate(actionId);
  }

  function refreshTree() {
    queryClient.invalidateQueries({ queryKey: queryKeys.explorer.tree() });
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

  function defaultOnboardingEndpoint() {
    if (typeof window === 'undefined') return 'https://mc.missioncontrolai.app';
    return window.location.origin;
  }

  async function syncOnboardingEndpoint() {
    const localUrl = `${defaultOnboardingEndpoint().replace(/\/$/, '')}/agent-onboarding.json`;
    try {
      const res = await fetch(localUrl);
      if (!res.ok) return;
      const manifest = await res.json();
      onboardingEndpoint = String(manifest?.generated_for_base_url || manifest?.endpoints?.ui || '').replace(/\/ui\/$/, '');
    } catch { /* ignore */ }
  }

  async function loadManifest() {
    const normalized = (onboardingEndpoint || defaultOnboardingEndpoint()).replace(/\/$/, '');
    manifestUrl = `${normalized}/agent-onboarding.json`;
    try {
      const res = await fetch(manifestUrl);
      if (!res.ok) throw new Error(`Manifest fetch failed (${res.status})`);
      const manifest = await res.json();
      onboardingManifest = JSON.stringify(manifest, null, 2);
    } catch (err) {
      onboardingManifest = '';
      showToast(err instanceof Error ? err.message : 'Failed to load onboarding manifest');
    }
  }

  onMount(() => {
    const savedTab = window.localStorage.getItem(TAB_STORAGE_KEY);
    if (isTabName(savedTab)) selectedTab = savedTab;

    onboardingEndpoint = defaultOnboardingEndpoint();
    syncOnboardingEndpoint().finally(() => loadManifest());

    const params = new URLSearchParams(window.location.search);
    const hashParams = new URLSearchParams(window.location.hash.replace(/^#/, ''));
    const grant = hashParams.get('oidc_grant') || params.get('oidc_grant');
    if (grant) {
      exchangeOidcGrant(grant)
        .then(() => {
          loginWithCookieSession();
          hashParams.delete('oidc_grant');
          params.delete('oidc_grant');
          const query = params.toString();
          const hash = hashParams.toString();
          window.history.replaceState(
            {}, '',
            `${window.location.pathname}${query ? `?${query}` : ''}${hash ? `#${hash}` : ''}`
          );
        })
        .catch(err => { showToast(err instanceof Error ? err.message : 'OIDC login failed'); });
    }

    bootstrapAuth();
  });

  onDestroy(() => {
    if (toastTimer) clearTimeout(toastTimer);
    stopMatrixStream();
  });
</script>

{#if isLoggedIn}
  <div class="main-shell">
    <section class="tabs">
      <button class={`tab ${selectedTab === 'ai' ? 'active' : ''}`} onclick={() => setSelectedTab('ai')}>AI Console</button>
      <button class={`tab ${selectedTab === 'matrix' ? 'active' : ''}`} onclick={() => setSelectedTab('matrix')}>Matrix</button>
      <button class={`tab ${selectedTab === 'explorer' ? 'active' : ''}`} onclick={() => setSelectedTab('explorer')}>Explorer</button>
      <button class={`tab ${selectedTab === 'onboarding' ? 'active' : ''}`} onclick={() => setSelectedTab('onboarding')}>Onboarding</button>
      <button class={`tab ${selectedTab === 'governance' ? 'active' : ''}`} onclick={() => setSelectedTab('governance')}>Governance</button>
    </section>

    {#if selectedTab === 'ai'}
      <div class="glass-panel ai-shell">
        <div class="ai-header">
          <div>
            <h3>MissionControl AI Console</h3>
            <p class="muted">AI-first workspace. Reads auto-run, writes require approval.</p>
          </div>
          <div class="onboarding-actions">
            <button class="ghost" onclick={() => (showEventDebug = !showEventDebug)}>
              {showEventDebug ? 'Hide Debug Events' : 'Show Debug Events'}
            </button>
            <button class="ghost" onclick={newAiSession}>New Session</button>
          </div>
        </div>

        <div class="terminal-window" bind:this={terminalEl} onscroll={onTranscriptScroll}>
          {#if transcript.length}
            {#each transcript as entry (entry.key)}
              <div class={`event-pill ${entry.kind === 'assistant' ? 'assistant-msg' : entry.kind === 'user' ? 'user-msg' : 'event-msg'}`}>
                <small>{entry.title} • {new Date(entry.createdAt).toLocaleTimeString()}</small>
                <p>{entry.body}</p>
                {#if entry.kind === 'event' && entry.payload && (showEventDebug || entry.eventType === 'tool_result')}
                  <details>
                    <summary>Details</summary>
                    <pre>{JSON.stringify(entry.payload, null, 2)}</pre>
                  </details>
                {/if}
              </div>
            {/each}
          {:else}
            <p class="muted">No events yet. Ask AI to list missions, inspect tasks, or explain capabilities.</p>
          {/if}
        </div>

        {#if !pinToBottom}
          <div class="jump-row">
            <button class="ghost" onclick={() => { pinToBottom = true; maybeScrollToBottom(true); }}>Jump to latest</button>
          </div>
        {/if}

        {#if pendingActions.length}
          <section class="grid" style="margin-top: 0.25rem;">
            {#each pendingActions as action}
              <article class="glass-panel">
                <strong>Approval Required</strong>
                <p class="muted">Tool: {action.tool}</p>
                <p class="muted">Reason: {action.reason || 'No reason provided'}</p>
                <details>
                  <summary>Arguments</summary>
                  <pre>{JSON.stringify(action.args, null, 2)}</pre>
                </details>
                <div class="onboarding-actions">
                  <button class="primary" onclick={() => approve(action.id)} disabled={aiBusy}>Approve</button>
                  <button class="ghost" onclick={() => reject(action.id)} disabled={aiBusy}>Reject</button>
                </div>
              </article>
            {/each}
          </section>
        {/if}

        <div class="composer">
          <textarea
            bind:value={aiInput}
            rows="3"
            placeholder="Ask MissionControl AI..."
            onkeydown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendAiMessage(); }
            }}
          ></textarea>
          <button class="primary" onclick={sendAiMessage} disabled={aiBusy || !aiInput.trim()}>
            {aiBusy ? 'Running...' : 'Send'}
          </button>
        </div>
        {#if aiError}
          <p class="error">{aiError}</p>
        {/if}
      </div>
    {/if}

    {#if selectedTab === 'matrix'}
      <div class="glass-panel">
        <div class="grid">
          <div>
            <div class="status-chip">Matrix stream {$matrixStatus.connected ? 'live' : 'offline'}</div>
            <p class="muted">Rate limit: {$matrixStatus.rateLimit?.remaining ?? '—'} / {$matrixStatus.rateLimit?.limit ?? '—'}</p>
          </div>
          <div class="status-chip">Last event: {#if $lastEvent}{$lastEvent.time}{:else}waiting...{/if}</div>
        </div>
        <div class="matrix-timeline">
          {#each $eventChunks as chunk}
            <div class="event-pill">
              <small>{chunk.time}</small>
              <p>{chunk.label} - {chunk.status ?? 'pending'}</p>
              <p class="muted">{(chunk.detail as { summary?: string })?.summary ?? JSON.stringify(chunk.detail)}</p>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    {#if selectedTab === 'explorer'}
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
    {/if}

    {#if selectedTab === 'onboarding'}
      <div class="glass-panel">
        <h3>Agent Onboarding</h3>
        <label>Endpoint<input bind:value={onboardingEndpoint} placeholder="https://mc.example.com" /></label>
        <div class="onboarding-actions">
          <button class="ghost" onclick={loadManifest}>Regenerate Manifest</button>
          <button class="ghost" onclick={() => navigator.clipboard.writeText(onboardingManifest || '')}>Copy</button>
        </div>
        <div class="grid">
          <section class="glass-panel"><h4>Manifest URL</h4><code>{manifestUrl || 'fetch to generate'}</code></section>
          <section class="glass-panel"><h4>Manifest Preview</h4><pre>{onboardingManifest || 'No manifest yet.'}</pre></section>
        </div>
      </div>
    {/if}

    {#if selectedTab === 'governance'}
      <div class="glass-panel">
        <div class="grid">
          <section class="glass-panel"><h4>Active Policy</h4><pre>{policyQuery.data ? JSON.stringify(policyQuery.data, null, 2) : 'Loading...'}</pre></section>
          <section class="glass-panel">
            <h4>Policy Events</h4>
            <ul>
              {#each (policyEventsQuery.data ?? []) as evt}
                <li class="muted">[{(evt as { level?: string }).level}] {(evt as { message?: string }).message}</li>
              {:else}
                <li>No events yet.</li>
              {/each}
            </ul>
          </section>
        </div>
      </div>
    {/if}

    {#if toastVisible && statusMessage}
      <div class="toast" role="alert">{statusMessage}</div>
    {/if}
  </div>
{:else}
  <section class="login">
    <div class="login-card">
      <div class="status-chip">MissionControl Secure</div>
      <h1>Team Console</h1>
      <p class="muted" style="margin:0;">OIDC is the production login path. Token login is for testing.</p>
      <div class="login-actions">
        <button class="primary" onclick={handleOidc}>Sign in via OIDC</button>
      </div>
      <label>Testing Token<input bind:value={initialToken} type="password" placeholder="MC_TOKEN" /></label>
      <div class="login-actions">
        <button class="ghost" onclick={handleToken}>Continue with token</button>
      </div>
    </div>
  </section>
{/if}
