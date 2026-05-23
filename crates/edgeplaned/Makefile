.PHONY: e2e-phase3
e2e-phase3:
	bash tests/e2e/phase3_smoke.sh

# Pull the latest @zed-industries/agent-client-protocol schema into the
# edgeplaned-acp crate. Compares against the vendored copy and only updates
# if it changed; bumps the VERSION file to match the npm package version.
# Re-runs the crate's tests (codegen runs in build.rs, so any breaking
# schema change surfaces as a compile error).
.PHONY: sync-acp
sync-acp:
	@bash crates/edgeplaned-acp/scripts/sync.sh
