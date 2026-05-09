# Release Upgrade Checklist

## Purpose

Use this checklist for each release that includes schema, auth, or deployment changes.

## Pre-Release

1. Confirm migration state:
   - `cd backend`
   - `alembic current`
   - `alembic heads`
2. Validate migration integrity locally:
   - `alembic upgrade head`
   - `alembic check`
   - `alembic downgrade base && alembic upgrade head`
3. Run tests:
   - `python -m unittest discover -s tests -p "test_*.py"`
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
   - `cd backend`
   - `alembic upgrade head`
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
