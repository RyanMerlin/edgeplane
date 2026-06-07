/**
 * /matrix — consolidated into the Feed route's "Raw" sub-view (routes/feed.tsx).
 *
 * The raw event stream now lives behind the Live|Raw toggle on /feed
 * (rendered by components/events/RawEventList.tsx). This route redirects there
 * so existing /matrix links keep working.
 */

import { createFileRoute, redirect } from '@tanstack/react-router';

export const Route = createFileRoute('/matrix')({
  beforeLoad: () => {
    throw redirect({ to: '/feed' });
  },
});
