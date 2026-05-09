import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

export function statusClass(status: string | undefined | null): string {
	const v = String(status ?? '').toLowerCase();
	if (v === 'done' || v === 'completed') return 'status-done';
	if (v === 'blocked' || v === 'failed' || v === 'error') return 'status-blocked';
	if (v === 'in_progress' || v === 'running') return 'status-progress';
	return 'status-proposed';
}

export function fmtRelative(isoOrMs: string | number): string {
	const ts = typeof isoOrMs === 'number' ? isoOrMs : new Date(isoOrMs).getTime();
	const diff = Date.now() - ts;
	const m = Math.floor(diff / 60_000);
	if (m < 1) return 'just now';
	if (m < 60) return `${m}m ago`;
	const h = Math.floor(m / 60);
	if (h < 24) return `${h}h ago`;
	return `${Math.floor(h / 24)}d ago`;
}

export function taskCountByStatus(tasks: Array<{ status?: string }>, status: string): number {
	return tasks.filter(t => String(t.status ?? '').toLowerCase() === status.toLowerCase()).length;
}
