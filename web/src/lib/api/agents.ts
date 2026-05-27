/**
 * Helpers for the persistent-agent attach WebSocket.
 *
 * The browser dials the controlplane (same origin), which proxies the
 * connection to the edgeplaned node over Tailscale. Auth is via the
 * session cookie set during OIDC login.
 */
export function attachAgentWsUrl(
	nodeId: string,
	agentId: string,
	_token?: string | null
): string {
	if (typeof window === 'undefined') {
		throw new Error('attachAgentWsUrl can only run in the browser');
	}
	const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
	const base = `${proto}//${window.location.host}`;
	const path = `/api/runtime/nodes/${encodeURIComponent(nodeId)}/agents/${encodeURIComponent(agentId)}/attach`;
	return `${base}${path}`;
}
