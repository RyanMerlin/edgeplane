# Web Frontend Revamp Plan

This document is the implementation plan for the current MissionControl web frontend rewrite.
It is intentionally operational: it focuses on what to build next, in what order, and what each step must prove before moving on.

## Why this needs a revamp

The current frontend works, but it has outgrown its structure:

- The main route is a monolithic Svelte page that mixes navigation, data loading, AI session management, explorer state, onboarding, governance, and telemetry in one file.
- The page-level experience is functional but not yet a clear product surface.
- Shared API logic is centralized in a large helper module instead of being organized by domain.
- The shell is minimal, which makes the app feel more like a dashboard prototype than a cohesive product.

The goal is not to change backend behavior first. The goal is to rebuild the frontend around clearer product boundaries while preserving all existing capabilities.

## Target Outcome

After the revamp, the web app should feel like a deliberate product with:

- A clear app shell and navigation model.
- AI console as the primary workflow.
- Explorer, governance, onboarding, and matrix telemetry as distinct secondary surfaces.
- Shared UI primitives for cards, panels, status chips, tables, forms, and empty states.
- Smaller route files and smaller domain components.
- A cleaner state model with less coupling between unrelated features.

## Non-Goals

Do not try to solve everything at once:

- Do not redesign backend APIs as part of the first frontend pass.
- Do not add new user-facing product areas unless they support the existing MissionControl flows.
- Do not keep the monolithic `+page.svelte` shape and only restyle it.
- Do not build a generic design system before the app structure is clarified.

## Current Frontend Shape

The current `web/src` layout is a useful baseline:

- `web/src/routes/+page.svelte` contains most of the application logic and UI.
- `web/src/routes/+layout.svelte` only provides shell chrome, theme, and auth actions.
- `web/src/app.css` already contains the visual foundation.
- `web/src/lib/api.ts` holds most shared backend calls and types.
- `web/src/lib/ai-console/*` contains the AI console-specific additions.

The first priority is to split these responsibilities without breaking the existing flows.

## Revamp Sequence

### Phase 0: Baseline and freeze the current behavior

Before changing layout or structure, capture the current frontend behavior so regressions are easy to spot.

Deliverables:

- Record the current page areas and flows that must remain available.
- Identify the minimum route set that the new UI must preserve.
- Keep the existing working interactions intact while refactoring.

Acceptance criteria:

- The AI console still loads and can send turns.
- Explorer still renders tree data.
- Governance and onboarding still expose their existing controls.
- The build and check commands keep passing during the refactor.

### Phase 1: Define the new information architecture

This is the key design step. The page structure should be decided before component extraction.

Recommended shape:

- `AI Console` as the default primary route.
- `Explorer` as a browsable operational view.
- `Governance` as the policy and audit surface.
- `Onboarding` as the setup and integration path.
- `Matrix` as the telemetry and event stream surface.

Decisions to make in this phase:

- Which route is the default landing page.
- Which pages should live under a shared shell versus standalone flows.
- Which content belongs in tabs, and which content deserves separate routes.
- Which navigation items are primary versus secondary.
- Whether the existing tabbed UI becomes route-based navigation or remains tabbed inside one top-level route.

Recommended direction:

- Move from one large tabbed page to route-based structure for the major surfaces.
- Keep smaller in-page tabs only where the interaction truly belongs to one workflow.

Acceptance criteria:

- The navigation model is documented and stable.
- Each major user task has a clear destination.
- The app no longer depends on one giant route to express all major surfaces.

### Phase 2: Split the monolith into route and domain components

Break `+page.svelte` into focused components with clear ownership.

Suggested extraction order:

1. App shell and page header.
2. AI session list, transcript, composer, approval panel, and event details.
3. Explorer tree, node detail pane, and search/filter controls.
4. Onboarding form sections and manifest preview/output.
5. Governance summary, policy events, and audit views.
6. Matrix telemetry stream and status surfaces.

What to extract into reusable pieces:

- Cards and panels.
- Empty states.
- Status chips and badges.
- Sidebars and split panes.
- Forms and prompt/composer blocks.
- Event logs and detail drawers.

Implementation rule:

- Keep components small enough that each one has one clear job.
- Avoid creating a `components` dump where everything becomes a generic prop soup.

