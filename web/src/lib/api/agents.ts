/**
 * Helpers for the persistent-agent attach WebSocket.
 *
 * The browser dials the controlplane (same origin), which proxies the
 * connection to the mc-mesh node over Tailscale. Auth is via the
 * `mc_token` query param — browsers can't set Authorization headers on
 * WebSocket upgrades, so we mirror the existing pattern from
 * `telemetry.ts`'s SSE stream.
 */
export function attachAgentWsUrl(
	nodeId: string,
	agentId: string,
	token: string
): string {
	if (typeof window === 'undefined') {
		throw new Error('attachAgentWsUrl can only run in the browser');
	}
	const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
	const base = `${proto}//${window.location.host}`;
	const path = `/runtime/nodes/${encodeURIComponent(nodeId)}/agents/${encodeURIComponent(agentId)}/attach`;
	return `${base}${path}?mc_token=${encodeURIComponent(token)}`;
}
