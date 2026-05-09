# Release Upgrade Checklist

## Purpose

Use this checklist for each release that includes schema, auth, or deployment changes.

## Pre-Release

1. Confirm migration state:
   - `cd integrations/mc-controlplane`
   - `sqlx migrate info` — confirm all migrations applied
2. Validate migration integrity locally:
   - `sqlx migrate run` — apply pending migrations
   - Start server and confirm `GET /health` returns 200
3. Run tests:
   - `cargo test -p mc-controlplane`
   - `cargo test -p mc-tui`
4. Validate docker profiles:
   - `bash scripts/smoke.sh --profile quickstart`
   - `bash scripts/smoke.sh --profile full`
5. Confirm auth config for target environment:
   - OIDC settings present for preferred auth path.
   - Admin identities set (`MC_ADMIN_SUBJECTS` and/or `MC_ADMIN_EMAILS`).

## Release Execution

1. Backup DB snapshot in target environment.
2. Deploy application image.
3. Run schema migrations:
   - mc-controlplane runs migrations automatically on startup
   - To run manually: `cd integrations/mc-controlplane && sqlx migrate run`
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
