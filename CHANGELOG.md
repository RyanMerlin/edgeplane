# Changelog

All notable changes to edgeplane, edgeplaned, and edgeplane-tower are recorded here. The three binaries ship in lockstep — `/VERSION` is the source of truth and `scripts/set-version.sh <new>` bumps all three in one step.

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

## [0.11.0] — 2026-05-25

### Breaking changes

- **`EP_TOKEN` removed.** The static shared-secret authentication path has been
  removed from edgeplane-tower. All callers must authenticate via OIDC
  (interactive), node JWT (machine-to-machine), or service account tokens
  (programmatic). Deployments that relied on `EP_TOKEN` must migrate to one of
  these mechanisms before upgrading. See the [auth documentation](https://edgeplane.ai/guides/oidc/).

### Node JWT authentication

- RS256 JWT module for node identity — sign, verify, and revoke per-node tokens
- `POST /runtime/nodes/register` — public registration endpoint, issues a signed JWT
- `POST /runtime/nodes/{id}/rotate-token` — token rotation with JTI revocation
- `edgeplaned register` subcommand — registers a node and persists identity to
  `/etc/edgeplane/node.json`
- `edgeplane agent node join-token create/get/rotate` — admin CLI for managing
  node enrollment tokens
- `nodetoken` table for JTI-based revocation tracking
- edgeplaned reads node JWT from disk at startup — no env var injection needed

### CLI schema & federated discovery

- `edgeplane cli-schema` — emits a versioned JSON contract (`schema_version: 1`)
  describing the full CLI surface. Any consumer can parse this to discover
  available commands without hardcoded knowledge of the CLI structure.

### Security

- **OIDC hardening:** id_token claims now verified via the provider's userinfo
  endpoint instead of unverified JWT parsing; `redirect_path` sanitized to
  prevent open redirects; user-controlled values HTML-escaped in success pages
- **Proxy auth forwarding:** the `EP_API_PROXY` fallback now forwards the
  `Authorization` header to the backend
- **Bootstrap script hardened:** `EP_BASE_URL` and `EP_TOKEN` (pre-removal)
  changed from insecure defaults to required variables
- **Helm secrets:** sensitive values moved from ConfigMap to K8s Secret template;
  placeholder defaults cleared
- **CI hardening:** all GitHub Actions pinned to commit SHAs; `permissions:`
  blocks added to 4 workflows; `cargo audit` job added; Rust ecosystem added
  to Dependabot

### Documentation & site

- **edgeplane.ai launched** — custom landing page + full Starlight documentation
  site deployed via Cloudflare Pages
- Fixed 55 broken internal links (stale `/edgeplane/` path prefix)
- Standardized branding: Edgeplane → EdgePlane across all docs
- Rewrote auth documentation for the three active auth modes (OIDC, node JWT,
  service account)
- Fixed all stale CLI commands, socket paths, and env var references
- `curl -fsSL https://edgeplane.ai/install.sh | bash` install path with
  Cloudflare server-side redirect
- `MISSIONCONTROL_PHILOSOPHY.md` renamed to `PHILOSOPHY.md`

### Other changes

- Agent registration decoupled from Aria-specific conventions
- Source-agnostic agent listing and correct `StateDirSpec` serialization
- Additional test coverage for manifest import, idempotency, and unenroll

---

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
