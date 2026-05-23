import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

const API_DEV = process.env.EP_DEV_API ?? 'http://localhost:8008';

// All known API path prefixes. Proxy these to the backend in dev; everything
// else is served by the Vite/SvelteKit frontend.
const API_PATHS = [
	'/agents', '/ai', '/approvals', '/artifacts',
	'/auth',
	'/budgets',
	'/docs', '/domains',
	'/event-triggers', '/evolve', '/explorer',
	'/feedback',
	'/governance',
	'/hooks',
	'/ingest', '/integrations',
	'/mcp', '/me', '/mesh', '/missions',
	'/onboarding', '/ops',
	'/packs', '/persistence', '/profiles',
	'/readyz', '/remotectl', '/runs', '/runtime',
	'/scheduled-jobs', '/schema-pack', '/search', '/skills',
	'/tasks', '/tools',
	'/work',
	'/ws',
];

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		proxy: Object.fromEntries(
			API_PATHS.map((p) => [p, { target: API_DEV, changeOrigin: true, ws: p === '/ws' }])
		)
	}
});
