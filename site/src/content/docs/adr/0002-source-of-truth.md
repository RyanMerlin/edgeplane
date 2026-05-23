---
title: "ADR 0002: Source-of-Truth Boundaries"
description: Explicit boundaries for each documentation layer — catalog, Markdown, OpenAPI, code, and ADRs.
---

**Status:** Accepted  
**Date:** 2026-03-21

## Context

Without explicit boundaries, the docs catalog risks becoming a second source of truth for implementation detail — duplicating OpenAPI schemas, narrative docs, and code behavior. This creates drift and makes it unclear which source to trust when they disagree.

## Decision

Each layer has exactly one type of content it owns:

| Layer | Source of Truth | Location |
|-------|----------------|----------|
| Subsystem inventory, status, navigation | Catalog | `docs/catalog/*.yaml` |
| Narrative and operator guidance | Markdown docs | `docs/*.md` |
| HTTP API contract | OpenAPI schema | `/api/docs` (generated) |
| Implementation behavior | Code and tests | source + test files |
| Architectural decisions | ADRs | `docs/adr/` |

## Consequences

**Positive:**
- Catalog files stay concise (~80 lines max) — navigational only
- OpenAPI is the authoritative reference for endpoint schemas; catalog references it, does not re-encode it
- ADRs capture the *why* for decisions; code captures the *how*

**Rules:**
- When adding a new subsystem: add a domain YAML, update `index.yaml`, run the catalog validator
- When adding a new endpoint: update the OpenAPI schema — do not document request/response shapes in Markdown
- When making an architectural decision: write an ADR before writing code
