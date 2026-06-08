---
title: "ADR-0005: edgeplane run as the Unified Agent Launcher"
description: Decision to retire edgeplane launch and unify all agent launch paths under edgeplane run.
---

**Status:** Accepted  
**Shipped:** v0.13.0

## Context

EdgePlane had two agent launch surfaces:

- `edgeplane run <runtime>` — the newer path for ACP-native runtimes (claude, codex, gemini)
- `edgeplane launch <agent>` — the older path for driver agents (openclaw, custom, and the original claude shim)

This split caused confusion:
- New users couldn't find all available runtimes in one place
- Gemini was a shim wrapped around `launch`; it worked inconsistently
- `openclaw` and `custom` were `launch`-only, not visible in `run --help`
- Generated lifecycle hooks invoked `edgeplane claude hook <event>` (a non-existent subcommand)

## Decision

Retire `edgeplane launch` entirely. All agents launch through `edgeplane run <runtime>`:

| Runtime | Was |
|---------|-----|
| `claude` | `edgeplane run claude` (unchanged) |
| `codex` | `edgeplane run codex` (unchanged) |
| `gemini` | `edgeplane run gemini` (was a shim over `launch`) |
| `goose` | `edgeplane run goose` (was `launch`-only) |
| `openclaw` | `edgeplane run openclaw` (was `launch`-only) |
| `custom` | `edgeplane run custom` (was `launch`-only) |

`edgeplane launch <anything>` now returns an unrecognized-subcommand error.

## Consequences

**Positive:**
- Single entry point — operators learn one command
- All runtimes visible in `edgeplane run --help`
- Claude lifecycle hooks work (`edgeplane run claude hook <event>` is valid)
- Gemini, openclaw, custom get first-class `run` parity (mode flags, profile flags, mission flag)
- Internal code unified: one `RunDispatch` replaces two dispatch paths

**Negative:**
- Breaking change for scripts using `edgeplane launch`
- Migration: replace `edgeplane launch <agent>` with `edgeplane run <agent>`

## Alternatives Considered

**Keep both surfaces:** Rejected — confusion compounds as new runtimes are added. A single canonical surface is worth the one-time migration cost.
