# Master Design Documents (MDDs)

A **Master Design Document** is the living, typed contract describing what an
Edgeplane binary does, what it owns, and what it talks to. One file per binary.

**MDD ≠ ADR.** ADRs explain *why* we decided something; they are append-only
history. MDDs encode *what* a binary is right now and what it will become.
ADRs reference the MDD they were written against; MDDs reference ADRs only
for stable rationale.

**MDD ≠ entities.md.** `entities.md` is the canonical reference for shared
data primitives (Domain, Mission, Task, MeshTask, Artifact, Agent, …) that
span the whole system. MDDs describe a single binary's surface, dependencies,
and roadmap — and reference entities.md for any primitive they touch.

**MDD ≠ OpenAPI.** OpenAPI describes a wire protocol in exhaustive detail.
MDDs describe a binary's intent and contract at a level a reviewer can scan
in two minutes. Link to OpenAPI from the MDD when relevant; don't duplicate.

---

## When to update

Update the MDD when any of the following changes:

- A new top-level subcommand is added, renamed, or removed
- A binary starts or stops owning an entity (CRUD on something in `entities.md`)
- An external dependency is added or dropped (Postgres, Tailscale, MCP server, etc.)
- A binary's role in the architecture shifts (e.g., gains a new bind port,
  starts serving an API it previously consumed)
- A `status: proposed` item lands or is rejected — flip the status field and
  commit the diff

Don't update on bugfixes, refactors that don't change the surface, or
internal-only changes. The MDD is the contract, not the implementation.

---

## The schema

```yaml
# Required header
schema_version: 1                       # integer, bumps on breaking MDD-schema changes
binary: edgeplaned                      # canonical name, matches Cargo.toml [[bin]]
crate: edgeplaned                       # rust crate name
status: implemented                     # proposed | accepted | implemented | deprecated
stability: stable                       # development | alpha | beta | stable
owner: edgeplane-core                   # team or maintainer reference
last_reviewed: 2026-05-24

# Purpose
description: |
  One paragraph. What does this binary do, for whom, at what scope.

position_in_stack: |
  One sentence describing where this binary sits relative to the others.

# Runtime characteristics
runtime:
  kind: long_running | one_shot
  systemd_unit: edgeplaned.service       # null if not a service
  binds: ["tcp:127.0.0.1:9090"]          # list, [] if none
  install_path: /usr/local/bin/edgeplaned
  config_paths:
    - ~/.edgeplane/edgeplaned/cron.toml
    - ~/.edgeplane/edgeplaned/modules.yaml  # example future addition
  state_paths:
    - ~/.edgeplane/edgeplaned/state/
  env_vars:
    - name: EP_BIND
      required: false
      default: "0.0.0.0:8008"

# Top-level commands. Status and stability per-command.
commands:
  - name: run
    status: implemented
    stability: stable
    description: "Run the daemon supervisor + task loops."
    subcommands: []
  - name: modules
    status: proposed
    stability: development
    spec_ref: docs/superpowers/specs/2026-05-24-edgeplane-recurring-tasks-design.md
    description: "Manage RecurringTaskTemplates and their emitted runs."
    subcommands: [list, describe, run, apply, disable, enable, logs, artifact]

# Entities from entities.md that this binary owns or mutates
entities_owned:
  - name: MeshTask
    ref: docs/architecture/entities.md#meshtask
    operations: [create, claim, complete, fail]

# External services this binary depends on (databases, third-party APIs, etc.)
external_dependencies:
  - name: postgres
    purpose: persistent state (tasks, runs, artifacts)
    via: edgeplane-tower API
    required: true

# Other EP binaries this binary depends on
internal_dependencies:
  - binary: edgeplane-tower
    via: HTTP API
    purpose: persistent state access

# What this binary produces
outputs:
  - type: artifact
    target: postgres.artifact via tower
  - type: log
    target: stdout + journal
    format: structured json (tracing-subscriber)

# Inbound: who calls this binary
consumed_by:
  - binary: edgeplane
    via: HTTP API + mesh signal

# References to ADRs that constrain this binary's shape
adr_refs:
  - id: "0003"
    title: edgeplane CLI hierarchy hard cutover
    path: docs/adr/0003-edgeplane-cli-hierarchy-hard-cutover.md

# OPTIONAL: free-form section for nuts-and-bolts a reviewer needs
notes: |
  Multi-line YAML block. Anything that doesn't fit the typed fields above
  but a reviewer would want to know. Migration plans, known constraints,
  upgrade paths.
```

