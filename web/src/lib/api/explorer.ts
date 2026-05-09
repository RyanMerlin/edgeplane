import { request, authHeader } from './client';

export type ExplorerTree = {
	missions?: unknown[];
	klusters?: unknown[];
	tasks?: unknown[];
};

export function fetchTree(token?: string) {
	return request<ExplorerTree>('/explorer/tree', { headers: authHeader(token) });
}

export function fetchNode(type: string, id: string, token?: string) {
	return request<unknown>(`/explorer/node/${type}/${id}`, { headers: authHeader(token) });
}
