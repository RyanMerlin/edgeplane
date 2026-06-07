# EdgePlane Web — Design-System Pivot to Linear (with EdgePlane flare)

**Status:** DESIGN — mockup-first; awaiting sign-off on the hosted static mockup before any React work.
**Date:** 2026-06-07
**Northstar:** [linear.app](https://linear.app) — adopt its design *language* (not a pixel clone), grounded in live token extraction (see token table below).
**Builds on:** the navigation IA from `2026-06-07-web-navigation-design.md` (left sidebar, two spines, Console dropped). This pivot is the *visual system* layered on that IA.

## Why a hard pivot

The v1 design (`app.css`) is GitHub-dark + **monospace-only, "no pills"** — a terminal aesthetic. It reads as a prototype, not a product. Reactive per-element fixes (alignment, glyph, headings) were never going to reach "professional." We anchor to Linear — the benchmark for dense, premium operator consoles — and conform every surface to one token system.

## v1 rules this pivot SUPERSEDES

- **Font:** monospace-only → **Inter Variable** for all UI text. Monospace (JetBrains Mono) is *retained only* for machine data: agent `public_id`s, node ids, JSON/code, timestamps.
- **Buttons:** "no pills, max 3px radius" → **Linear pill buttons** (`9999px`). (Tags stay square at `2px` — consistent with both systems.)
- **Palette:** GitHub-dark (`#0d1117`) → Linear warm-black (`#08090A`) with **rgba-opacity borders** and **border-based elevation (zero shadows)**.

## EdgePlane flare (how we differ from a Linear clone)

1. **Accent = EdgePlane blue `#58a6ff`** (kept from v1), NOT Linear indigo `#5E6AD2`. Same premium neutral system, distinct brand pop. *(Easy to swap on the mockup if you want a more distinct hue.)*
2. **Hexagon ⬡ motif** as the logo mark + empty-state/accent glyph (already in the app).
3. Monospace for machine data is an EdgePlane signature that nods to the operator/terminal roots without the whole UI being mono.

## Token system (Linear-anchored → EdgePlane vars)

Live-extracted from linear.app. The React app's `app.css` `:root` will be replaced with this.

```css
:root[data-theme="dark"] {
  /* warm-coherent neutrals — Linear's structure, warmed ~28° so rust harmonizes
     (merlin-site's own lesson: rust on COOL neutrals clashed). Depth via steps, no shadows. */
  --base:#0c0a09; --frame:#141210; --surface:#16130f; --card:#110e0c;
  --raised:rgba(255,240,230,0.055); --raised-2:rgba(255,240,230,0.10); --input:rgba(255,240,230,0.025);
  /* borders — warm white-alpha, opacity not color */
  --border:rgba(255,238,224,0.09); --border-subtle:rgba(255,238,224,0.06); --border-strong:#2a2420;
  /* text — warm off-white */
  --text:#F2EDE6; --text-secondary:#D6CDC2; --muted:#9A8F84; --dim:#6E6459;
  /* brand (EdgePlane flare = merlin-site rust orange) + semantic */
  --accent:#e75b2a; --accent-dim:rgba(231,91,42,0.16); --accent-deep:#b8451e;
  --ok:#57d08a; --warn:#E6B355; --err:#F2685C; --info:#4a9eda;
  /* type */
  --font:"InterVariable","Inter","SF Pro Display",-apple-system,sans-serif;
  --mono:"JetBrains Mono","SF Mono",ui-monospace,monospace; /* data only */
  /* radii */ --r-xs:2px; --r-md:6px; --r-lg:8px; --r-xl:12px; --r-pill:9999px;
  /* metrics */ --sidebar:232px;
}
```

Key fidelity notes (the "things clones get wrong"): warm-black not `#000`; borders are `rgba(255,255,255,N)`; Inter Variable weights **510 / 590** (between Medium and SemiBold) for nav/labels; **no uppercase tracked group labels**; depth via background steps, never shadows.

## Shell anatomy (Linear-style)

- **Sidebar 232px**, transparent over `--frame`. Hex logo + "EdgePlane" wordmark (13px/400). A search row + (optional) action. Nav rows: **28px tall, 6px radius, 13px / weight 510**, `--text-secondary` idle, hover `--raised`, active = brighter bg + `--text` (NO left accent bar). **No WHO/WHAT group labels.**
- Nav items: **Dashboard · Agents · Domains**, then **Feed · Governance** (no group headings — Linear-style).
- **Bottom account control** (left-justified, matches nav rows): hex/initials avatar + name → opens a popover menu: subject, **Settings → Onboarding** (nested), theme toggle, Logout. **Onboarding NEVER appears in the rail** — only under Settings.
- **Content**: 44px breadcrumb header (`border-bottom: --border-subtle`) + full-height body.

## Mockup scope (what the hosted static mockup covers)

One self-contained `mockup.html`, hosted for click-through review:
1. **Dashboard** — Fleet + Work summary cards + Recent activity list.
2. **Agents** — compact Linear-density table (status dot, name, mono id, node, last-seen, source tag).
3. **Agent detail** — breadcrumb header, identity/status strip, **conversation pane (Claude-style messages + composer)** as the body.
4. **Domains** — Domain › Mission › Task tree (Linear list density).
5. **Account menu** — the Settings → Onboarding nesting.

## Migration approach (after mockup lock)

1. Replace `app.css` `:root` tokens + base (`body` font → Inter Variable; load the web font) — one commit; verify the existing shell/screens reflow.
2. Conform the `components/shell/*` + route screens to the locked mockup, component by component (TDD where logic changes; pure style otherwise), each gated on build+test+lint.
3. No new IA — this is visual conformance to the already-approved sidebar IA.

## Out of scope (unchanged from the nav spec)

Realtime transport (feed/conversation connectivity), the AI Console runtime, Domains nested routes (Pass 2), and the `/auth/me` email field for real avatar initials.
