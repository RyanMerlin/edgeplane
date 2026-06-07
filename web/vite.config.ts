import path from 'path';
import react from '@vitejs/plugin-react';
import { TanStackRouterVite } from '@tanstack/router-plugin/vite';
import tailwindcss from '@tailwindcss/vite';
import { type ProxyOptions, defineConfig } from 'vite';

// Dev proxy to the backend. Override the target with EP_DEV_API to point a
// local frontend at a remote tower. EP_DEV_SESSION (an ep_session_token value)
// authenticates the proxy to that remote tower by injecting the session cookie
// on every upstream request — frontend-only dev against a live backend. Both
// are no-ops in normal local dev.
const apiProxy: ProxyOptions = {
  target: process.env.EP_DEV_API ?? 'http://localhost:8008',
  changeOrigin: true,
  ws: true,
};
if (process.env.EP_DEV_SESSION) {
  const cookie = `ep_session_token=${process.env.EP_DEV_SESSION}`;
  apiProxy.configure = (proxy) => {
    proxy.on('proxyReq', (proxyReq) => proxyReq.setHeader('cookie', cookie));
    proxy.on('proxyReqWs', (proxyReq) => proxyReq.setHeader('cookie', cookie));
  };
}

export default defineConfig({
  plugins: [
    TanStackRouterVite({
      target: 'react',
      autoCodeSplitting: true,
      routeFileIgnorePattern: '\\.test\\.(tsx|ts)$',
    }),
    react(),
    tailwindcss(),
  ],
  server: {
    host: true,
    // Dev server only — allow LAN/Tailscale hostnames (Vite blocks unknown Host
    // headers by default).
    allowedHosts: true,
    proxy: {
      '/api': apiProxy,
    },
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  build: {
    outDir: 'dist',
  },
  base: '/',
});
