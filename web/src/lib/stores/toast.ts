import { writable } from 'svelte/store';

export const toastStore = writable<{ message: string; visible: boolean }>({
	message: '',
	visible: false
});

let _timer: ReturnType<typeof setTimeout> | null = null;

export function showToast(message: string) {
	if (_timer) clearTimeout(_timer);
	toastStore.set({ message, visible: true });
	_timer = setTimeout(() => toastStore.set({ message: '', visible: false }), 4000);
}
