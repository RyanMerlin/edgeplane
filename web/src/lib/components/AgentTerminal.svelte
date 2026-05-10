<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { attachAgentWsUrl } from '$lib/api/agents';

	type Props = {
		nodeId: string;
		agentId: string;
		token: string;
	};
	const { nodeId, agentId, token }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state();
	let status = $state<'connecting' | 'open' | 'closed' | 'error'>('connecting');
	let lastError = $state<string | null>(null);

	// xterm and addons reference `self` at module load and crash under SSR.
	// Dynamic imports inside onMount keep them off the server build path.
	// Types stay narrow via `any` here; runtime usage is exercised by the build.
	let term: any | undefined;
	let fit: any | undefined;
	let ws: WebSocket | undefined;
	let resizeObserver: ResizeObserver | undefined;
	let reconnectTimeout: ReturnType<typeof setTimeout> | undefined;
	let reconnectDelay = 1000;
	let disposed = false;

	async function ensureTerm() {
		if (term || !containerEl) return;
		const [{ Terminal }, { FitAddon }, { WebLinksAddon }] = await Promise.all([
			import('@xterm/xterm'),
			import('@xterm/addon-fit'),
			import('@xterm/addon-web-links')
		]);
		// The xterm CSS comes from the package; pulled by the dynamic
		// import chain. If you see unstyled output, the import resolved
		// before vite injected the CSS — refresh once.
		await import('@xterm/xterm/css/xterm.css');

		if (disposed || !containerEl) return;

		term = new Terminal({
			cursorBlink: true,
			fontSize: 13,
			fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
			theme: { background: '#0b0e14' },
			allowProposedApi: true
		});
		fit = new FitAddon();
		term.loadAddon(fit);
		term.loadAddon(new WebLinksAddon());
		term.open(containerEl);
		fit.fit();

		term.onData((data: string) => {
			if (ws?.readyState === WebSocket.OPEN) {
				ws.send(new TextEncoder().encode(data));
			}
		});

		term.onResize(({ cols, rows }: { cols: number; rows: number }) => {
			if (ws?.readyState === WebSocket.OPEN) {
				ws.send(JSON.stringify({ kind: 'resize', cols, rows }));
			}
		});

		resizeObserver = new ResizeObserver(() => {
			try {
				fit?.fit();
			} catch {
				// Container being torn down — ignore.
			}
		});
		resizeObserver.observe(containerEl);
	}

	async function open() {
		status = 'connecting';
		lastError = null;
		await ensureTerm();
		if (disposed) return;

		try {
			ws = new WebSocket(attachAgentWsUrl(nodeId, agentId, token));
			ws.binaryType = 'arraybuffer';
		} catch (err) {
			status = 'error';
			lastError = err instanceof Error ? err.message : 'failed to open WebSocket';
			scheduleReconnect();
			return;
		}

		ws.onopen = () => {
			status = 'open';
			reconnectDelay = 1000;
			if (term && fit) {
				fit.fit();
				ws?.send(JSON.stringify({ kind: 'resize', cols: term.cols, rows: term.rows }));
			}
		};

		ws.onmessage = (ev) => {
			if (typeof ev.data === 'string') {
				term?.write(ev.data);
				return;
			}
			term?.write(new Uint8Array(ev.data as ArrayBuffer));
		};

		ws.onerror = () => {
			lastError = 'connection error';
			status = 'error';
		};

		ws.onclose = () => {
			status = 'closed';
			scheduleReconnect();
		};
	}

	function scheduleReconnect() {
		if (disposed) return;
		clearTimeout(reconnectTimeout);
		reconnectTimeout = setTimeout(() => {
			reconnectDelay = Math.min(reconnectDelay * 1.5, 30000);
			open();
		}, reconnectDelay);
	}

	onMount(() => {
		open();
	});

	onDestroy(() => {
		disposed = true;
		clearTimeout(reconnectTimeout);
		try {
			ws?.close();
		} catch {
			// ignore
		}
		resizeObserver?.disconnect();
		term?.dispose();
	});
</script>

<div class="terminal-wrap">
	<div class="terminal-status">
		<span class="agent">{agentId}</span>
		<span class="node">@ {nodeId}</span>
		<span class="state state-{status}">{status}</span>
		{#if lastError}<span class="err" title={lastError}>{lastError}</span>{/if}
	</div>
	<div class="terminal-host" bind:this={containerEl}></div>
</div>

<style>
	.terminal-wrap {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		background: #0b0e14;
		border-radius: 6px;
		overflow: hidden;
	}
	.terminal-status {
		display: flex;
		gap: 0.75rem;
		align-items: center;
		padding: 0.4rem 0.6rem;
		font-size: 12px;
		color: #c8ccd4;
		background: #11151c;
		border-bottom: 1px solid #1c2230;
		font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
	}
	.agent {
		color: #ffae57;
		font-weight: 600;
	}
	.node {
		color: #6e7785;
	}
	.state {
		margin-left: auto;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		font-size: 11px;
	}
	.state-connecting {
		color: #ddc05a;
	}
	.state-open {
		color: #4ade80;
	}
	.state-closed,
	.state-error {
		color: #f87171;
	}
	.err {
		color: #f87171;
	}
	.terminal-host {
		flex: 1;
		min-height: 0;
		padding: 0.4rem;
	}
</style>
