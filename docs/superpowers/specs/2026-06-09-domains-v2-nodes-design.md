# EdgePlane Web — Domains v2 + Nodes Navigation

**Status:** APPROVED  
**Date:** 2026-06-09  
**Builds on:** `2026-06-07-linear-design-system-pivot.md` (token system + shell already in progress)

---

## 1. What this is

Pass 2 of the Domains navigation — replaces the flat 3-pane Explorer with a proper hierarchical navigation model. Adds a Nodes page (Kubernetes-style node visibility). The Domains section of the sidebar becomes a live, expandable tree. Entity pages (Domain, Mission) are full-width with tabs and Monaco-powered document editors for NORTHSTAR and BRIEF narratives.

---

## 2. Navigation rail changes

Nodes is inserted between Agents and the Domains section. Domains becomes a sidebar **section** (not a single nav item) — domains expand inline, missions appear as sub-items.

```
⬡ EdgePlane
──────────────────
◇  Dashboard
◉  Agents
▦  Nodes           ← NEW
──────────────────
▤  DOMAINS         ← section label (clicking → /domains overview)
   ▾ Apollo        ← expanded domain
     ● Warehouse rebuild
     ● Briefing pipeline
   ▸ Mercury
   ▸ Homelab
──────────────────
≋  Feed
⚖  Governance
```

- Clicking a domain header expands/collapses it inline; also navigates to `/domains/:id`
- Clicking a mission sub-item navigates to `/domains/:id/missions/:mid`
- Clicking the "DOMAINS" section label navigates to `/domains` (overview)
- Sidebar tree data from the existing `GET /api/explorer/tree` (30s refetch cadence)

---

## 3. Route map

| Route | Component | Notes |
|---|---|---|
| `/domains` | `DomainsOverviewPage` | Interactive tree/summary of all domains — replaces the 3-pane Explorer |
| `/domains/:domainId` | `DomainPage` | Domain entity page (tabs) |
| `/domains/:domainId/missions/:missionId` | `MissionPage` | Mission entity page (tabs) |
| `/nodes` | `NodesPage` | Node list table |
| `/nodes/:nodeId` | `NodeDetailPage` | Node detail |

`/explorer` redirect to `/domains` is preserved.  
The existing `domains.tsx` (3-pane Explorer) is retired and replaced by these routes.

---

## 4. Domains overview page (`/domains`)

An interactive tree/summary — the "all teams" view. Shows all domains with their missions, status, and task counts at a glance. Replaces the old 3-pane Explorer as the landing surface for the Domains section.

- Domain cards or Linear-density rows: name, status tag, mission count, task count
- Each domain is expandable inline to show its missions
- Click domain → `/domains/:id`
- Click mission → `/domains/:id/missions/:mid`
- Data: `GET /api/explorer/tree` (same source as sidebar)

---

## 5. Domain page (`/domains/:domainId`)

**Header:** name, status tag, `N missions · N tasks`, one-line description.

**Tabs:**

| Tab | Default | Content |
|-----|---------|---------|
| Northstar | ✓ | Monaco editor full-width. Loads `GET /api/domains/:id/northstar`. Save → `PUT`. Shows `v{N} · saved {time}` footer. |
| Missions | | List rows: name, status dot+tag, task count. Click → `/domains/:id/missions/:mid`. |
| Overview | | Description, created date, status breakdown stats. |

---

## 6. Mission page (`/domains/:domainId/missions/:missionId`)

**Header:** name, status tag, task count, breadcrumb back to parent domain.

**Tabs:**

| Tab | Default | Content |
|-----|---------|---------|
| Brief | ✓ | Monaco editor full-width. Loads `GET /api/domains/:id/m/:mid/brief`. Save → `PUT`. Same `v{N} · saved {time}` footer. |
| Tasks | | List rows: title, status dot+tag, owner, description snippet. Click → right slide-over panel (not a new route). |
| Overview | | Description, created date, stats. Uses existing `GET /api/explorer/node/mission/:id`. |

---

## 7. Nodes page (`/nodes`)

Linear-density table. Data from `GET /api/runtime/nodes`.

Columns: **Status** (dot+tag) · **Name** (node_name) · **Tailscale FQDN** · **Runtime version** · **Last heartbeat** (relative) · **Agents** (count)

Click row → `/nodes/:nodeId`.

---

## 8. Node detail page (`/nodes/:nodeId`)

**Header:** hostname, status tag, tailscale IP/FQDN, trust tier, runtime version.

**Agents section:** table of agents where `node_id` matches this node. Uses `GET /api/agents` filtered client-side (or by query param if supported).

**Info section:** capacity JSON rendered as key/value pairs, labels, last heartbeat timestamp, registered date.

Read-only. No cordon/drain actions in scope.

---

## 9. Monaco editor

- Package: `@monaco-editor/react` (new dependency)
- Language: `"markdown"`, theme: `"vs-dark"`
- Dynamic import — lazy-loaded so it doesn't bloat the initial bundle
- No live preview — raw markdown with syntax highlighting only
- Edit/Save pattern: editor is always editable; Save button top-right; optimistic version counter in footer
- Error state: if save fails, show inline error; don't discard content

---

## 10. API endpoints (all existing, no new backend work)

| Endpoint | Used by |
|----------|---------|
| `GET /api/explorer/tree` | Sidebar tree + `/domains` overview |
| `GET /api/domains/:id/northstar` | Northstar tab load |
| `PUT /api/domains/:id/northstar` | Northstar tab save |
| `GET /api/domains/:id/m/:mid/brief` | Brief tab load |
| `PUT /api/domains/:id/m/:mid/brief` | Brief tab save |
| `GET /api/runtime/nodes` | Nodes list |
| `GET /api/runtime/nodes/:id` | Node detail header |
| `GET /api/agents` | Node detail agents section |
| `GET /api/explorer/node/mission/:id` | Mission Overview tab |

---

## 11. Out of scope

- Creating or editing domain/mission metadata (name, description, status) from the UI
- Creating new domains or missions from the UI
- Real-time collaborative editing of NORTHSTAR/BRIEF
- Task sub-routes — task detail is a slide-over panel only, no `/tasks/:id` route
- The existing 3-pane Explorer component — retired; replaced by DomainsOverviewPage

---

## 12. File map (new + changed)

**New routes:**
- `web/src/routes/domains.index.tsx` — DomainsOverviewPage
- `web/src/routes/domains.$domainId.tsx` — DomainPage (tabs)
- `web/src/routes/domains.$domainId.missions.$missionId.tsx` — MissionPage (tabs)
- `web/src/routes/nodes.tsx` — NodesPage layout
- `web/src/routes/nodes.index.tsx` — NodesPage list
- `web/src/routes/nodes.$nodeId.tsx` — NodeDetailPage

**New components:**
- `web/src/components/domains/NarrativeEditor.tsx` — Monaco editor wrapper (NORTHSTAR + BRIEF shared)
- `web/src/components/domains/TaskSlideOver.tsx` — right slide-over for task detail
- `web/src/components/nodes/NodesTable.tsx` — node list table

**Changed:**
- `web/src/components/shell/Sidebar.tsx` — Domains section becomes inline tree; Nodes added
- `web/src/components/shell/navModel.ts` — add Nodes, restructure Domains
- `web/src/lib/queryKeys.ts` — add northstar, brief, nodes query keys
- `web/src/routes/domains.tsx` — retired (replaced by domains.index.tsx)

**Package:**
- `web/package.json` — add `@monaco-editor/react`
