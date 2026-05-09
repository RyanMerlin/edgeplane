import { request } from './client';

export type OidcExchangeResponse = {
	token: string;
	subject: string;
	expires_at: string;
	session_id: number;
	ttl_hours: number;
};

export function exchangeOidcGrant(grantId: string) {
	return request<OidcExchangeResponse>('/auth/oidc/exchange', {
		method: 'POST',
		body: JSON.stringify({ grant_id: grantId })
	});
}
