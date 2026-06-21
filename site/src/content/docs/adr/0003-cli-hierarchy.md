---
title: "ADR 0003: CLI Hierarchy Hard Cutover"
description: Adopt a grouped top-level command hierarchy for edgeplane with no legacy aliases.
---

**Status:** Accepted  
**Date:** 2026-03-24

## Context

The `edgeplane` command surface grew organically and mixed concerns at the top level: `tools`, `sync`, `explorer`, `maintenance`, `update`, `compat`, `drift`, `remote`, `evolve`, `login`, `logout`, `whoami` all lived at the root. This made command discovery and onboarding harder and increased ambiguity around where functionality belongs.

Because EdgePlane is in a pilot stage, a hard cutover without backward-compatibility aliases is acceptable.

## Decision

Adopt the following top-level command hierarchy:

**Keep at top level:** `launch`, `serve`, `daemon`, `ops`, `workspace`, `approvals`, `profile`, `init`, `run`, `tui`

**Group into domains:**

| Domain | Commands |
|--------|---------|
| `auth` | `login`, `logout`, `whoami` |
| `admin` | `policy ...`, `governance ...` |
| `data` | `tools ...`, `sync ...`, `explorer ...` |
| `system` | `doctor`, `maintenance ...`, `update ...`, `compat ...`, `drift ...` |
| `agent` | `signal`, `list`, `describe`, `attach`, `cancel`, `cron ...`, `supervise ...`, `evolve ...` |

No legacy aliases are retained.

## Consequences

**Positive:**
- Clear information architecture by user intent
- Less cognitive load for new operators and agent authors
- Better foundation for docs, catalogs, and scripted playbooks

**Negative:**
- Breaking command changes require immediate doc and script updates
- Existing shell history and muscle memory are invalidated at the transition point

## Follow-up

- Update docs and catalog entries to the new hierarchy
- Ensure in-product hints and repair messages reference new command paths
- Maintain [`docs/reference/COMMAND-MAP.md`](https://github.com/edgeplane/edgeplane/blob/main/docs/reference/COMMAND-MAP.md) as the canonical command index

See [Reference: Command Map](/reference/command-map/) for the current full hierarchy.
