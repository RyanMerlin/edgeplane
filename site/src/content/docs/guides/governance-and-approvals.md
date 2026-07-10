---
title: Domain Access Control
description: How EdgePlane controls read and write access to domains.
---

EdgePlane uses a lightweight ownership model for domain access control. There is no separate approval queue or governance policy engine.

## Authorization Model

Access is determined by two columns on the `domain` row:

- **`owners`** — comma-separated list of subject identifiers with full write access
- **`contributors`** — comma-separated list of subjects with create/modify access

Subjects listed in the `EP_ADMIN_EMAILS` environment variable on edgeplane-tower bypass both checks and have full access to all domains.

## Managing Domain Members

```sh
# List current owners and contributors
edgeplane domain show <domain-id>

# Owners and contributors are each a comma-separated list on the domain row —
# `update` replaces the full list, it does not append a single member.
edgeplane domain update <domain-id> --owners "alice@example.com,bob@example.com"
edgeplane domain update <domain-id> --contributors "carol@example.com"
```

## See Also

- [Domains, Missions & Tasks](/concepts/domains-missions-tasks/) — the entity hierarchy
- [Security Architecture](/architecture/security/) — full authorization model