### Field semantics

| Field | Required | Notes |
|---|---|---|
| `schema_version` | yes | Plain integer. Bumps on breaking MDD-schema changes only. |
| `binary`, `crate` | yes | Must match the Cargo.toml entries. Used by CI for validation. |
| `status` | yes | Document-level lifecycle of the binary itself. |
| `stability` | yes | Stability of the binary as a whole. Mixed-stability binaries use per-command stability. |
| `description` | yes | ~3 sentences. Skim-readable. |
| `runtime` | yes | What does this thing look like at runtime — process kind, ports, paths. |
| `commands` | yes for CLI/daemon | Skip for pure libraries. |
| `entities_owned` | yes | Empty list `[]` is acceptable. |
| `external_dependencies` | yes | Empty list is acceptable. |
| `internal_dependencies` | yes | Empty list is acceptable. |
| `adr_refs` | optional | List relevant ADRs. |
| `notes` | optional | Free-form. Use sparingly. |

### Status state machine

```
proposed ──accept──> accepted ──ship──> implemented ──sunset──> deprecated
   │                    │                                            │
   └────reject──────────┴──────────cancel─────────────────────────reject
```

- `proposed`: design exists (spec_ref required), not yet built
- `accepted`: approved for implementation; building soon
- `implemented`: shipped and running in production
- `deprecated`: still works but slated for removal; record removal_target

Per-command status uses the same state machine. Use `spec_ref` to point at
the design doc that supports a proposed status.

### Stability levels (OTel Weaver pattern)

| Level | Meaning | Breaking changes? |
|---|---|---|
| `development` | Active design, surface unstable | Anytime, no notice |
| `alpha` | Buildable, expect breakage | With release notes |
| `beta` | Behavior stable, surface may shift | One deprecation cycle |
| `stable` | Surface frozen, breaking changes require ADR | Major version bump only |

---

## Why YAML and not Pkl/CUE

YAML is universally editable. The barrier-to-contribution wins.

The schema described above can (and probably should, later) be enforced via
a Pkl or CUE definition checked in CI. That validation is a separate concern
from the authoring format. Use Pkl/CUE for the schema, YAML for the artifacts.

When CI validation lands, it should:

1. Parse each MDD YAML
2. Validate against the schema
3. Run `<binary> --help` (or its discover surface) and check that all
   commands with `status: implemented` exist in the actual binary
4. Run `<binary> --help` and flag any command in the binary that's missing
   from the MDD (drift in the other direction)

Validation is advisory in early days. Lock the policy when the team can
trust the MDDs are kept current.

---

## Files in this directory

- `INDEX.md` — this file (the schema reference)
- `edgeplane.yaml` — MDD for the `edgeplane` CLI
- `edgeplaned.yaml` — MDD for the `edgeplaned` daemon
- `edgeplane-tower.yaml` — MDD for the `edgeplane-tower` API server

Each YAML is independently readable. Cross-references between binaries
appear in `internal_dependencies` and `consumed_by`.

---

## Comparison with other patterns

- **K8s CRD**: same structural idea (typed contract via schema) applied to
  binaries instead of API resources. CRDs validate resource instances; MDDs
  describe binary contracts.
- **Terraform modules**: directory-as-resource pattern. Not used here because
  binary contracts don't compose; flat single-file wins for readability.
- **Helm values.yaml + templates/**: split powerful for templated deployment
  configs. Not applicable to binary contracts.
- **OTel Semantic Conventions**: closest precedent. Stability markers, machine-
  validated, registry of typed YAML files. The `stability` field is borrowed
  directly from there.
- **OpenAPI**: API protocol details. MDD links to OpenAPI for wire-level specs;
  doesn't duplicate.
- **TLA+/Alloy**: formal methods. Not the primary format. Use as supplementary
  annotation for concurrency-sensitive sections if and when needed (AWS practice).
- **ADR**: decision history. Strictly complementary to MDD, not duplicative.

---

## Authorship origin

This MDD pattern was synthesized 2026-05-24 from a research survey covering
K8s CRDs, OTel Weaver, Pkl/CUE, AWS TLA+ practice, GitHub Actions, and
modern RFC patterns. The full research output is preserved in the
brainstorming session log; the load-bearing decisions are captured in
the schema above.
