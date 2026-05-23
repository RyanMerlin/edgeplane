# Changelog

All notable changes to edgeplane, edgeplaned, and edgeplane-tower are recorded here. The three binaries ship in lockstep — `/VERSION` is the source of truth and `scripts/set-version.sh <new>` bumps all three in one step.

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

## [0.10.0] — 2026-05-23

Initial release under the **edgeplane** name. Forked from
[`RyanMerlin/missioncontrol`](https://github.com/RyanMerlin/missioncontrol)
at commit `ceff9f1` (the post-rename baseline that landed the
Domain / Mission / Task entity vocabulary).

### Brand identity

- Binaries: `edgeplane` (CLI), `edgeplaned` (per-node daemon), `edgeplane-tower` (server)
- Environment namespace: `EP_*` (replaces the prior `MC_*` prefix)
- Paths: `~/.edgeplane/` for user state, `~/.ep/` shorthand
- Org: edgeplane.ai

### Architectural changes from the fork point

- Single-workspace Cargo layout — all 12 crates resolve dependencies through
  a unified workspace root (previously 3 sibling workspaces with independent
  lockfiles)
- Cruft removal: disabled CI workflow referencing a missing `backend/`
  directory; design-validation prototypes whose ideas have shipped in the
  real implementation; meta scripts left over from prior renames
- No backward-compat shims to MissionControl — environment variables, file
  paths, binary names, HTTP headers, and MCP tool argument fields are all
  edgeplane-native from line 1

### Inherited from MissionControl

The engineering substrate — entity model (Domain / Mission / Task),
governance, overlap detection, persistent agent sessions via ACP, MCP-native
tool surface, three-tier persistence (Postgres + S3 + Git) — is all
inherited at maturity. See the upstream [`RyanMerlin/missioncontrol`](https://github.com/RyanMerlin/missioncontrol)
repo (archived) for development history and architectural decision records
prior to the fork.
