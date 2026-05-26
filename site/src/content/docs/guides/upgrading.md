---
title: Upgrading
description: Release upgrade checklist — schema migrations, validation, and rollback procedures.
---

Use this checklist for each release that includes schema, auth, or deployment changes.

## Pre-Release

### 1. Confirm migration state

```bash
cd crates/edgeplane-tower
sqlx migrate info
```

All migrations should show as applied on your current version.

### 2. Validate migrations locally

```bash
sqlx migrate run
# Start server and verify
curl http://localhost:8008/health
```

### 3. Run tests

```bash
cargo test -p edgeplane-tower
cargo test -p edgeplane-tui
```

### 4. Validate Docker profiles

```bash
bash scripts/smoke.sh --profile quickstart
bash scripts/smoke.sh --profile full
```

### 5. Confirm auth config

- OIDC settings are present and valid for the target environment
- Admin identities set (`EP_ADMIN_SUBJECTS` and/or `EP_ADMIN_EMAILS`)
- `EP_TOKEN` is set if MCP clients use static token auth

## Release Execution

1. **Take a DB snapshot** in the target environment before proceeding
2. **Deploy the new image** (or binary, depending on your deployment path)
3. **Run schema migrations:**
   - `edgeplane-tower` runs migrations automatically on startup
   - To run manually: `cd crates/edgeplane-tower && sqlx migrate run`
4. **Verify API health:**
   ```bash
   curl https://<edgeplane-host>/health
   curl https://<edgeplane-host>/schema-pack
   ```

## Post-Release Validation

### Authorization checks

- Owner/contributor update paths work as expected
- Owner/admin delete paths work as expected
- Non-admin subjects cannot call admin-only endpoints

### Data checks

```bash
# Verify post-upgrade connectivity and entity state
edgeplane status
# create + update a test mission via edgeplane or MCP tools
```

### Publish checks (if publication is enabled)

- Create a pending ledger event
- Approve it
- Verify Git commit is created and provenance written back to Postgres

## Rollback

If the release must be rolled back:

1. Roll back the application image (or binary)
2. If the migration is **not backward-safe**: restore the DB snapshot taken in step 1 of Release Execution
3. Document migration constraints in release notes before the next attempt

### Migration safety assessment

A migration is backward-safe if:
- It only adds columns (with defaults or nullable)
- It adds tables
- It adds indexes

A migration is **not** backward-safe if:
- It drops or renames columns
- It changes column types
- It drops tables
- It changes constraint behavior

When in doubt, treat the migration as not backward-safe and keep the DB snapshot.

## Schema Drift Detection

After any schema change, run:

```bash
# From the edgeplane-tower crate
cargo sqlx prepare
```

If this produces changes, commit them. Uncommitted schema drift causes CI failures on the SQLx offline check.

## See Also

- [Deployment](/guides/deployment/) — server setup and systemd/Compose configuration
- [OIDC Authentication](/guides/oidc/) — auth configuration
