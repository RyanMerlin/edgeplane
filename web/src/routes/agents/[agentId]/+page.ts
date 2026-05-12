// The conversation pane reaches for `window` (WebSocket) at mount and
// needs runtime auth + node resolution. There's no static HTML we can
// pre-render usefully, so skip prerender + SSR for this route.
export const prerender = false;
export const ssr = false;
