# Security Policy

## Supported Scope
Security reports are accepted for:
- Edgeplane backend and APIs
- MCP bridge/integration packages in this repository
- CI/release workflow security issues

## Reporting a Vulnerability
Please report privately and do not open a public issue for unpatched findings.

Contact:
- Preferred: GitHub Security Advisory (private report)
- Backup: security@merlinlabs.cloud

Include:
- Affected component/version/commit
- Reproduction steps or proof-of-concept
- Impact assessment and suggested mitigation (if known)

## Response Targets
- Initial acknowledgement: within 3 business days
- Triage decision: within 7 business days
- Remediation timeline: based on severity and exploitability

## Disclosure
We follow coordinated disclosure. Public disclosure happens after a fix is
available or mitigations are documented.

## Production deployment — change the defaults

The dev `docker-compose.*.yml` files ship with **insecure defaults** suitable
only for a developer's local machine:

- `POSTGRES_PASSWORD: edgeplane` (literal)
- `MQTT_PASSWORD: ""` (empty)
- CORS allow-list points at `localhost`

**Note:** The static `EP_TOKEN` shared-secret authentication path was removed in
edgeplane-tower 0.11.0. The tower now accepts only OIDC sessions and
`mcs_sa_*` service-account tokens. Any `EP_TOKEN: dev-token` lines remaining in
legacy compose files are ignored by the server; remove them to avoid confusion.

Before exposing any deployment outside a personal workstation, you MUST:

1. Replace every literal credential with a value sourced from a secret manager
   (Infisical, Vault, sealed secrets, etc.) — never commit the production
   values.
2. Configure OIDC (recommended) or issue service-account tokens (`mcs_sa_*`) for
   programmatic access. Steady-state callers should use session tokens (`mcs_*`)
   or service-account tokens; the static `EP_TOKEN` mechanism was removed in
   0.11.0.
3. Configure `EP_CORS_ALLOW_ORIGINS` to your real frontend origin(s).
4. Run behind TLS (reverse proxy or otherwise; do NOT set
   `EP_ALLOW_INSECURE=true`).
5. Rotate any credential that has appeared in a screen-share, log, or
   chat — treat it as compromised. The dev defaults above are public; a
   deployment reachable from the internet that uses them is open by
   construction.
