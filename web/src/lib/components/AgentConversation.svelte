<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import {
		AcpAttach,
		type ConnectionStatus,
		type SessionNotification,
		type SessionUpdate
	} from '$lib/api/acp-attach';

	type Props = {
		nodeId: string;
		agentId: string;
		token: string;
	};
	const { nodeId, agentId, token }: Props = $props();

	// ── State ──────────────────────────────────────────────────────────────

	let status = $state<ConnectionStatus>('connecting');
	let helloProtocol = $state<string | null>(null);

	/** Flat event log shown in the pane. Each entry is an already-coalesced
	 * view of the SessionUpdate stream — assistant text streams append to the
	 * latest assistant entry instead of producing one entry per chunk. */
	type EventEntry =
		| { kind: 'assistant'; text: string; id: number }
		| { kind: 'thinking'; text: string; id: number }
		| { kind: 'tool'; title: string; toolCallId: string; status: string; id: number }
		| { kind: 'plan'; raw: unknown; id: number }
		| { kind: 'unknown'; raw: unknown; id: number };

	let events = $state<EventEntry[]>([]);
	let nextId = 0;
	let prompt = $state('');
	let attach: AcpAttach | undefined;
	let logEl: HTMLDivElement | undefined = $state();

	// ── Attach lifecycle ───────────────────────────────────────────────────

	onMount(() => {
		attach = new AcpAttach(nodeId, agentId, token, {
			onStatus: (s) => {
				status = s;
			},
			onHello: (h) => {
				helloProtocol = h.protocol;
			},
			onNotification: ingest
		});
		attach.open();
	});

	onDestroy(() => {
		attach?.dispose();
		attach = undefined;
	});

	// ── Frame ingestion ────────────────────────────────────────────────────

	function ingest(notif: SessionNotification): void {
		const u = notif.update as SessionUpdate;
		switch (u.sessionUpdate) {
			case 'agent_message_chunk': {
				const text = textOf(u.content);
				if (!text) break;
				const last = events.at(-1);
				if (last && last.kind === 'assistant') {
					// Append to running assistant turn. Svelte 5 reactivity
					// works on assignment to the array index.
					events[events.length - 1] = { ...last, text: last.text + text };
				} else {
					events = [...events, { kind: 'assistant', text, id: nextId++ }];
				}
				break;
			}
			case 'agent_thought_chunk': {
				const text = textOf(u.content);
				if (!text) break;
				const last = events.at(-1);
				if (last && last.kind === 'thinking') {
					events[events.length - 1] = { ...last, text: last.text + text };
				} else {
					events = [...events, { kind: 'thinking', text, id: nextId++ }];
				}
				break;
			}
			case 'tool_call': {
				events = [
					...events,
					{
						kind: 'tool',
						title: (u as Record<string, unknown>).title as string ?? 'tool',
						toolCallId: ((u as Record<string, unknown>).toolCallId as string) ?? '',
						status: 'started',
						id: nextId++
					}
				];
				break;
			}
			case 'tool_call_update': {
				const id = (u as Record<string, unknown>).toolCallId as string | undefined;
				const newStatus = (u as Record<string, unknown>).status as string | undefined;
				if (!newStatus) break;
				// Update the matching tool entry in place; if absent, append.
				let found = false;
				for (let i = events.length - 1; i >= 0; i--) {
					const e = events[i];
					if (e.kind === 'tool' && e.toolCallId === id) {
						events[i] = { ...e, status: newStatus };
						found = true;
						break;
					}
				}
				if (!found) {
					events = [
						...events,
						{
							kind: 'tool',
							title: (u as Record<string, unknown>).title as string ?? 'tool',
							toolCallId: id ?? '',
							status: newStatus,
							id: nextId++
						}
					];
				}
				break;
			}
			case 'plan': {
				events = [...events, { kind: 'plan', raw: u, id: nextId++ }];
				break;
			}
			default:
				// Metadata updates (available_commands, current_mode, usage,
				// session_info, etc.) are noisy; drop. Unknown variants get
				// the unknown bucket so the operator can still see them in
				// dev mode.
				if (!METADATA_KINDS.has(u.sessionUpdate)) {
					events = [...events, { kind: 'unknown', raw: u, id: nextId++ }];
				}
		}
		queueScroll();
	}

	const METADATA_KINDS = new Set([
		'user_message_chunk',
		'available_commands_update',
		'current_mode_update',
		'config_option_update',
		'session_info_update',
		'usage_update'
	]);

	function textOf(content: unknown): string {
		if (!content || typeof content !== 'object') return '';
		const c = content as Record<string, unknown>;
		if (typeof c.text === 'string') return c.text;
		return '';
	}

	// ── Auto-scroll ────────────────────────────────────────────────────────

	let scrollPending = false;
	function queueScroll(): void {
		if (scrollPending) return;
		scrollPending = true;
		requestAnimationFrame(() => {
			scrollPending = false;
			if (logEl) {
				logEl.scrollTop = logEl.scrollHeight;
			}
		});
	}

	// ── Outbound ───────────────────────────────────────────────────────────

	function send(): void {
		if (!prompt.trim()) return;
		if (attach?.sendPrompt(prompt)) {
			// Echo the prompt locally as an assistant-distinct "you" entry?
			// For now don't — keeps the pane a faithful mirror of what the
			// agent is doing. The user already sees their own text in the
			// input. Future enhancement: a `user` event entry kind.
			prompt = '';
		}
	}

	function cancel(): void {
		attach?.sendCancel();
	}

	function handleKey(e: KeyboardEvent): void {
		// Cmd/Ctrl+Enter sends; plain Enter inserts a newline so multi-line
		// prompts work without surprise-sends.
		if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
			e.preventDefault();
			send();
		}
	}
