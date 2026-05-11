/**
 * Tests for the ACP attach WS client.
 *
 * The frame routing logic — hello vs SessionNotification vs unparseable —
 * is what we can exercise without spinning up an actual mc-mesh node.
 * Connection lifecycle (reconnect timers, real WebSocket state) is
 * covered end-to-end via Phase A validation in the tmux-retirement plan.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { AcpAttach, type SessionNotification, type HelloFrame } from './acp-attach';

// Minimal Browser WebSocket stub — captures sends and lets us trigger
// onmessage/onopen/onerror/onclose. Each `new WebSocket(url)` returns the
// LATEST instance via `lastInstance` so tests can drive it.
class MockWebSocket {
	// Mirror the numeric readyState constants the production code reads
	// off the WebSocket class. Without these, `WebSocket.OPEN` resolves
	// to `undefined` once we swap globalThis.WebSocket for the mock and
	// the `readyState !== WebSocket.OPEN` guard always trips.
	static readonly CONNECTING = 0;
	static readonly OPEN = 1;
	static readonly CLOSING = 2;
	static readonly CLOSED = 3;

	static lastInstance: MockWebSocket | undefined;
	url: string;
	readyState: number = 0; // CONNECTING
	binaryType: string = 'blob';
	onopen: ((this: WebSocket, ev: Event) => unknown) | null = null;
	onmessage: ((this: WebSocket, ev: MessageEvent) => unknown) | null = null;
	onerror: ((this: WebSocket, ev: Event) => unknown) | null = null;
	onclose: ((this: WebSocket, ev: CloseEvent) => unknown) | null = null;
	sent: string[] = [];

	constructor(url: string) {
		this.url = url;
		MockWebSocket.lastInstance = this;
	}

	send(data: string): void {
		this.sent.push(data);
	}

	close(): void {
		this.readyState = 3; // CLOSED
	}

	// Test helpers.
	openIt(): void {
		this.readyState = 1; // OPEN
		this.onopen?.call(this as unknown as WebSocket, new Event('open'));
	}
	push(text: string): void {
		this.onmessage?.call(
			this as unknown as WebSocket,
			new MessageEvent('message', { data: text })
		);
	}
}

// JSDOM has window but not WebSocket; vitest's `happy-dom` env may or
// may not. Patch globalThis directly so the source's `new WebSocket(...)`
// hits our mock.
beforeEach(() => {
	(globalThis as unknown as { WebSocket: typeof WebSocket }).WebSocket =
		MockWebSocket as unknown as typeof WebSocket;
	(globalThis as unknown as { window: Window }).window = {
		location: { protocol: 'http:', host: 'localhost:8008' }
	} as unknown as Window;
	MockWebSocket.lastInstance = undefined;
});

afterEach(() => {
	vi.restoreAllMocks();
});

describe('AcpAttach', () => {
	it('routes hello frames to onHello', () => {
		const onHello = vi.fn();
		const onNotif = vi.fn();
		const attach = new AcpAttach('node1', 'agent1', 'tok', {
			onHello,
			onNotification: onNotif
		});
		attach.open();
		const ws = MockWebSocket.lastInstance!;
		ws.openIt();
		ws.push(JSON.stringify({ kind: 'hello', protocol: 'acp/1' }));
		expect(onHello).toHaveBeenCalledWith({ kind: 'hello', protocol: 'acp/1' });
		expect(onNotif).not.toHaveBeenCalled();
		attach.dispose();
	});

	it('routes SessionNotification frames to onNotification', () => {
		const onNotif = vi.fn();
		const attach = new AcpAttach('n', 'a', 't', { onNotification: onNotif });
		attach.open();
		MockWebSocket.lastInstance!.openIt();
		const frame: SessionNotification = {
			sessionId: 'sess-1',
			update: {
				sessionUpdate: 'agent_message_chunk',
				content: { type: 'text', text: 'hello' }
			}
		};
		MockWebSocket.lastInstance!.push(JSON.stringify(frame));
		expect(onNotif).toHaveBeenCalledTimes(1);
		expect(onNotif.mock.calls[0][0]).toMatchObject({ sessionId: 'sess-1' });
		attach.dispose();
	});

	it('routes unparseable frames to onUnparseable', () => {
		const onUnparseable = vi.fn();
		const attach = new AcpAttach('n', 'a', 't', { onUnparseable });
		attach.open();
		MockWebSocket.lastInstance!.openIt();
		MockWebSocket.lastInstance!.push('not-json{');
		expect(onUnparseable).toHaveBeenCalledTimes(1);
		expect(onUnparseable.mock.calls[0][0]).toBe('not-json{');
		attach.dispose();
	});

	it('sendPrompt drops when socket is not open', () => {
		const attach = new AcpAttach('n', 'a', 't', {});
		expect(attach.sendPrompt('x')).toBe(false);
		attach.dispose();
	});

	it('sendPrompt serialises the envelope correctly when open', () => {
		const attach = new AcpAttach('n', 'a', 't', {});
		attach.open();
		MockWebSocket.lastInstance!.openIt();
		expect(attach.sendPrompt('do the thing')).toBe(true);
		expect(MockWebSocket.lastInstance!.sent).toEqual([
			JSON.stringify({ kind: 'prompt', text: 'do the thing' })
		]);
		attach.dispose();
	});

	it('sendPrompt rejects whitespace-only input', () => {
		const attach = new AcpAttach('n', 'a', 't', {});
		attach.open();
		MockWebSocket.lastInstance!.openIt();
		expect(attach.sendPrompt('   ')).toBe(false);
		expect(MockWebSocket.lastInstance!.sent).toHaveLength(0);
		attach.dispose();
	});

	it('sendCancel writes the cancel envelope', () => {
		const attach = new AcpAttach('n', 'a', 't', {});
		attach.open();
		MockWebSocket.lastInstance!.openIt();
		expect(attach.sendCancel()).toBe(true);
		expect(MockWebSocket.lastInstance!.sent).toEqual([
			JSON.stringify({ kind: 'cancel' })
		]);
		attach.dispose();
	});

	it('onStatus reflects lifecycle transitions', () => {
		const transitions: string[] = [];
		const attach = new AcpAttach('n', 'a', 't', {
			onStatus: (s) => transitions.push(s)
		});
		attach.open();
		expect(transitions).toEqual(['connecting']);
		MockWebSocket.lastInstance!.openIt();
		expect(transitions).toEqual(['connecting', 'open']);
		attach.dispose();
	});

	it('frames with unknown shape route to onUnparseable rather than misclassify', () => {
		const onNotif = vi.fn();
		const onHello = vi.fn();
		const onUnparseable = vi.fn();
		const attach = new AcpAttach('n', 'a', 't', {
			onHello,
			onNotification: onNotif,
			onUnparseable
		});
		attach.open();
		MockWebSocket.lastInstance!.openIt();
		// Has neither `kind === 'hello'` nor `sessionId` + `update`.
		MockWebSocket.lastInstance!.push(JSON.stringify({ random: 'shape' }));
		expect(onHello).not.toHaveBeenCalled();
		expect(onNotif).not.toHaveBeenCalled();
		expect(onUnparseable).toHaveBeenCalledTimes(1);
		attach.dispose();
	});
});

// Exercising the type shape: confirm a HelloFrame literal compiles and
// the union matches the wire we expect.
describe('types', () => {
	it('HelloFrame matches the wire shape', () => {
		const h: HelloFrame = { kind: 'hello', protocol: 'acp/1' };
		expect(h.kind).toBe('hello');
	});
});
