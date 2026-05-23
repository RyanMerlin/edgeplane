---
title: "ADR 0001: Project Documentation Catalog"
description: Introduce machine-readable catalog YAML files as a navigational layer for agent and engineer onboarding.
---

**Status:** Accepted  
**Date:** 2026-03-21

## Context

MissionControl has useful narrative documentation but it is fragmented for agent consumption. Agents need to grep through the codebase or read many docs sequentially to understand subsystem status, commands, gaps, and navigation. Doc drift between files creates confusion and slows onboarding.

## Decision

Introduce `docs/catalog/*.yaml` as a machine-readable status and topology layer.

- One YAML file per subsystem domain
- `index.yaml` as the single agent entry point
- `schema.yaml` as the field contract
- CI validation to prevent catalog rot

## Consequences

**Positive:**
- Easier agent and new-engineer onboarding
- Single entry point for catalog queries — agents can navigate the codebase without grepping

**Negative:**
- Some maintenance burden when adding subsystems or changing maturity status
- CI must enforce structural validity to prevent catalog rot

**Neutral:**
- Does not replace Markdown docs — catalog is navigational only, not a second source of truth for implementation detail

See [ADR 0002](/missioncontrol/adr/0002-source-of-truth/) for the explicit source-of-truth boundaries that govern what the catalog should and should not contain.
