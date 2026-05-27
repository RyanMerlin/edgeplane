function readCookie(name: string): string | null {
	if (typeof document === 'undefined') return null;
	const needle = `${name}=`;
	for (const part of document.cookie.split(';')) {
		const item = part.trim();
		if (item.startsWith(needle)) return decodeURIComponent(item.slice(needle.length));
	}
	return null;
}

export function authHeader(token?: string): Record<string, string> {
	const headers: Record<string, string> = {};
	if (token) headers.Authorization = `Bearer ${token}`;
	const csrf = readCookie('ep_csrf_token');
	if (csrf) headers['X-CSRF-Token'] = csrf;
	return headers;
}

export class ApiError extends Error {
	constructor(
		message: string,
		public status: number,
		public body: unknown
	) {
		super(message);
		this.name = 'ApiError';
	}
}

async function parseBody(res: Response): Promise<unknown> {
	const text = await res.text();
	if (!text) return null;
	try {
		return JSON.parse(text);
	} catch {
		return text;
	}
}

export async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
	const headers = new Headers(init.headers);

	if (!headers.has('Content-Type') && !(init.body instanceof FormData)) {
		headers.set('Content-Type', 'application/json');
	}

	const url = path.startsWith('/') ? `/api${path}` : path;
	const res = await fetch(url, { ...init, headers, credentials: 'include' });

	if (!res.ok) {
		const body = await parseBody(res);
		const message =
			typeof body === 'object' && body && 'error' in body
				? String((body as { error: unknown }).error)
				: `Request failed: ${res.status}`;
		throw new ApiError(message, res.status, body);
	}

	if (res.status === 204) return undefined as T;

	return parseBody(res) as Promise<T>;
}

export const api = {
	get: <T>(path: string) => request<T>(path),
	post: <T>(path: string, body?: unknown) =>
		request<T>(path, {
			method: 'POST',
			body: body === undefined ? undefined : JSON.stringify(body)
		}),
	put: <T>(path: string, body?: unknown) =>
		request<T>(path, {
			method: 'PUT',
			body: body === undefined ? undefined : JSON.stringify(body)
		}),
	patch: <T>(path: string, body?: unknown) =>
		request<T>(path, {
			method: 'PATCH',
			body: body === undefined ? undefined : JSON.stringify(body)
		}),
	delete: <T>(path: string) => request<T>(path, { method: 'DELETE' })
};
