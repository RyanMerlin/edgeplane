import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://edgeplane.ai',
  base: '/',
  redirects: {
    '/concepts/missions-klusters-tasks/': '/concepts/domains-missions-tasks/',
    // /install.sh handled by public/_redirects (Cloudflare Pages server-side redirect)
  },
  integrations: [
    starlight({
      title: 'EdgePlane',
      favicon: '/favicon.ico',
      description: 'Control plane for AI agents and human collaborators — structured missions, durable task ownership, and governed artifact publication.',
      head: [
        {
          tag: 'script',
          attrs: { async: true, src: 'https://www.googletagmanager.com/gtag/js?id=G-NCWY6TL7EW' },
        },
        {
          tag: 'script',
          content: "window.dataLayer=window.dataLayer||[];function gtag(){dataLayer.push(arguments)}gtag('js',new Date());gtag('config','G-NCWY6TL7EW');",
        },
      ],
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/RyanMerlin/edgeplane' },
      ],
      sidebar: [
        { label: 'Getting Started', autogenerate: { directory: 'getting-started' } },
        { label: 'Concepts', autogenerate: { directory: 'concepts' } },
        { label: 'Architecture', autogenerate: { directory: 'architecture' } },
        { label: 'Guides', autogenerate: { directory: 'guides' } },
        { label: 'Reference', autogenerate: { directory: 'reference' } },
        { label: 'ADRs', autogenerate: { directory: 'adr' } },
      ],
    }),
  ],
});
