# Release Upgrade Checklist

## Purpose

Use this checklist for each release that includes schema, auth, or deployment changes.

## Cutting the Release (build + publish)

edgeplane / edgeplaned / edgeplane-tower ship in lockstep off a single git tag.

1. **Bump the version:** `scripts/set-version.sh <X.Y.Z>` (updates `/VERSION` + the root
   `Cargo.toml` `[workspace.package]` version), then `cargo metadata` to sync `Cargo.lock`.
   Add a `## [X.Y.Z]` section to `CHANGELOG.md`.
2. **PR → `main`, merge once green.** Commits need a `Signed-off-by` (DCO check), and
   `version-sync` asserts `/VERSION` == the `Cargo.toml` workspace version.
3. **Tag the merged commit `vX.Y.Z` (annotated) and push it.** The tag push fires:
   - `release-edgeplane.yml` → CLI + daemon binaries, the GitHub Release, and the
     `latest.json` self-update manifest.
   - `build-image.yml` → the `ghcr.io/ryanmerlin/edgeplane:X.Y.Z` tower image.

> **Watch the tower image / workspace drift.** `build-image.yml` runs only on main-push and
> tags, **not on PRs**. A new `[workspace]` member that isn't added to
> `crates/edgeplane-tower/Dockerfile` (it `COPY`s + stubs each member individually) therefore
> passes PR CI but breaks the tower image *after* merge / on the release tag — this is what sank
> the v0.13.0 image (`edgeplane-zrpc-proto`; fixed in v0.13.1). The `tower-dockerfile-guard` CI
> job (`scripts/check-tower-dockerfile.sh`) now enforces member parity on every PR, so this
> fails fast before merge.

## Pre-Release

1. Confirm migration state:
   - `cd crates/edgeplane-tower`
   - `sqlx migrate info` — confirm all migrations applied
2. Validate migration integrity locally:
   - `sqlx migrate run` — apply pending migrations
   - Start server and confirm `GET /health` returns 200
3. Run tests:
   - `cargo test -p edgeplane-tower`
   - `cargo test -p edgeplane-tui`
4. Validate docker profiles:
   - `bash scripts/smoke.sh --profile quickstart`
   - `bash scripts/smoke.sh --profile full`
5. Confirm auth config for target environment:
   - OIDC settings present for preferred auth path.
   - Admin identities set (`EP_ADMIN_EMAILS`).

## Release Execution

1. Backup DB snapshot in target environment.
2. Deploy application image.
3. Run schema migrations:
   - edgeplane-tower runs migrations automatically on startup
   - To run manually: `cd crates/edgeplane-tower && sqlx migrate run`
4. Verify API health:
   - `GET /`
   - `GET /schema-pack`

## Post-Release Validation

1. Authorization checks:
   - Owner/contributor/admin update paths.
   - Owner/admin delete paths.
2. Data checks:
   - Create and update mission/cluster/task.
   - Run search endpoints.
3. Publish checks (if enabled):
   - Pending ledger flow and publish operation.

## Rollback

1. If release must roll back:
   - Roll back application image.
   - Restore DB snapshot if migration is not backward-safe.
2. Record incident notes and migration constraints before next release.
