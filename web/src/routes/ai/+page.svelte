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
</script>

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