Acceptance criteria:

- `+page.svelte` becomes orchestration, not implementation detail.
- Each major feature area can be reasoned about independently.
- Shared UI pieces are reused instead of duplicated.

### Phase 3: Rebuild the visual system around the app shell

After the structure is split, apply a consistent product-level visual system.

Design priorities:

- Give the shell more hierarchy.
- Make primary actions obvious without overwhelming the page.
- Separate dense operational data from conversational surfaces.
- Use spacing, elevation, and section headings to create scannable structure.
- Keep dark and light themes both usable rather than optimizing only for one.

Specific UI changes to consider:

- Stronger top-level nav and page title structure.
- A persistent left rail or secondary navigation for major sections.
- A more intentional AI console layout with transcript, context, and actions separated.
- Better treatment of empty states and loading states.
- Stronger visual distinction between read-only data, user actions, and system events.

Acceptance criteria:

- The app reads as a product, not a debug dashboard.
- Dense views remain readable.
- The AI console still feels like the primary surface.

### Phase 4: Rationalize state and API boundaries

Once the UI structure is stable, clean up the state model.

Recommended work:

- Split `web/src/lib/api.ts` by domain if it keeps growing.
- Separate AI console API helpers from explorer/governance helpers.
- Move derived UI state into local feature modules where appropriate.
- Reduce cross-feature coupling in the main page component.

Good split candidates:

- `lib/api/ai.ts`
- `lib/api/explorer.ts`
- `lib/api/governance.ts`
- `lib/api/evolve.ts`
- `lib/ui/*` for reusable visual primitives

Acceptance criteria:

- New code is easier to place in the right module.
- Feature changes do not require editing a single large shared file.
- The app state flow is explicit enough that bugs are easier to isolate.

### Phase 5: Accessibility, responsiveness, and performance

Do not treat this as optional polish. The redesign should be usable on smaller screens and with keyboard-only interaction.

Checklist:

- Verify keyboard navigation across the new shell and major controls.
- Ensure buttons, inputs, and status indicators have sensible labels.
- Check mobile layout for transcript, explorer, and policy panes.
- Reduce unnecessary rerenders where state is currently broad and shared.
- Keep long streams and logs usable without forcing the entire app to reflow constantly.

Acceptance criteria:

- Core workflows work on desktop and mobile widths.
- The AI console remains responsive while transcripts and event streams update.
- The app is usable without relying on pointer-only interactions.

## Concrete Next Implementation Steps

If work starts immediately, do it in this order:

1. Create the new route/component structure on paper first, then in code.
2. Extract the AI console into the first dedicated feature component set.
3. Move explorer into its own feature component set.
4. Introduce a real shell navigation model.
5. Split the largest shared API helpers by feature.
6. Restyle the shell and major surfaces after the structure settles.
7. Run the backend and web checks after every refactor slice.

## Suggested First Slices

These are the safest first changes:

- Extract the AI transcript/composer into components while keeping behavior identical.
- Extract the explorer tree and detail pane next.
- Leave onboarding and governance visible, but refactor them after the first two flows are stable.
- Keep the existing backend contract unchanged during the first pass.

## Testing Gate

Each slice should pass the existing checks before it is merged:

- `cd backend && uv run --python ../.venv/bin/python -m py_compile app/main.py`
- `cd backend && uv run --python ../.venv/bin/python -m unittest discover -s tests -q`
- `cd web && npm run check`
- `cd web && npm run build`
- `cd integrations/mc && cargo check -p mc`

If a slice changes UI behavior, add a manual verification note in the PR or commit message describing the flow that was checked.

## Risks To Watch

- A route split can accidentally duplicate shared state if the shell contract is not defined first.
- A visual redesign can regress operator workflows if dense information is hidden behind too many layers.
- A shared API module can become a dumping ground if feature boundaries are not enforced.
- The AI console can become noisy if event/debug content is not clearly separated from the transcript.

## Definition of Done

The web revamp is done when:

- The app has clear routes or sections for the primary workflows.
- `+page.svelte` is no longer the place where all feature logic lives.
- The AI console, explorer, governance, onboarding, and matrix views each have a clear presentation model.
- The shell feels intentional and stable.
- The standard checks pass after the refactor.

