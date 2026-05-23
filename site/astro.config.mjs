import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://ryanmerlin.github.io',
  base: '/missioncontrol',
  redirects: {
    '/concepts/missions-klusters-tasks/': '/concepts/domains-missions-tasks/',
  },
  integrations: [
    starlight({
      title: 'MissionControl',
      description: 'Control plane for AI agents and human collaborators — structured missions, durable task ownership, and governed artifact publication.',
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/RyanMerlin/missioncontrol' },
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
