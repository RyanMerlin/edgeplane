# ADR 0003: edgeplane CLI Hierarchy Hard Cutover

## Status
Accepted (2026-03-24)

## Context
The `edgeplane` command surface grew organically and mixed concerns at the top level (`tools`, `sync`, `explorer`, `maintenance`, `update`, `compat`, `drift`, `remote`, `evolve`, `login`, `logout`, `whoami`). This made command discovery and onboarding harder, and increased ambiguity around where functionality belongs.

Pilot stage allows a hard cutover without backward-compatibility aliases.

## Decision
Adopt the following top-level command hierarchy:

- Keep top-level: `launch`, `serve`, `daemon`, `ops`, `workspace`, `approvals`, `profile`, `init`
- Add grouped top-level domains:
  - `auth` (`login`, `logout`, `whoami`)
  - `admin` (`policy ...`, `governance ...`)
  - `data` (`tools ...`, `sync ...`, `explorer ...`)
  - `system` (`maintenance ...`, `update ...`, `compat ...`, `drift ...`)
  - `agent` (`remote ...`, `evolve ...`)

No legacy aliases are retained in this phase.

## Consequences
- Pros:
  - Clear information architecture by user intent.
  - Less cognitive load for new operators and agent authors.
  - Better foundation for docs, catalogs, and scripted playbooks.
- Cons:
  - Breaking command changes require immediate doc/script updates.
  - Existing shell history and muscle memory are invalidated.

## Follow-up
- Update docs and catalog entries to the new hierarchy.
- Ensure in-product hints and repair messages reference new command paths.
- Maintain [`docs/reference/COMMAND-MAP.md`](../reference/COMMAND-MAP.md) as the canonical command index.
