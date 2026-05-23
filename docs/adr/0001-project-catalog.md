# ADR 0001: Project Documentation Catalog

**Status:** Accepted
**Date:** 2026-03-21

## Context

Edgeplane has useful narrative documentation but it is fragmented for agent consumption. Agents currently need to grep through the codebase or read many docs sequentially to understand subsystem status, commands, gaps, and navigation. Doc drift between files (e.g. architecture.md vs README.md) creates confusion.

## Decision

Introduce `docs/catalog/*.yaml` as a machine-readable status and topology layer.
- One YAML file per subsystem domain
- `index.yaml` as the single agent entry point
- `schema.yaml` as the field contract
- CI validation to prevent catalog rot

## Consequences

- Easier agent and new-engineer onboarding
- Some maintenance burden when adding subsystems or changing maturity
- CI enforces that catalog stays structurally valid
- Does not replace Markdown docs; catalog is navigational only
