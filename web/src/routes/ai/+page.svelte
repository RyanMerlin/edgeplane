<script lang="ts">
  import { tick } from 'svelte';
  import { createQuery, createMutation, useQueryClient } from '@tanstack/svelte-query';
  import { useAuthState } from '$lib/stores/auth-state.svelte';
  import { showToast } from '$lib/stores/toast';
  import { queryKeys } from '$lib/queryKeys';
  import {
    createAiSession, listAiSessions, getAiSession,
    sendAiTurn, approveAiAction, rejectAiAction,
    type AiSession, type AiEvent
  } from '$lib/api';

  type UiEntry = {
    key: string;
    kind: 'user' | 'assistant' | 'event';
    title: string;
    body: string;
    eventType?: string;
    payload?: Record<string, unknown>;
    createdAt: string;
  };

  const queryClient = useQueryClient();
  const auth = useAuthState();

  let activeSessionId = $state<string | null>(null);
  let sessionAutoCreated = $state(false);
  let aiInput = $state('');
  let aiError = $state('');
  let pinToBottom = $state(true);
  let showEventDebug = $state(false);
  let terminalEl = $state<HTMLDivElement | null>(null);

  // ── Queries ──────────────────────────────────────────────────────────────────

  const sessionsQuery = createQuery(() => ({
    queryKey: queryKeys.ai.sessions(),
    queryFn: () => listAiSessions(auth.currentToken || undefined),
    enabled: auth.isLoggedIn
  }));

  const activeSessionQuery = createQuery(() => ({
    queryKey: activeSessionId
      ? queryKeys.ai.session(activeSessionId)
      : (['__ai_none__'] as const),
    queryFn: () => getAiSession(activeSessionId!, auth.currentToken || undefined, 0),
    enabled: auth.isLoggedIn && !!activeSessionId,
    refetchInterval: auth.isLoggedIn && !!activeSessionId ? 2500 : false
  }));

  // ── Mutations ─────────────────────────────────────────────────────────────────

  const createSessionMutation = createMutation(() => ({
    mutationFn: () => createAiSession(auth.currentToken || undefined, 'AI Console Session'),
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
      sendAiTurn(activeSessionId!, message, auth.currentToken || undefined),
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
      approveAiAction(activeSessionId!, actionId, auth.currentToken || undefined),
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
      rejectAiAction(activeSessionId!, actionId, auth.currentToken || undefined),
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

  // ── Effects ───────────────────────────────────────────────────────────────────

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

  $effect(() => {
    if (activeSessionQuery.data) maybeScrollToBottom();
  });

  // ── Functions ─────────────────────────────────────────────────────────────────

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
        title: t.role === 'assistant' ? 'Edgeplane' : 'You',
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
</script>

<div class="console-page">

  <!-- console topbar -->
  <div class="console-bar">
    <span class="console-title">Console</span>
    <span class="muted" style="font-size:11px;">Reads auto-run · writes require approval</span>
    <div style="margin-left:auto; display:flex; gap:6px;">
      <button class="ghost" onclick={() => (showEventDebug = !showEventDebug)}>
        {showEventDebug ? 'Hide debug' : 'Show debug'}
      </button>
      <button class="ghost" onclick={newAiSession}>New Session</button>
    </div>
  </div>

  <div class="pane-row" style="flex:1; min-height:0;">

    <!-- transcript pane -->
    <div class="pane" style="flex:1; min-width:0; display:flex; flex-direction:column;">
      <div class="pane-body" bind:this={terminalEl} onscroll={onTranscriptScroll} style="flex:1; min-height:0; overflow-y:auto;">
        {#if transcript.length}
          {#each transcript as entry (entry.key)}
            <div class="transcript-row transcript-{entry.kind}">
              <div class="transcript-meta">
                <span class="dim" style="font-size:10px;">{new Date(entry.createdAt).toLocaleTimeString()}</span>
                <span class="transcript-label">{entry.title}</span>
              </div>
              <div class="transcript-body">{entry.body}</div>
              {#if entry.kind === 'event' && entry.payload && (showEventDebug || entry.eventType === 'tool_result')}
                <details>
                  <summary>details</summary>
                  <pre style="font-size:11px;">{JSON.stringify(entry.payload, null, 2)}</pre>
                </details>
              {/if}
            </div>
          {/each}
        {:else}
          <div style="padding:12px;"><p class="muted">No events yet. Ask AI to list missions, inspect tasks, or explain capabilities.</p></div>
        {/if}
      </div>

      {#if !pinToBottom}
        <div style="padding:4px; text-align:center; border-top:1px solid var(--border);">
          <button class="ghost" onclick={() => { pinToBottom = true; maybeScrollToBottom(true); }}>Jump to latest</button>
        </div>
      {/if}

      <!-- composer -->
      <div class="console-composer">
        <textarea
          bind:value={aiInput}
          rows="2"
          placeholder="Ask EdgePlane AI… (Enter to send)"
          onkeydown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendAiMessage(); }
          }}
          style="flex:1;"
        ></textarea>
        <button class="primary" onclick={sendAiMessage} disabled={aiBusy || !aiInput.trim()}>
          {aiBusy ? '⟳' : 'Send'}
        </button>
      </div>
      {#if aiError}
        <div style="padding:4px 8px; border-top:1px solid var(--border);"><p class="error" style="margin:0;">{aiError}</p></div>
      {/if}
    </div>

    <!-- approvals pane -->
    {#if pendingActions.length}
      <div class="pane" style="width:280px; flex-shrink:0;">
        <div class="pane-header"><span class="pane-title">▲ Approvals</span><span class="warn">{pendingActions.length}</span></div>
        <div class="pane-body">
          {#each pendingActions as action}
            <div class="approval-row">
              <div style="font-size:11px; font-weight:600; color:var(--warn); margin-bottom:4px;">Approval Required</div>
              <div class="dim" style="font-size:11px;">Tool: {action.tool}</div>
              <div class="dim" style="font-size:11px;">{action.reason || 'No reason provided'}</div>
              <details style="margin-top:4px;">
                <summary>arguments</summary>
                <pre style="font-size:11px; margin-top:3px;">{JSON.stringify(action.args, null, 2)}</pre>
              </details>
              <div style="display:flex; gap:5px; margin-top:6px;">
                <button class="primary" style="flex:1;" onclick={() => approve(action.id)} disabled={aiBusy}>Approve</button>
                <button class="ghost" style="flex:1;" onclick={() => reject(action.id)} disabled={aiBusy}>Reject</button>
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}

  </div>

</div>

<style>
  .console-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .console-bar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .console-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }

  .transcript-row {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    animation: fade-in 180ms ease-out;
  }
  .transcript-row:hover { background: var(--surface-2); }

  .transcript-assistant { border-left: 3px solid var(--accent); }
  .transcript-user      { border-left: 3px solid var(--ok); }
  .transcript-event     { border-left: 3px solid var(--border-2); }

  .transcript-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 3px;
  }

  .transcript-label {
    font-size: 11px;
    color: var(--muted);
  }

  .transcript-body {
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .console-composer {
    display: flex;
    gap: 6px;
    padding: 6px 8px;
    background: var(--surface);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
    align-items: flex-end;
  }

  .approval-row {
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
</style>
