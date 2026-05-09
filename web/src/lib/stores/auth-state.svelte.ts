import { get } from 'svelte/store';
import { authStore } from '$lib/auth';

export function useAuthState() {
	let isLoggedIn = $state(get(authStore).loggedIn);
	let currentToken = $state<string | null>(get(authStore).token ?? null);

	$effect(() => {
		return authStore.subscribe($auth => {
			isLoggedIn = $auth.loggedIn;
			currentToken = $auth.token ?? null;
		});
	});

	return {
		get isLoggedIn() {
			return isLoggedIn;
		},
		get currentToken() {
			return currentToken;
		}
	};
}