</script>

<section class="conversation">
	<header class="conv-header">
		<div class="conv-id">
			<strong>{agentId}</strong>
			<span class="node">@ {nodeId}</span>
		</div>
		<div class="conv-status">
			<span class="state state-{status}">{status}</span>
			{#if helloProtocol}<span class="proto">acp/{helloProtocol.replace(/^acp\//, '')}</span>{/if}
		</div>
	</header>

	<div class="conv-log" bind:this={logEl}>
		{#if events.length === 0}
			<div class="empty">Waiting for the agent…</div>
		{/if}
		{#each events as ev (ev.id)}
			{#if ev.kind === 'assistant'}
				<div class="entry assistant">
					<div class="entry-text">{ev.text}</div>
				</div>
			{:else if ev.kind === 'thinking'}
				<details class="entry thinking">
					<summary>thinking</summary>
					<pre>{ev.text}</pre>
				</details>
			{:else if ev.kind === 'tool'}
				<div class="entry tool">
					<span class="tool-title">{ev.title}</span>
					<span class="tool-status status-{ev.status}">{ev.status}</span>
				</div>
			{:else if ev.kind === 'plan'}
				<details class="entry plan">
					<summary>plan update</summary>
					<pre>{JSON.stringify(ev.raw, null, 2)}</pre>
				</details>
			{:else if ev.kind === 'unknown'}
				<details class="entry unknown">
					<summary>{(ev.raw as { sessionUpdate?: string })?.sessionUpdate ?? 'unknown'}</summary>
					<pre>{JSON.stringify(ev.raw, null, 2)}</pre>
				</details>
			{/if}
		{/each}
	</div>

	<form class="conv-input" onsubmit={(e) => { e.preventDefault(); send(); }}>
		<textarea
			bind:value={prompt}
			placeholder="Type a prompt — ⌘/Ctrl+Enter to send"
			rows="3"
			disabled={status !== 'open'}
			onkeydown={handleKey}
		></textarea>
		<div class="conv-actions">
			<button type="button" class="ghost" onclick={cancel} disabled={status !== 'open'}>Cancel turn</button>
			<button type="submit" class="primary" disabled={status !== 'open' || !prompt.trim()}>Send</button>
		</div>
	</form>
</section>

<style>
	.conversation {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		background: var(--panel, #0b0e14);
		border-radius: 6px;
		overflow: hidden;
		border: 1px solid var(--border, #1c2230);
	}
	.conv-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 0.5rem 0.75rem;
		background: var(--panel-darker, #11151c);
		border-bottom: 1px solid var(--border, #1c2230);
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
		font-size: 12px;
	}
	.conv-id strong {
		color: var(--accent, #ffae57);
	}
	.node {
		color: var(--muted, #6e7785);
		margin-left: 0.5rem;
	}
	.state {
		text-transform: uppercase;
		letter-spacing: 0.05em;
		font-size: 11px;
		font-weight: 600;
	}
	.state-connecting { color: #ddc05a; }
	.state-open { color: #4ade80; }
	.state-closed, .state-error { color: #f87171; }
	.proto {
		margin-left: 0.5rem;
		color: var(--muted, #6e7785);
	}
	.conv-log {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 0.75rem;
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		font-family: ui-sans-serif, system-ui, sans-serif;
		font-size: 14px;
		line-height: 1.5;
	}
	.empty {
		color: var(--muted, #6e7785);
		font-style: italic;
	}
	.entry { padding: 0.4rem 0.6rem; border-radius: 4px; }
	.assistant {
		background: var(--panel-lift, #11151c);
		white-space: pre-wrap;
	}
	.entry-text { color: var(--text, #c8ccd4); }
	.thinking, .plan, .unknown {
		background: rgba(110, 119, 133, 0.08);
		font-size: 12px;
		color: var(--muted, #6e7785);
	}
	.thinking summary, .plan summary, .unknown summary {
		cursor: pointer;
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
	}
	.thinking pre, .plan pre, .unknown pre {
		margin: 0.4rem 0 0;
		padding: 0.4rem;
		background: rgba(0, 0, 0, 0.2);
		border-radius: 3px;
		white-space: pre-wrap;
		word-break: break-word;
		font-size: 11px;
	}
	.tool {
		display: flex;
		gap: 0.6rem;
		align-items: center;
		background: rgba(88, 166, 255, 0.06);
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
		font-size: 12px;
	}
	.tool-title {
		color: #58a6ff;
		font-weight: 600;
	}
	.tool-status {
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.status-started { color: #ddc05a; }
	.status-in_progress { color: #58a6ff; }
	.status-completed { color: #4ade80; }
	.status-failed { color: #f87171; }
	.status-cancelled { color: #6e7785; }
	.conv-input {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
		padding: 0.5rem 0.75rem 0.75rem;
		background: var(--panel-darker, #11151c);
		border-top: 1px solid var(--border, #1c2230);
	}
	.conv-input textarea {
		resize: vertical;
		font-family: inherit;
		font-size: 14px;
		padding: 0.5rem;
		background: var(--input-bg, #0b0e14);
		color: var(--text, #c8ccd4);
		border: 1px solid var(--border, #1c2230);
		border-radius: 4px;
	}
	.conv-input textarea:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.conv-actions {
		display: flex;
		gap: 0.5rem;
		justify-content: flex-end;
	}
</style>
