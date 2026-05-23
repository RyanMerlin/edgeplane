# Governance

## Project Stewardship
Edgeplane is stewarded by Ryan Merlin with maintainers delegated per area
(API, MCP integration, docs, release).

## Decision Model
- Day-to-day technical decisions: area maintainers via PR review.
- Breaking changes, governance policy, and release policy: maintainer consensus
  with steward tie-break.

## Roles
- Steward: project direction, final arbitration, trademark stewardship.
- Maintainers: review/merge responsibility and release operations.
- Contributors: code/docs/tests via PR.

## Merge Policy
- At least one maintainer approval required.
- CI checks must pass.
- DCO sign-off required.

## Release Policy
- Use tagged releases with release notes.
- Security and critical reliability fixes may be released out-of-band.

## Compatibility
Public APIs and MCP tool contracts should prefer additive changes. Breaking
changes require migration notes and release-callout.
