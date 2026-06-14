# Command-Line Help for `edgeplane`

This document contains the help content for the `edgeplane` command-line program.

**Command Overview:**

* [`edgeplane`↴](#edgeplane)
* [`edgeplane status`↴](#edgeplane-status)
* [`edgeplane doctor`↴](#edgeplane-doctor)
* [`edgeplane health`↴](#edgeplane-health)
* [`edgeplane version`↴](#edgeplane-version)
* [`edgeplane config`↴](#edgeplane-config)
* [`edgeplane use`↴](#edgeplane-use)
* [`edgeplane release`↴](#edgeplane-release)
* [`edgeplane logs`↴](#edgeplane-logs)
* [`edgeplane completion`↴](#edgeplane-completion)
* [`edgeplane artifact`↴](#edgeplane-artifact)
* [`edgeplane artifact inspect`↴](#edgeplane-artifact-inspect)
* [`edgeplane artifact view`↴](#edgeplane-artifact-view)
* [`edgeplane artifact edit`↴](#edgeplane-artifact-edit)
* [`edgeplane artifact replace`↴](#edgeplane-artifact-replace)
* [`edgeplane auth`↴](#edgeplane-auth)
* [`edgeplane auth login`↴](#edgeplane-auth-login)
* [`edgeplane auth logout`↴](#edgeplane-auth-logout)
* [`edgeplane auth whoami`↴](#edgeplane-auth-whoami)
* [`edgeplane admin`↴](#edgeplane-admin)
* [`edgeplane admin policy`↴](#edgeplane-admin-policy)
* [`edgeplane admin policy active`↴](#edgeplane-admin-policy-active)
* [`edgeplane admin policy versions`↴](#edgeplane-admin-policy-versions)
* [`edgeplane admin policy events`↴](#edgeplane-admin-policy-events)
* [`edgeplane admin governance`↴](#edgeplane-admin-governance)
* [`edgeplane admin governance roles`↴](#edgeplane-admin-governance-roles)
* [`edgeplane admin governance roles list`↴](#edgeplane-admin-governance-roles-list)
* [`edgeplane admin governance roles upsert`↴](#edgeplane-admin-governance-roles-upsert)
* [`edgeplane admin governance roles remove`↴](#edgeplane-admin-governance-roles-remove)
* [`edgeplane admin governance policy`↴](#edgeplane-admin-governance-policy)
* [`edgeplane admin governance policy active`↴](#edgeplane-admin-governance-policy-active)
* [`edgeplane admin governance policy versions`↴](#edgeplane-admin-governance-policy-versions)
* [`edgeplane admin governance policy create-draft`↴](#edgeplane-admin-governance-policy-create-draft)
* [`edgeplane admin governance policy publish`↴](#edgeplane-admin-governance-policy-publish)
* [`edgeplane admin governance policy rollback`↴](#edgeplane-admin-governance-policy-rollback)
* [`edgeplane admin governance policy reload`↴](#edgeplane-admin-governance-policy-reload)
* [`edgeplane admin governance events`↴](#edgeplane-admin-governance-events)
* [`edgeplane data`↴](#edgeplane-data)
* [`edgeplane data tools`↴](#edgeplane-data-tools)
* [`edgeplane data tools list`↴](#edgeplane-data-tools-list)
* [`edgeplane data tools call`↴](#edgeplane-data-tools-call)
* [`edgeplane data explorer`↴](#edgeplane-data-explorer)
* [`edgeplane data explorer tree`↴](#edgeplane-data-explorer-tree)
* [`edgeplane data explorer node`↴](#edgeplane-data-explorer-node)
* [`edgeplane system`↴](#edgeplane-system)
* [`edgeplane system doctor`↴](#edgeplane-system-doctor)
* [`edgeplane system backup`↴](#edgeplane-system-backup)
* [`edgeplane system profile-gc`↴](#edgeplane-system-profile-gc)
* [`edgeplane system update`↴](#edgeplane-system-update)
* [`edgeplane system update self-update`↴](#edgeplane-system-update-self-update)
* [`edgeplane system compat`↴](#edgeplane-system-compat)
* [`edgeplane system compat matrix`↴](#edgeplane-system-compat-matrix)
* [`edgeplane system compat matrix run`↴](#edgeplane-system-compat-matrix-run)
* [`edgeplane system compat matrix report-latest`↴](#edgeplane-system-compat-matrix-report-latest)
* [`edgeplane system drift`↴](#edgeplane-system-drift)
* [`edgeplane system drift ingest`↴](#edgeplane-system-drift-ingest)
* [`edgeplane system drift triage`↴](#edgeplane-system-drift-triage)
* [`edgeplane agent`↴](#edgeplane-agent)
* [`edgeplane agent signal`↴](#edgeplane-agent-signal)
* [`edgeplane agent cancel`↴](#edgeplane-agent-cancel)
* [`edgeplane agent list`↴](#edgeplane-agent-list)
* [`edgeplane agent describe`↴](#edgeplane-agent-describe)
* [`edgeplane agent node`↴](#edgeplane-agent-node)
* [`edgeplane agent node register`↴](#edgeplane-agent-node-register)
* [`edgeplane agent node run`↴](#edgeplane-agent-node-run)
* [`edgeplane agent node doctor`↴](#edgeplane-agent-node-doctor)
* [`edgeplane agent node join-token`↴](#edgeplane-agent-node-join-token)
* [`edgeplane agent node join-token create`↴](#edgeplane-agent-node-join-token-create)
* [`edgeplane agent node join-token get`↴](#edgeplane-agent-node-join-token-get)
* [`edgeplane agent node join-token rotate`↴](#edgeplane-agent-node-join-token-rotate)
* [`edgeplane agent attach`↴](#edgeplane-agent-attach)
* [`edgeplane agent cron`↴](#edgeplane-agent-cron)
* [`edgeplane agent cron list`↴](#edgeplane-agent-cron-list)
* [`edgeplane agent cron describe`↴](#edgeplane-agent-cron-describe)
* [`edgeplane agent cron reload`↴](#edgeplane-agent-cron-reload)
* [`edgeplane agent cron history`↴](#edgeplane-agent-cron-history)
* [`edgeplane agent cron gc-now`↴](#edgeplane-agent-cron-gc-now)
* [`edgeplane agent supervise`↴](#edgeplane-agent-supervise)
* [`edgeplane agent supervise list`↴](#edgeplane-agent-supervise-list)
* [`edgeplane agent supervise status`↴](#edgeplane-agent-supervise-status)
* [`edgeplane agent supervise restart`↴](#edgeplane-agent-supervise-restart)
* [`edgeplane agent supervise pause`↴](#edgeplane-agent-supervise-pause)
* [`edgeplane agent supervise resume`↴](#edgeplane-agent-supervise-resume)
* [`edgeplane agent supervise history`↴](#edgeplane-agent-supervise-history)
* [`edgeplane agent supervise events`↴](#edgeplane-agent-supervise-events)
* [`edgeplane agent supervise watch`↴](#edgeplane-agent-supervise-watch)
* [`edgeplane agent register`↴](#edgeplane-agent-register)
* [`edgeplane agent set-status`↴](#edgeplane-agent-set-status)
* [`edgeplane agent delete`↴](#edgeplane-agent-delete)
* [`edgeplane runtime`↴](#edgeplane-runtime)
* [`edgeplane runtime nodes`↴](#edgeplane-runtime-nodes)
* [`edgeplane runtime nodes register`↴](#edgeplane-runtime-nodes-register)
* [`edgeplane runtime nodes list`↴](#edgeplane-runtime-nodes-list)
* [`edgeplane runtime nodes heartbeat`↴](#edgeplane-runtime-nodes-heartbeat)
* [`edgeplane runtime jobs`↴](#edgeplane-runtime-jobs)
* [`edgeplane runtime jobs submit`↴](#edgeplane-runtime-jobs-submit)
* [`edgeplane runtime jobs list`↴](#edgeplane-runtime-jobs-list)
* [`edgeplane runtime leases`↴](#edgeplane-runtime-leases)
* [`edgeplane runtime leases create`↴](#edgeplane-runtime-leases-create)
* [`edgeplane runtime leases status`↴](#edgeplane-runtime-leases-status)
* [`edgeplane runtime leases complete`↴](#edgeplane-runtime-leases-complete)
* [`edgeplane runtime sessions`↴](#edgeplane-runtime-sessions)
* [`edgeplane runtime sessions attach`↴](#edgeplane-runtime-sessions-attach)
* [`edgeplane approvals`↴](#edgeplane-approvals)
* [`edgeplane approvals create`↴](#edgeplane-approvals-create)
* [`edgeplane approvals list`↴](#edgeplane-approvals-list)
* [`edgeplane approvals approve`↴](#edgeplane-approvals-approve)
* [`edgeplane approvals reject`↴](#edgeplane-approvals-reject)
* [`edgeplane workspace`↴](#edgeplane-workspace)
* [`edgeplane workspace load`↴](#edgeplane-workspace-load)
* [`edgeplane workspace heartbeat`↴](#edgeplane-workspace-heartbeat)
* [`edgeplane workspace fetch-artifact`↴](#edgeplane-workspace-fetch-artifact)
* [`edgeplane workspace commit`↴](#edgeplane-workspace-commit)
* [`edgeplane workspace release`↴](#edgeplane-workspace-release)
* [`edgeplane ops`↴](#edgeplane-ops)
* [`edgeplane ops domain`↴](#edgeplane-ops-domain)
* [`edgeplane update`↴](#edgeplane-update)
* [`edgeplane init`↴](#edgeplane-init)
* [`edgeplane serve`↴](#edgeplane-serve)
* [`edgeplane channel`↴](#edgeplane-channel)
* [`edgeplane channel claude`↴](#edgeplane-channel-claude)
* [`edgeplane channel claude webhook`↴](#edgeplane-channel-claude-webhook)
* [`edgeplane profile`↴](#edgeplane-profile)
* [`edgeplane profile create`↴](#edgeplane-profile-create)
* [`edgeplane profile list`↴](#edgeplane-profile-list)
* [`edgeplane profile show`↴](#edgeplane-profile-show)
* [`edgeplane profile activate`↴](#edgeplane-profile-activate)
* [`edgeplane profile download`↴](#edgeplane-profile-download)
* [`edgeplane profile publish`↴](#edgeplane-profile-publish)
* [`edgeplane profile pull`↴](#edgeplane-profile-pull)
* [`edgeplane profile pin`↴](#edgeplane-profile-pin)
* [`edgeplane profile delete`↴](#edgeplane-profile-delete)
* [`edgeplane profile status`↴](#edgeplane-profile-status)
* [`edgeplane profile use`↴](#edgeplane-profile-use)
* [`edgeplane secrets`↴](#edgeplane-secrets)
* [`edgeplane secrets status`↴](#edgeplane-secrets-status)
* [`edgeplane secrets provider`↴](#edgeplane-secrets-provider)
* [`edgeplane secrets provider env`↴](#edgeplane-secrets-provider-env)
* [`edgeplane secrets provider infisical`↴](#edgeplane-secrets-provider-infisical)
* [`edgeplane secrets get`↴](#edgeplane-secrets-get)
* [`edgeplane secrets bootstrap`↴](#edgeplane-secrets-bootstrap)
* [`edgeplane secrets rotate`↴](#edgeplane-secrets-rotate)
* [`edgeplane secrets export-env`↴](#edgeplane-secrets-export-env)
* [`edgeplane secrets infisical`↴](#edgeplane-secrets-infisical)
* [`edgeplane secrets infisical add`↴](#edgeplane-secrets-infisical-add)
* [`edgeplane secrets infisical list`↴](#edgeplane-secrets-infisical-list)
* [`edgeplane secrets infisical use`↴](#edgeplane-secrets-infisical-use)
* [`edgeplane secrets infisical test`↴](#edgeplane-secrets-infisical-test)
* [`edgeplane secrets infisical rm`↴](#edgeplane-secrets-infisical-rm)
* [`edgeplane secrets infisical get`↴](#edgeplane-secrets-infisical-get)
* [`edgeplane daemon`↴](#edgeplane-daemon)
* [`edgeplane daemon up`↴](#edgeplane-daemon-up)
* [`edgeplane daemon down`↴](#edgeplane-daemon-down)
* [`edgeplane daemon uninstall`↴](#edgeplane-daemon-uninstall)
* [`edgeplane daemon status`↴](#edgeplane-daemon-status)
* [`edgeplane daemon health`↴](#edgeplane-daemon-health)
* [`edgeplane daemon upgrade`↴](#edgeplane-daemon-upgrade)
* [`edgeplane daemon version`↴](#edgeplane-daemon-version)
* [`edgeplane daemon runtime`↴](#edgeplane-daemon-runtime)
* [`edgeplane daemon runtime ls`↴](#edgeplane-daemon-runtime-ls)
* [`edgeplane daemon runtime install`↴](#edgeplane-daemon-runtime-install)
* [`edgeplane daemon runtime test`↴](#edgeplane-daemon-runtime-test)
* [`edgeplane daemon agent`↴](#edgeplane-daemon-agent)
* [`edgeplane daemon agent ls`↴](#edgeplane-daemon-agent-ls)
* [`edgeplane daemon agent enroll`↴](#edgeplane-daemon-agent-enroll)
* [`edgeplane daemon agent enroll-home`↴](#edgeplane-daemon-agent-enroll-home)
* [`edgeplane daemon agent import`↴](#edgeplane-daemon-agent-import)
* [`edgeplane daemon agent reassign`↴](#edgeplane-daemon-agent-reassign)
* [`edgeplane daemon agent unenroll`↴](#edgeplane-daemon-agent-unenroll)
* [`edgeplane daemon agent attach`↴](#edgeplane-daemon-agent-attach)
* [`edgeplane daemon agent profile`↴](#edgeplane-daemon-agent-profile)
* [`edgeplane daemon mission`↴](#edgeplane-daemon-mission)
* [`edgeplane daemon mission ls`↴](#edgeplane-daemon-mission-ls)
* [`edgeplane daemon mission show`↴](#edgeplane-daemon-mission-show)
* [`edgeplane daemon mission watch`↴](#edgeplane-daemon-mission-watch)
* [`edgeplane daemon task`↴](#edgeplane-daemon-task)
* [`edgeplane daemon task run`↴](#edgeplane-daemon-task-run)
* [`edgeplane daemon task ls`↴](#edgeplane-daemon-task-ls)
* [`edgeplane daemon task show`↴](#edgeplane-daemon-task-show)
* [`edgeplane daemon task watch`↴](#edgeplane-daemon-task-watch)
* [`edgeplane daemon task attach`↴](#edgeplane-daemon-task-attach)
* [`edgeplane daemon task cancel`↴](#edgeplane-daemon-task-cancel)
* [`edgeplane daemon task retry`↴](#edgeplane-daemon-task-retry)
* [`edgeplane daemon msg`↴](#edgeplane-daemon-msg)
* [`edgeplane daemon msg send`↴](#edgeplane-daemon-msg-send)
* [`edgeplane daemon msg tail`↴](#edgeplane-daemon-msg-tail)
* [`edgeplane daemon attach`↴](#edgeplane-daemon-attach)
* [`edgeplane daemon watch`↴](#edgeplane-daemon-watch)
* [`edgeplane daemon profile`↴](#edgeplane-daemon-profile)
* [`edgeplane daemon profile add`↴](#edgeplane-daemon-profile-add)
* [`edgeplane daemon profile list`↴](#edgeplane-daemon-profile-list)
* [`edgeplane daemon profile remove`↴](#edgeplane-daemon-profile-remove)
* [`edgeplane daemon profile rename`↴](#edgeplane-daemon-profile-rename)
* [`edgeplane daemon profile show`↴](#edgeplane-daemon-profile-show)
* [`edgeplane daemon use`↴](#edgeplane-daemon-use)
* [`edgeplane run`↴](#edgeplane-run)
* [`edgeplane capabilities`↴](#edgeplane-capabilities)
* [`edgeplane capabilities list`↴](#edgeplane-capabilities-list)
* [`edgeplane capabilities describe`↴](#edgeplane-capabilities-describe)
* [`edgeplane exec`↴](#edgeplane-exec)
* [`edgeplane receipts`↴](#edgeplane-receipts)
* [`edgeplane receipts last`↴](#edgeplane-receipts-last)
* [`edgeplane receipts get`↴](#edgeplane-receipts-get)
* [`edgeplane receipts ls`↴](#edgeplane-receipts-ls)
* [`edgeplane mesh-sync`↴](#edgeplane-mesh-sync)
* [`edgeplane mesh-sync pull`↴](#edgeplane-mesh-sync-pull)
* [`edgeplane mesh-sync status`↴](#edgeplane-mesh-sync-status)
* [`edgeplane mesh-sync push`↴](#edgeplane-mesh-sync-push)
* [`edgeplane tui`↴](#edgeplane-tui)
* [`edgeplane context`↴](#edgeplane-context)
* [`edgeplane context list`↴](#edgeplane-context-list)
* [`edgeplane context current`↴](#edgeplane-context-current)
* [`edgeplane context use`↴](#edgeplane-context-use)
* [`edgeplane context add`↴](#edgeplane-context-add)
* [`edgeplane context remove`↴](#edgeplane-context-remove)
* [`edgeplane context discover`↴](#edgeplane-context-discover)
* [`edgeplane domain`↴](#edgeplane-domain)
* [`edgeplane domain home`↴](#edgeplane-domain-home)
* [`edgeplane domain attach`↴](#edgeplane-domain-attach)
* [`edgeplane domain detach`↴](#edgeplane-domain-detach)
* [`edgeplane domain create`↴](#edgeplane-domain-create)
* [`edgeplane domain list`↴](#edgeplane-domain-list)
* [`edgeplane domain show`↴](#edgeplane-domain-show)
* [`edgeplane domain update`↴](#edgeplane-domain-update)
* [`edgeplane domain delete`↴](#edgeplane-domain-delete)
* [`edgeplane domain northstar`↴](#edgeplane-domain-northstar)
* [`edgeplane domain northstar get`↴](#edgeplane-domain-northstar-get)
* [`edgeplane domain northstar edit`↴](#edgeplane-domain-northstar-edit)
* [`edgeplane mission`↴](#edgeplane-mission)
* [`edgeplane mission create`↴](#edgeplane-mission-create)
* [`edgeplane mission list`↴](#edgeplane-mission-list)
* [`edgeplane mission show`↴](#edgeplane-mission-show)
* [`edgeplane mission update`↴](#edgeplane-mission-update)
* [`edgeplane mission delete`↴](#edgeplane-mission-delete)
* [`edgeplane mission brief`↴](#edgeplane-mission-brief)
* [`edgeplane mission brief get`↴](#edgeplane-mission-brief-get)
* [`edgeplane mission brief edit`↴](#edgeplane-mission-brief-edit)
* [`edgeplane task`↴](#edgeplane-task)
* [`edgeplane task create`↴](#edgeplane-task-create)
* [`edgeplane task list`↴](#edgeplane-task-list)
* [`edgeplane task show`↴](#edgeplane-task-show)
* [`edgeplane task update`↴](#edgeplane-task-update)
* [`edgeplane task delete`↴](#edgeplane-task-delete)
* [`edgeplane discover`↴](#edgeplane-discover)

## `edgeplane`

EdgePlane — fleet control-plane CLI

**Usage:** `edgeplane [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `status` — Show quick local/runtime/auth context for the current shell
* `doctor` — Shortcut for `edgeplane system doctor`
* `health` — Lightweight backend readiness check
* `version` — Show binary + backend version details
* `config` — Show effective runtime config (redacted)
* `use` — Convenience context/profile switcher
* `release` — Release the currently attached workspace lease
* `logs` — Tail local Edgeplane logs
* `completion` — Generate shell completion scripts
* `artifact` — Artifact retrieval and mutation helpers
* `auth` — Authentication and identity helpers
* `admin` — Governance and admin workflows
* `data` — Data/catalog/read workflows (tools, sync, explorer)
* `system` — Platform diagnostics and release-control workflows
* `agent` — Agent control workflows (remote, swarm/subagent workflows)
* `runtime` — Runtime fabric workflows (nodes, jobs, leases)
* `approvals` — Approval workflow commands (requests, decisions)
* `workspace` — Workspace lifecycle helpers (load/heartbeat/artifact/commit/release)
* `ops` — Domain operations (lifecycle orchestration and execution workflows)
* `update` — Self-update helper for the edgeplane binary
* `init` — Initialize EdgePlane profile state for first-time usage
* `serve` — Start an MCP server (stdio JSON-RPC 2.0) for LLM runtime connections
* `channel` — Claude channel server integrations
* `profile` — Manage Edgeplane user profiles
* `secrets` — Secrets provider + reference helpers
* `daemon` — edgeplaned daemon control and work-model commands
* `run` — Launch and manage an agent runtime: claude, codex, gemini, goose, openclaw, custom
* `capabilities` — List and describe capability packs available through edgeplaned
* `exec` — Execute a capability
* `receipts` — Inspect capability execution receipts stored in the local SQLite audit log
* `mesh-sync` — Bidirectional git-backed config sync for this node
* `tui` — Launch the terminal UI (ratatui) for fleet monitoring and management
* `context` — Manage named controlplane connection contexts
* `domain` — Domain attachment and home-domain management for this agent
* `mission` — Mission (workstream) CRUD — create, list, show, update, delete
* `task` — Task CRUD — create, list, show, update, delete
* `discover` — Emit the CLI surface as a versioned JSON schema contract; drill into a subtree with [path...]

###### **Options:**

* `--base-url <BASE_URL>` — Base URL pointing at an existing Edgeplane deployment
* `--agent-id <AGENT_ID>` — Optional agent identifier propagated throughout approvals and sync calls
* `--runtime-session-id <RUNTIME_SESSION_ID>` — Optional runtime session identifier propagated for per-instance attribution
* `--profile-name <PROFILE_NAME>` — Optional profile name propagated for per-profile attribution
* `--timeout-secs <TIMEOUT_SECS>` — Timeout (in seconds) for all outbound calls

  Default value: `10`
* `--allow-insecure` — Allow invalid TLS certificates when running against local or self-signed endpoints

  Default value: `false`
* `--booster-wasm <BOOSTER_WASM>` — Optional WASM booster module path
* `--disable-booster` — Disable the booster hook even if a module is configured

  Default value: `false`
* `--allow-booster-short-circuit` — Allow booster modules to short-circuit MCP tool execution

  Default value: `false`
* `--json` — Emit machine-readable JSON output

  Default value: `false`



## `edgeplane status`

Show quick local/runtime/auth context for the current shell

**Usage:** `edgeplane status [OPTIONS]`

###### **Options:**

* `--verify-lease` — Validate active lease by sending a heartbeat call

  Default value: `false`



## `edgeplane doctor`

Shortcut for `edgeplane system doctor`

**Usage:** `edgeplane doctor [OPTIONS]`

###### **Options:**

* `--fix`

  Default value: `false`
* `--cleanup` — Also cleanup local profile/session artifacts after checks

  Default value: `false`
* `--cleanup-keep-instances <CLEANUP_KEEP_INSTANCES>` — When --cleanup is set, keep at most this many runtime instance dirs

  Default value: `8`
* `--cleanup-keep-bundles <CLEANUP_KEEP_BUNDLES>` — When --cleanup is set, keep at most this many bundle tar files per profile

  Default value: `6`
* `--cleanup-max-age-days <CLEANUP_MAX_AGE_DAYS>` — When --cleanup is set, remove instance dirs older than this many days

  Default value: `7`



## `edgeplane health`

Lightweight backend readiness check

**Usage:** `edgeplane health`



## `edgeplane version`

Show binary + backend version details

**Usage:** `edgeplane version`



## `edgeplane config`

Show effective runtime config (redacted)

**Usage:** `edgeplane config`



## `edgeplane use`

Convenience context/profile switcher

**Usage:** `edgeplane use [OPTIONS]`

###### **Options:**

* `--profile <PROFILE>`
* `--mission-id <MISSION_ID>`
* `--lease-seconds <LEASE_SECONDS>`

  Default value: `900`
* `--workspace-label <WORKSPACE_LABEL>`
* `--auto-release` — Auto-release existing lease when switching missions

  Default value: `false`
* `-y`, `--yes` — Non-interactive confirmation for releasing/switching

  Default value: `false`
* `--release` — Release currently attached lease instead of attaching a mission

  Default value: `false`



## `edgeplane release`

Release the currently attached workspace lease

**Usage:** `edgeplane release [OPTIONS]`

###### **Options:**

* `--reason <REASON>` — Optional reason recorded in lease release metadata
* `--ignore-missing` — Succeed even when no active lease is tracked

  Default value: `false`



## `edgeplane logs`

Tail local Edgeplane logs

**Usage:** `edgeplane logs [OPTIONS]`

###### **Options:**

* `--lines <LINES>`

  Default value: `120`



## `edgeplane completion`

Generate shell completion scripts

**Usage:** `edgeplane completion <SHELL>`

###### **Arguments:**

* `<SHELL>`

  Possible values: `bash`, `elvish`, `fish`, `powershell`, `zsh`




## `edgeplane artifact`

Artifact retrieval and mutation helpers

**Usage:** `edgeplane artifact <COMMAND>`

###### **Subcommands:**

* `inspect` — Show artifact metadata
* `view` — Retrieve artifact bytes for validation/view
* `edit` — Edit a text artifact in your local editor, then save back
* `replace` — Replace artifact bytes from a local file



## `edgeplane artifact inspect`

Show artifact metadata

**Usage:** `edgeplane artifact inspect --id <ID>`

###### **Options:**

* `--id <ID>`



## `edgeplane artifact view`

Retrieve artifact bytes for validation/view

**Usage:** `edgeplane artifact view [OPTIONS] --id <ID>`

###### **Options:**

* `--id <ID>`
* `--lease-id <LEASE_ID>` — Optional active lease for workspace-scoped retrieval
* `--out <OUT>` — Write bytes to local path instead of printing text



## `edgeplane artifact edit`

Edit a text artifact in your local editor, then save back

**Usage:** `edgeplane artifact edit [OPTIONS] --id <ID>`

###### **Options:**

* `--id <ID>`
* `--lease-id <LEASE_ID>` — Optional active lease for workspace-scoped authorization check
* `--mime <MIME>`
* `-y`, `--yes` — Confirm cross-mission mutation without explicit --lease-id

  Default value: `false`



## `edgeplane artifact replace`

Replace artifact bytes from a local file

**Usage:** `edgeplane artifact replace [OPTIONS] --id <ID> --from-file <FROM_FILE>`

###### **Options:**

* `--id <ID>`
* `--from-file <FROM_FILE>`
* `--lease-id <LEASE_ID>` — Optional active lease for workspace-scoped mutation
* `--mime <MIME>`
* `-y`, `--yes` — Confirm cross-mission mutation without explicit --lease-id

  Default value: `false`



## `edgeplane auth`

Authentication and identity helpers

**Usage:** `edgeplane auth <COMMAND>`

###### **Subcommands:**

* `login` — Authenticate and create a session token stored at ~/.edgeplane/session.json
* `logout` — Revoke the current session token and clear local credentials
* `whoami` — Show the current authenticated identity



## `edgeplane auth login`

Authenticate and create a session token stored at ~/.edgeplane/session.json

**Usage:** `edgeplane auth login [OPTIONS]`

###### **Options:**

* `--ttl-hours <TTL_HOURS>` — Session TTL in hours (default: 8, max: 8760)

  Default value: `8`
* `--print-token` — Print the session token to stdout after login (useful in scripts)
* `--non-interactive` — Skip prompts: use EP_AGENT_TOKEN env var directly (non-interactive)
* `--with-token` — Use API token auth instead of OIDC (prompts for token interactively)



## `edgeplane auth logout`

Revoke the current session token and clear local credentials

**Usage:** `edgeplane auth logout [OPTIONS]`

###### **Options:**

* `--local-only` — Only clear the local session file; do not call the revoke endpoint



## `edgeplane auth whoami`

Show the current authenticated identity

**Usage:** `edgeplane auth whoami`



## `edgeplane admin`

Governance and admin workflows

**Usage:** `edgeplane admin <COMMAND>`

###### **Subcommands:**

* `policy` — Governance policy summaries and event feeds
* `governance` — Governance automation helpers (roles, policies, events)



## `edgeplane admin policy`

Governance policy summaries and event feeds

**Usage:** `edgeplane admin policy <COMMAND>`

###### **Subcommands:**

* `active` — Show the currently active governance policy
* `versions` — List previous policy versions (limit defaults to 50)
* `events` — Show the recent policy events emitted from approvals



## `edgeplane admin policy active`

Show the currently active governance policy

**Usage:** `edgeplane admin policy active`



## `edgeplane admin policy versions`

List previous policy versions (limit defaults to 50)

**Usage:** `edgeplane admin policy versions [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`



## `edgeplane admin policy events`

Show the recent policy events emitted from approvals

**Usage:** `edgeplane admin policy events [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`



## `edgeplane admin governance`

Governance automation helpers (roles, policies, events)

**Usage:** `edgeplane admin governance <COMMAND>`

###### **Subcommands:**

* `roles` — Manage domain-level roles and memberships
* `policy` — Work with governance policies
* `events` — Inspect governance policy events



## `edgeplane admin governance roles`

Manage domain-level roles and memberships

**Usage:** `edgeplane admin governance roles <COMMAND>`

###### **Subcommands:**

* `list` — List role assignments for a domain
* `upsert` — Add or update a role
* `remove` — Remove a role assignment



## `edgeplane admin governance roles list`

List role assignments for a domain

**Usage:** `edgeplane admin governance roles list [OPTIONS] --domain-id <DOMAIN_ID>`

###### **Options:**

* `--domain-id <DOMAIN_ID>`
* `--limit <LIMIT>`

  Default value: `50`



## `edgeplane admin governance roles upsert`

Add or update a role

**Usage:** `edgeplane admin governance roles upsert --domain-id <DOMAIN_ID> --subject <SUBJECT> --role <ROLE>`

###### **Options:**

* `--domain-id <DOMAIN_ID>`
* `--subject <SUBJECT>`
* `--role <ROLE>`



## `edgeplane admin governance roles remove`

Remove a role assignment

**Usage:** `edgeplane admin governance roles remove --domain-id <DOMAIN_ID> --subject <SUBJECT>`

###### **Options:**

* `--domain-id <DOMAIN_ID>`
* `--subject <SUBJECT>`



## `edgeplane admin governance policy`

Work with governance policies

**Usage:** `edgeplane admin governance policy <COMMAND>`

###### **Subcommands:**

* `active` — Show the active governance policy
* `versions` — List historical policy versions
* `create-draft` — Create a new draft from JSON file
* `publish` — Publish an existing draft
* `rollback` — Roll back to a specific version
* `reload` — Reload the active policy config



## `edgeplane admin governance policy active`

Show the active governance policy

**Usage:** `edgeplane admin governance policy active`



## `edgeplane admin governance policy versions`

List historical policy versions

**Usage:** `edgeplane admin governance policy versions [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`

  Default value: `20`



## `edgeplane admin governance policy create-draft`

Create a new draft from JSON file

**Usage:** `edgeplane admin governance policy create-draft [OPTIONS] --file <FILE>`

###### **Options:**

* `--file <FILE>`
* `--change-note <CHANGE_NOTE>`



## `edgeplane admin governance policy publish`

Publish an existing draft

**Usage:** `edgeplane admin governance policy publish [OPTIONS] --draft-id <DRAFT_ID>`

###### **Options:**

* `--draft-id <DRAFT_ID>`
* `--change-note <CHANGE_NOTE>`



## `edgeplane admin governance policy rollback`

Roll back to a specific version

**Usage:** `edgeplane admin governance policy rollback [OPTIONS] --version <VERSION>`

###### **Options:**

* `--version <VERSION>`
* `--change-note <CHANGE_NOTE>`



## `edgeplane admin governance policy reload`

Reload the active policy config

**Usage:** `edgeplane admin governance policy reload`



## `edgeplane admin governance events`

Inspect governance policy events

**Usage:** `edgeplane admin governance events [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`

  Default value: `50`



## `edgeplane data`

Data/catalog/read workflows (tools, sync, explorer)

**Usage:** `edgeplane data <COMMAND>`

###### **Subcommands:**

* `tools` — Inspect and invoke Edgeplane MCP tools
* `explorer` — Explore domains, missions, and tasks via the explorer endpoints



## `edgeplane data tools`

Inspect and invoke Edgeplane MCP tools

**Usage:** `edgeplane data tools <COMMAND>`

###### **Subcommands:**

* `list` — List all registered MCP tools
* `call` — Call an MCP tool with JSON payload and show the response



## `edgeplane data tools list`

List all registered MCP tools

**Usage:** `edgeplane data tools list`



## `edgeplane data tools call`

Call an MCP tool with JSON payload and show the response

**Usage:** `edgeplane data tools call [OPTIONS] --tool <TOOL>`

###### **Options:**

* `-t`, `--tool <TOOL>` — Name of the MCP tool to call (e.g. edgeplane.mission.load)
* `--payload <PAYLOAD>` — JSON payload to send as MCP tool args. Defaults to empty object

  Default value: `{}`



## `edgeplane data explorer`

Explore domains, missions, and tasks via the explorer endpoints

**Usage:** `edgeplane data explorer <COMMAND>`

###### **Subcommands:**

* `tree` — Dump the tree-view of domains, missions, and recent tasks
* `node` — Inspect a single domain/mission/task node



## `edgeplane data explorer tree`

Dump the tree-view of domains, missions, and recent tasks

**Usage:** `edgeplane data explorer tree [OPTIONS]`

###### **Options:**

* `--domain-id <DOMAIN_ID>`
* `--status <STATUS>`
* `--q <Q>`
* `--limit-tasks-per-cluster <LIMIT_TASKS_PER_CLUSTER>`
* `--limit-missions <LIMIT_MISSIONS>`



## `edgeplane data explorer node`

Inspect a single domain/mission/task node

**Usage:** `edgeplane data explorer node --node-type <NODE_TYPE> --node-id <NODE_ID>`

###### **Options:**

* `--node-type <NODE_TYPE>`

  Possible values: `domain`, `mission`, `task`

* `--node-id <NODE_ID>`



## `edgeplane system`

Platform diagnostics and release-control workflows

**Usage:** `edgeplane system <COMMAND>`

###### **Subcommands:**

* `doctor` — Diagnostics + auto-fix helpers
* `backup` — Trigger local backups (postgres, rustfs, or both)
* `profile-gc` — Cleanup local profile/session artifacts with retention limits
* `update` — Self-update helper for the edgeplane binary
* `compat` — Compatibility matrix commands and reports for provider/version drift control
* `drift` — Drift ingestion + policy decision helpers for staged release gates



## `edgeplane system doctor`

Diagnostics + auto-fix helpers

**Usage:** `edgeplane system doctor [OPTIONS]`

###### **Options:**

* `--fix`

  Default value: `false`
* `--cleanup` — Also cleanup local profile/session artifacts after checks

  Default value: `false`
* `--cleanup-keep-instances <CLEANUP_KEEP_INSTANCES>` — When --cleanup is set, keep at most this many runtime instance dirs

  Default value: `8`
* `--cleanup-keep-bundles <CLEANUP_KEEP_BUNDLES>` — When --cleanup is set, keep at most this many bundle tar files per profile

  Default value: `6`
* `--cleanup-max-age-days <CLEANUP_MAX_AGE_DAYS>` — When --cleanup is set, remove instance dirs older than this many days

  Default value: `7`



## `edgeplane system backup`

Trigger local backups (postgres, rustfs, or both)

**Usage:** `edgeplane system backup [OPTIONS]`

###### **Options:**

* `--target <TARGET>`

  Default value: `all`

  Possible values: `postgres`, `rustfs`, `all`

* `--reason <REASON>`



## `edgeplane system profile-gc`

Cleanup local profile/session artifacts with retention limits

**Usage:** `edgeplane system profile-gc [OPTIONS]`

###### **Options:**

* `--keep-instances <KEEP_INSTANCES>` — Keep at most this many runtime instance dirs (newest first)

  Default value: `20`
* `--keep-bundles <KEEP_BUNDLES>` — Keep at most this many bundle tar files per profile (newest first)

  Default value: `10`
* `--max-age-days <MAX_AGE_DAYS>` — Remove instance dirs older than this many days regardless of count

  Default value: `14`



## `edgeplane system update`

Self-update helper for the edgeplane binary

**Usage:** `edgeplane system update <COMMAND>`

###### **Subcommands:**

* `self-update` — Update edgeplane by downloading the latest release artifact



## `edgeplane system update self-update`

Update edgeplane by downloading the latest release artifact

**Usage:** `edgeplane system update self-update [OPTIONS]`

###### **Options:**

* `--manifest-url <MANIFEST_URL>` — Manifest URL describing available releases

  Default value: `https://github.com/RyanMerlin/edgeplane/releases/latest/download/latest.json`
* `--skip-verify` — Skip checksum verification



## `edgeplane system compat`

Compatibility matrix commands and reports for provider/version drift control

**Usage:** `edgeplane system compat <COMMAND>`

###### **Subcommands:**

* `matrix` — Run compatibility checks for configured providers and emit a report artifact



## `edgeplane system compat matrix`

Run compatibility checks for configured providers and emit a report artifact

**Usage:** `edgeplane system compat matrix <COMMAND>`

###### **Subcommands:**

* `run` — Execute compatibility checks and write a report
* `report-latest` — Print the latest compatibility report



## `edgeplane system compat matrix run`

Execute compatibility checks and write a report

**Usage:** `edgeplane system compat matrix run [OPTIONS]`

###### **Options:**

* `--providers <PROVIDERS>` — Providers to test. Comma-delimited values, e.g. claude,codex

  Default value: `claude,codex`
* `--mode <MODE>` — Test depth profile

  Default value: `smoke`

  Possible values: `smoke`, `full`

* `--out <OUT>` — Optional explicit output path for the report JSON



## `edgeplane system compat matrix report-latest`

Print the latest compatibility report

**Usage:** `edgeplane system compat matrix report-latest`



## `edgeplane system drift`

Drift ingestion + policy decision helpers for staged release gates

**Usage:** `edgeplane system drift <COMMAND>`

###### **Subcommands:**

* `ingest` — Ingest a provider change signal and persist a capability delta artifact
* `triage` — Produce a policy decision from latest compatibility and drift artifacts



## `edgeplane system drift ingest`

Ingest a provider change signal and persist a capability delta artifact

**Usage:** `edgeplane system drift ingest [OPTIONS] --provider <PROVIDER> --version <VERSION> --source-url <SOURCE_URL> --summary <SUMMARY>`

###### **Options:**

* `--provider <PROVIDER>` — Provider identifier (e.g. claude, codex)
* `--version <VERSION>` — Version label seen in docs/release notes
* `--source-url <SOURCE_URL>` — Source URL where change was observed
* `--summary <SUMMARY>` — Human summary of the observed drift/change
* `--severity <SEVERITY>` — Drift severity classification

  Default value: `degraded`

  Possible values: `compatible`, `degraded`, `breaking`




## `edgeplane system drift triage`

Produce a policy decision from latest compatibility and drift artifacts

**Usage:** `edgeplane system drift triage [OPTIONS]`

###### **Options:**

* `--domain <DOMAIN>` — Optional domain id for bookkeeping
* `--provider <PROVIDER>` — Optional provider filter



## `edgeplane agent`

Agent control workflows (remote, swarm/subagent workflows)

**Usage:** `edgeplane agent <COMMAND>`

###### **Subcommands:**

* `signal` — Send a prompt to an agent (auto-resolves local vs controlplane)
* `cancel` — Interrupt an in-flight agent (auto-resolves local vs controlplane)
* `list` — List visible agents — local (edgeplaned-supervised) and/or remote (controlplane)
* `describe` — Describe a single agent — auto-resolves local vs controlplane
* `node` — Resident node-agent control verbs
* `attach` — Attach to a persistent ACP session — stream session/update frames to stdout, forward stdin lines as session/prompt
* `cron` — Cron jobs scheduled by edgeplaned (Phase 4 daemon-absorption)
* `supervise` — systemd-unit liveness supervision (Phase 5 daemon-absorption)
* `register` — Register a new agent with the controlplane
* `set-status` — Update a controlplane agent's status (online/offline/busy)
* `delete` — Delete an agent from the controlplane (sends DELETE /agents/{id})



## `edgeplane agent signal`

Send a prompt to an agent (auto-resolves local vs controlplane)

**Usage:** `edgeplane agent signal [OPTIONS] --content <CONTENT> <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>` — Agent id. For local agents this is the profile name (e.g. `work`); for controlplane agents it's the `public_id`

###### **Options:**

* `--content <CONTENT>` — Prompt text to inject. Multi-line is fine; quote it
* `--local` — Force the local mgmt-gateway path; skip the controlplane fallback
* `--remote` — Force the controlplane path; skip the local lookup



## `edgeplane agent cancel`

Interrupt an in-flight agent (auto-resolves local vs controlplane)

**Usage:** `edgeplane agent cancel [OPTIONS] <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>`

###### **Options:**

* `--local`
* `--remote`



## `edgeplane agent list`

List visible agents — local (edgeplaned-supervised) and/or remote (controlplane)

**Usage:** `edgeplane agent list [OPTIONS]`

###### **Options:**

* `--source <SOURCE>` — Which source to list. Default: `all` (both local + controlplane)

  Default value: `all`

  Possible values: `local`, `remote`, `all`

* `--json` — Emit raw JSON instead of the table view



## `edgeplane agent describe`

Describe a single agent — auto-resolves local vs controlplane

**Usage:** `edgeplane agent describe [OPTIONS] <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>`

###### **Options:**

* `--local`
* `--remote`
* `--json`



## `edgeplane agent node`

Resident node-agent control verbs

**Usage:** `edgeplane agent node <COMMAND>`

###### **Subcommands:**

* `register` — Register a node with Edgeplane and persist its identity locally
* `run` — Run the resident node loop
* `doctor` — Inspect local node-agent readiness
* `join-token` — Manage node join tokens (single-use bootstrap credentials)



## `edgeplane agent node register`

Register a node with Edgeplane and persist its identity locally

**Usage:** `edgeplane agent node register [OPTIONS] --hostname <HOSTNAME>`

###### **Options:**

* `--hostname <HOSTNAME>`
* `--trust-tier <TRUST_TIER>`

  Default value: `untrusted`



## `edgeplane agent node run`

Run the resident node loop

**Usage:** `edgeplane agent node run [OPTIONS]`

###### **Options:**

* `--poll-seconds <POLL_SECONDS>`

  Default value: `30`
* `--heartbeat-seconds <HEARTBEAT_SECONDS>`

  Default value: `15`
* `--node-name <NODE_NAME>`

  Default value: `node`
* `--hostname <HOSTNAME>`

  Default value: ``
* `--trust-tier <TRUST_TIER>`

  Default value: `untrusted`
* `--capabilities <CAPABILITIES>`

  Default value: `container,host_process`
* `--labels <LABELS>`

  Default value: ``



## `edgeplane agent node doctor`

Inspect local node-agent readiness

**Usage:** `edgeplane agent node doctor [OPTIONS]`

###### **Options:**

* `--node-name <NODE_NAME>`

  Default value: `node`



## `edgeplane agent node join-token`

Manage node join tokens (single-use bootstrap credentials)

**Usage:** `edgeplane agent node join-token <COMMAND>`

###### **Subcommands:**

* `create` — Create a new join token for bootstrapping a node
* `get` — Get a join token by ID
* `rotate` — Rotate a join token (invalidates the old one, issues a new secret)



## `edgeplane agent node join-token create`

Create a new join token for bootstrapping a node

**Usage:** `edgeplane agent node join-token create [OPTIONS]`

###### **Options:**

* `--ttl-seconds <TTL_SECONDS>` — Token TTL in seconds (default: 600 — 10 minutes)

  Default value: `600`



## `edgeplane agent node join-token get`

Get a join token by ID

**Usage:** `edgeplane agent node join-token get <TOKEN_ID>`

###### **Arguments:**

* `<TOKEN_ID>` — Join token ID (returned by `create`)



## `edgeplane agent node join-token rotate`

Rotate a join token (invalidates the old one, issues a new secret)

**Usage:** `edgeplane agent node join-token rotate <TOKEN_ID>`

###### **Arguments:**

* `<TOKEN_ID>` — Join token ID (returned by `create`)



## `edgeplane agent attach`

Attach to a persistent ACP session — stream session/update frames to stdout, forward stdin lines as session/prompt

**Usage:** `edgeplane agent attach [OPTIONS] <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>` — Agent id. For local ZellijHosted agents this is the profile name (e.g. `work`); for controlplane ACP agents it's the `public_id` (e.g. `aria-operator-e8820c0d`)

###### **Options:**

* `--json` — Stream raw `SessionNotification` JSON, one frame per line. Default is a human-readable rendering of assistant turns, tool calls, etc. Only meaningful for ACP attach; ignored for ZellijHosted
* `--node-id <NODE_ID>` — Override node id when the agent registry doesn't know which node hosts this agent (rare; mostly useful during early bringup before linkage is fully populated). Only meaningful for ACP attach
* `--web` — For ZellijHosted agents only: instead of exec'ing `zellij attach`, print the `zellij web` URL (`http://<base>/<session>`). Useful for embedding in a browser or sharing the link
* `--web-base-url <WEB_BASE_URL>` — Base URL for `zellij web` when --web is set. Defaults to the local `zellij web` listener at `http://127.0.0.1:8082`

  Default value: `http://127.0.0.1:8082`
* `--remote` — Force the controlplane ACP attach path; skip the local lookup. Useful when an agent ID happens to collide between local and controlplane and you know you want the controlplane one



## `edgeplane agent cron`

Cron jobs scheduled by edgeplaned (Phase 4 daemon-absorption)

**Usage:** `edgeplane agent cron <COMMAND>`

###### **Subcommands:**

* `list` — List all cron jobs from `~/.ep/edgeplaned/cron.toml` + their runtime state
* `describe` — Describe one cron job: schedule, last fire, recent history
* `reload` — Re-parse `cron.toml` (edgeplaned reloads on its next tick)
* `history` — Recent fires across all (or one) job
* `gc-now` — Force a retention sweep on `agent_cron_fire_log` now



## `edgeplane agent cron list`

List all cron jobs from `~/.ep/edgeplaned/cron.toml` + their runtime state

**Usage:** `edgeplane agent cron list [OPTIONS]`

###### **Options:**

* `--json`



## `edgeplane agent cron describe`

Describe one cron job: schedule, last fire, recent history

**Usage:** `edgeplane agent cron describe [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>` — Job name as it appears in `cron.toml`

###### **Options:**

* `--limit <LIMIT>` — How many recent fires to include in the output

  Default value: `5`
* `--json`



## `edgeplane agent cron reload`

Re-parse `cron.toml` (edgeplaned reloads on its next tick)

**Usage:** `edgeplane agent cron reload`



## `edgeplane agent cron history`

Recent fires across all (or one) job

**Usage:** `edgeplane agent cron history [OPTIONS]`

###### **Options:**

* `--name <NAME>` — Filter to one job; default shows fires across all jobs
* `-n`, `--limit <LIMIT>` — Maximum number of fires to show. Default 20

  Default value: `20`
* `--json`



## `edgeplane agent cron gc-now`

Force a retention sweep on `agent_cron_fire_log` now

**Usage:** `edgeplane agent cron gc-now [OPTIONS]`

###### **Options:**

* `--history-days <HISTORY_DAYS>` — Override `cron.toml`'s `[retention] history_days` for this sweep only
* `--max-rows-per-job <MAX_ROWS_PER_JOB>` — Override `cron.toml`'s `[retention] max_rows_per_job` for this sweep only



## `edgeplane agent supervise`

systemd-unit liveness supervision (Phase 5 daemon-absorption)

**Usage:** `edgeplane agent supervise <COMMAND>`

###### **Subcommands:**

* `list` — List supervised agents and their current unit state
* `status` — One agent's launch context + recent restart history
* `restart` — Manual `systemctl --user restart <agent>` (logged as reason=manual)
* `pause` — Pause the auto-restart loop for an agent (or all)
* `resume` — Resume the auto-restart loop
* `history` — Recent restart events across all (or one) agent
* `events` — Stream live SupervisorEvents from edgeplaned (Ctrl-C to exit)
* `watch` — Live fleet dashboard: agent table + event tail (TUI, q to quit)



## `edgeplane agent supervise list`

List supervised agents and their current unit state

**Usage:** `edgeplane agent supervise list [OPTIONS]`

###### **Options:**

* `--json`



## `edgeplane agent supervise status`

One agent's launch context + recent restart history

**Usage:** `edgeplane agent supervise status [OPTIONS] <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>` — Agent id (e.g. "work", "operator", or the full agent_id)

###### **Options:**

* `--limit <LIMIT>` — How many recent restart events to show. Default 5

  Default value: `5`
* `--json`



## `edgeplane agent supervise restart`

Manual `systemctl --user restart <agent>` (logged as reason=manual)

**Usage:** `edgeplane agent supervise restart <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>` — Agent id to restart. Logged as reason="manual"



## `edgeplane agent supervise pause`

Pause the auto-restart loop for an agent (or all)

**Usage:** `edgeplane agent supervise pause [OPTIONS] [AGENT_ID]`

###### **Arguments:**

* `<AGENT_ID>` — Agent id to pause/resume. Mutually exclusive with --all

###### **Options:**

* `--all` — Apply to every supervised agent on this node



## `edgeplane agent supervise resume`

Resume the auto-restart loop

**Usage:** `edgeplane agent supervise resume [OPTIONS] [AGENT_ID]`

###### **Arguments:**

* `<AGENT_ID>` — Agent id to pause/resume. Mutually exclusive with --all

###### **Options:**

* `--all` — Apply to every supervised agent on this node



## `edgeplane agent supervise history`

Recent restart events across all (or one) agent

**Usage:** `edgeplane agent supervise history [OPTIONS]`

###### **Options:**

* `--agent-id <AGENT_ID>` — Filter to one agent's restart history. Default shows recent restarts across all supervised agents
* `-n`, `--limit <LIMIT>` — Maximum entries to show. Default 20

  Default value: `20`
* `--json`



## `edgeplane agent supervise events`

Stream live SupervisorEvents from edgeplaned (Ctrl-C to exit)

**Usage:** `edgeplane agent supervise events [OPTIONS]`

###### **Options:**

* `--json` — Emit raw JSON event frames (one per line) instead of pretty-printed lines



## `edgeplane agent supervise watch`

Live fleet dashboard: agent table + event tail (TUI, q to quit)

**Usage:** `edgeplane agent supervise watch [OPTIONS]`

###### **Options:**

* `--poll-secs <POLL_SECS>` — Snapshot poll interval in seconds. Default 5

  Default value: `5`
* `--tail-size <TAIL_SIZE>` — Maximum events to retain in the scrollback. Default 200

  Default value: `200`



## `edgeplane agent register`

Register a new agent with the controlplane

**Usage:** `edgeplane agent register [OPTIONS] --name <NAME>`

###### **Options:**

* `--name <NAME>` — Agent name (must be unique on the controlplane)
* `--capabilities <CAPABILITIES>` — Comma-separated capability tags (e.g. `fleet-management,code-editing`)

  Default value: ``
* `--metadata <METADATA>` — Optional JSON metadata string (e.g. `{"runtime":"claude-code","node_id":"excalibur"}`)
* `--json` — Emit raw JSON instead of a human-readable summary



## `edgeplane agent set-status`

Update a controlplane agent's status (online/offline/busy)

**Usage:** `edgeplane agent set-status [OPTIONS] --id <ID> --status <STATUS>`

###### **Options:**

* `--id <ID>` — Agent id or public_id on the controlplane
* `--status <STATUS>` — New status value (e.g. `online`, `offline`, `busy`)
* `--json` — Emit raw JSON instead of a human-readable summary



## `edgeplane agent delete`

Delete an agent from the controlplane (sends DELETE /agents/{id})

**Usage:** `edgeplane agent delete [OPTIONS] <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>` — Agent public_id or numeric id to delete

###### **Options:**

* `-y`, `--yes` — Skip the confirmation prompt
* `--json` — Emit raw JSON instead of a human-readable summary



## `edgeplane runtime`

Runtime fabric workflows (nodes, jobs, leases)

**Usage:** `edgeplane runtime <COMMAND>`

###### **Subcommands:**

* `nodes` — Runtime node operations
* `jobs` — Runtime job operations
* `leases` — Runtime lease helpers
* `sessions` — Runtime execution-session helpers



## `edgeplane runtime nodes`

Runtime node operations

**Usage:** `edgeplane runtime nodes <COMMAND>`

###### **Subcommands:**

* `register` — 
* `list` — 
* `heartbeat` — 



## `edgeplane runtime nodes register`

**Usage:** `edgeplane runtime nodes register [OPTIONS] --hostname <HOSTNAME>`

###### **Options:**

* `--hostname <HOSTNAME>`
* `--trust-tier <TRUST_TIER>`

  Default value: `untrusted`



## `edgeplane runtime nodes list`

**Usage:** `edgeplane runtime nodes list [OPTIONS]`

###### **Options:**

* `--status <STATUS>`



## `edgeplane runtime nodes heartbeat`

**Usage:** `edgeplane runtime nodes heartbeat [OPTIONS] --node-id <NODE_ID>`

###### **Options:**

* `--node-id <NODE_ID>`
* `--status <STATUS>`

  Default value: `online`



## `edgeplane runtime jobs`

Runtime job operations

**Usage:** `edgeplane runtime jobs <COMMAND>`

###### **Subcommands:**

* `submit` — 
* `list` — 



## `edgeplane runtime jobs submit`

**Usage:** `edgeplane runtime jobs submit [OPTIONS]`

###### **Options:**

* `--domain-id <DOMAIN_ID>`

  Default value: ``
* `--runtime-session-id <RUNTIME_SESSION_ID>`

  Default value: ``
* `--runtime-class <RUNTIME_CLASS>`

  Default value: `container`
* `--image <IMAGE>`

  Default value: ``
* `--command <COMMAND>`

  Default value: ``



## `edgeplane runtime jobs list`

**Usage:** `edgeplane runtime jobs list [OPTIONS]`

###### **Options:**

* `--status <STATUS>`



## `edgeplane runtime leases`

Runtime lease helpers

**Usage:** `edgeplane runtime leases <COMMAND>`

###### **Subcommands:**

* `create` — 
* `status` — 
* `complete` — 



## `edgeplane runtime leases create`

**Usage:** `edgeplane runtime leases create --job-id <JOB_ID> --node-id <NODE_ID>`

###### **Options:**

* `--job-id <JOB_ID>`
* `--node-id <NODE_ID>`



## `edgeplane runtime leases status`

**Usage:** `edgeplane runtime leases status --lease-id <LEASE_ID> --status <STATUS>`

###### **Options:**

* `--lease-id <LEASE_ID>`
* `--status <STATUS>`



## `edgeplane runtime leases complete`

**Usage:** `edgeplane runtime leases complete [OPTIONS] --lease-id <LEASE_ID>`

###### **Options:**

* `--lease-id <LEASE_ID>`
* `--exit-code <EXIT_CODE>`

  Default value: `0`
* `--error-message <ERROR_MESSAGE>`

  Default value: ``



## `edgeplane runtime sessions`

Runtime execution-session helpers

**Usage:** `edgeplane runtime sessions <COMMAND>`

###### **Subcommands:**

* `attach` — 



## `edgeplane runtime sessions attach`

**Usage:** `edgeplane runtime sessions attach [OPTIONS] --session-id <SESSION_ID>`

###### **Options:**

* `--session-id <SESSION_ID>`
* `--raw`

  Default value: `false`



## `edgeplane approvals`

Approval workflow commands (requests, decisions)

**Usage:** `edgeplane approvals <COMMAND>`

###### **Subcommands:**

* `create` — Create an approval request for a domain action
* `list` — List approval requests for a domain
* `approve` — Approve a pending request
* `reject` — Reject a pending request



## `edgeplane approvals create`

Create an approval request for a domain action

**Usage:** `edgeplane approvals create [OPTIONS] --domain-id <DOMAIN_ID> --action <ACTION>`

###### **Options:**

* `--domain-id <DOMAIN_ID>`
* `--action <ACTION>`
* `--channel <CHANNEL>`
* `--reason <REASON>`
* `--target-entity-type <TARGET_ENTITY_TYPE>`
* `--target-entity-id <TARGET_ENTITY_ID>`
* `--request-context <REQUEST_CONTEXT>`
* `--expires-in-seconds <EXPIRES_IN_SECONDS>`



## `edgeplane approvals list`

List approval requests for a domain

**Usage:** `edgeplane approvals list [OPTIONS] --domain-id <DOMAIN_ID>`

###### **Options:**

* `--domain-id <DOMAIN_ID>`
* `--status <STATUS>`
* `--limit <LIMIT>`



## `edgeplane approvals approve`

Approve a pending request

**Usage:** `edgeplane approvals approve [OPTIONS] --approval-id <APPROVAL_ID>`

###### **Options:**

* `--approval-id <APPROVAL_ID>`
* `--expires-in-seconds <EXPIRES_IN_SECONDS>`
* `--note <NOTE>`



## `edgeplane approvals reject`

Reject a pending request

**Usage:** `edgeplane approvals reject [OPTIONS] --approval-id <APPROVAL_ID>`

###### **Options:**

* `--approval-id <APPROVAL_ID>`
* `--note <NOTE>`



## `edgeplane workspace`

Workspace lifecycle helpers (load/heartbeat/artifact/commit/release)

**Usage:** `edgeplane workspace <COMMAND>`

###### **Subcommands:**

* `load` — Load and lease a Mission workspace
* `heartbeat` — Heartbeat an existing workspace lease
* `fetch-artifact` — Fetch an artifact via the lease (download URL or inline content)
* `commit` — Commit workspace changes with a JSON change_set
* `release` — Release a lease with an optional reason



## `edgeplane workspace load`

Load and lease a Mission workspace

**Usage:** `edgeplane workspace load [OPTIONS] --mission-id <MISSION_ID>`

###### **Options:**

* `--mission-id <MISSION_ID>`
* `--workspace-label <WORKSPACE_LABEL>`
* `--agent-id <AGENT_ID>`
* `--lease-seconds <LEASE_SECONDS>`

  Default value: `900`



## `edgeplane workspace heartbeat`

Heartbeat an existing workspace lease

**Usage:** `edgeplane workspace heartbeat --lease-id <LEASE_ID>`

###### **Options:**

* `--lease-id <LEASE_ID>`



## `edgeplane workspace fetch-artifact`

Fetch an artifact via the lease (download URL or inline content)

**Usage:** `edgeplane workspace fetch-artifact [OPTIONS] --lease-id <LEASE_ID> --artifact-id <ARTIFACT_ID>`

###### **Options:**

* `--lease-id <LEASE_ID>`
* `--artifact-id <ARTIFACT_ID>`
* `--mode <MODE>`

  Default value: `content`
* `--expires-seconds <EXPIRES_SECONDS>`

  Default value: `60`
* `--out <OUT>` — When mode=content, decode and write bytes to this local path



## `edgeplane workspace commit`

Commit workspace changes with a JSON change_set

**Usage:** `edgeplane workspace commit [OPTIONS] --lease-id <LEASE_ID> --change-set <CHANGE_SET>`

###### **Options:**

* `--lease-id <LEASE_ID>`
* `--change-set <CHANGE_SET>`
* `--validation-mode <VALIDATION_MODE>`



## `edgeplane workspace release`

Release a lease with an optional reason

**Usage:** `edgeplane workspace release [OPTIONS] --lease-id <LEASE_ID>`

###### **Options:**

* `--lease-id <LEASE_ID>`
* `--reason <REASON>`



## `edgeplane ops`

Domain operations (lifecycle orchestration and execution workflows)

**Usage:** `edgeplane ops <COMMAND>`

###### **Subcommands:**

* `domain` — Domain-level lifecycle actions that build on workspace leases



## `edgeplane ops domain`

Domain-level lifecycle actions that build on workspace leases

**Usage:** `edgeplane ops domain [OPTIONS] --action <ACTION>`

###### **Options:**

* `--action <ACTION>` — Domain action to execute

  Possible values: `start`, `heartbeat`, `commit`, `release`

* `--mission-id <MISSION_ID>` — Target mission (required for start)
* `--lease-id <LEASE_ID>` — Lease ID to manage
* `--workspace-label <WORKSPACE_LABEL>` — Optional workspace label created during start
* `--agent-id <AGENT_ID>` — Optional agent identifier for the lease
* `--lease-seconds <LEASE_SECONDS>` — Lease duration in seconds
* `--change-set <CHANGE_SET>` — Change set JSON for commits

  Default value: `{}`
* `--validation-mode <VALIDATION_MODE>` — Validation mode used when committing
* `--reason <REASON>` — Optional release reason



## `edgeplane update`

Self-update helper for the edgeplane binary

**Usage:** `edgeplane update [OPTIONS]`

###### **Options:**

* `--manifest-url <MANIFEST_URL>` — Manifest URL describing available releases

  Default value: `https://github.com/RyanMerlin/edgeplane/releases/latest/download/latest.json`
* `--skip-verify` — Skip checksum verification



## `edgeplane init`

Initialize EdgePlane profile state for first-time usage

**Usage:** `edgeplane init [OPTIONS]`

###### **Options:**

* `--profile <PROFILE>` — Initial profile name to create when none exists

  Default value: `default`
* `--repo <REPO>` — Bootstrap this node from a sync repo URL (clones repo, stores INFISICAL_TOKEN, writes config)



## `edgeplane serve`

Start an MCP server (stdio JSON-RPC 2.0) for LLM runtime connections

**Usage:** `edgeplane serve [OPTIONS]`

###### **Options:**

* `--tools-cache-ttl <TOOLS_CACHE_TTL>` — Tools cache TTL in seconds (default: 60)

  Default value: `60`
* `--preflight` — Run a preflight health check before entering the message loop.

   Disabled by default because an stdio MCP server must respond to `initialize` immediately; blocking on a network call delays startup and causes agents (e.g. Codex) to time out waiting for the handshake. Enable only when invoking `edgeplane serve` outside an agent context.
* `--debug-protocol` — Log MCP messages to stderr for debugging



## `edgeplane channel`

Claude channel server integrations

**Usage:** `edgeplane channel <COMMAND>`

###### **Subcommands:**

* `claude` — Claude channel integrations



## `edgeplane channel claude`

Claude channel integrations

**Usage:** `edgeplane channel claude <COMMAND>`

###### **Subcommands:**

* `webhook` — Expose a local webhook that forwards inbound messages to Claude via channel notifications



## `edgeplane channel claude webhook`

Expose a local webhook that forwards inbound messages to Claude via channel notifications

**Usage:** `edgeplane channel claude webhook [OPTIONS]`

###### **Options:**

* `--listen-host <LISTEN_HOST>` — Host/interface for the local webhook listener

  Default value: `127.0.0.1`
* `--listen-port <LISTEN_PORT>` — Port for the local webhook listener

  Default value: `8788`
* `--channel-name <CHANNEL_NAME>` — Name used in channel metadata

  Default value: `edgeplane`
* `--instructions <INSTRUCTIONS>` — Optional instructions to pass to Claude for this channel
* `--enable-reply` — Expose a standard MCP reply tool

  Default value: `false`
* `--debug-protocol` — Log protocol traffic to stderr

  Default value: `false`



## `edgeplane profile`

Manage Edgeplane user profiles

**Usage:** `edgeplane profile <COMMAND>`

###### **Subcommands:**

* `create` — Create a new profile shell on Edgeplane (empty bundle)
* `list` — List current user's profiles
* `show` — Show profile metadata by name
* `activate` — Activate profile as default
* `download` — Download profile bundle to a local file
* `publish` — Publish/replace a profile bundle in Edgeplane
* `pull` — Pull profile bundle from Edgeplane into local profile cache
* `pin` — Pin a local profile to a specific remote sha256
* `delete` — Delete a profile from Edgeplane (requires explicit confirmation flag)
* `status` — Show remote/local pin status for a profile
* `use` — Activate a profile as default and apply its bundle locally in one step



## `edgeplane profile create`

Create a new profile shell on Edgeplane (empty bundle)

**Usage:** `edgeplane profile create [OPTIONS] --name <NAME>`

###### **Options:**

* `--name <NAME>`
* `--description <DESCRIPTION>`

  Default value: ``
* `--activate`



## `edgeplane profile list`

List current user's profiles

**Usage:** `edgeplane profile list [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`

  Default value: `50`



## `edgeplane profile show`

Show profile metadata by name

**Usage:** `edgeplane profile show --name <NAME>`

###### **Options:**

* `--name <NAME>`



## `edgeplane profile activate`

Activate profile as default

**Usage:** `edgeplane profile activate --name <NAME>`

###### **Options:**

* `--name <NAME>`



## `edgeplane profile download`

Download profile bundle to a local file

**Usage:** `edgeplane profile download [OPTIONS] --name <NAME>`

###### **Options:**

* `--name <NAME>`
* `--out <OUT>`



## `edgeplane profile publish`

Publish/replace a profile bundle in Edgeplane

**Usage:** `edgeplane profile publish [OPTIONS] --name <NAME>`

###### **Options:**

* `--name <NAME>`
* `--bundle <BUNDLE>`
* `--from-profile-dir <FROM_PROFILE_DIR>`
* `--description <DESCRIPTION>`
* `--manifest-file <MANIFEST_FILE>`
* `--activate`



## `edgeplane profile pull`

Pull profile bundle from Edgeplane into local profile cache

**Usage:** `edgeplane profile pull [OPTIONS] --name <NAME>`

###### **Options:**

* `--name <NAME>`
* `--apply`
* `--allow-pin-mismatch`



## `edgeplane profile pin`

Pin a local profile to a specific remote sha256

**Usage:** `edgeplane profile pin --name <NAME> --sha256 <SHA256>`

###### **Options:**

* `--name <NAME>`
* `--sha256 <SHA256>`



## `edgeplane profile delete`

Delete a profile from Edgeplane (requires explicit confirmation flag)

**Usage:** `edgeplane profile delete [OPTIONS] --name <NAME>`

###### **Options:**

* `--name <NAME>`
* `--confirm-delete`

  Default value: `false`



## `edgeplane profile status`

Show remote/local pin status for a profile

**Usage:** `edgeplane profile status --name <NAME>`

###### **Options:**

* `--name <NAME>`



## `edgeplane profile use`

Activate a profile as default and apply its bundle locally in one step

**Usage:** `edgeplane profile use --name <NAME>`

###### **Options:**

* `--name <NAME>`



## `edgeplane secrets`

Secrets provider + reference helpers

**Usage:** `edgeplane secrets <COMMAND>`

###### **Subcommands:**

* `status` — Inspect current secrets provider config for a profile
* `provider` — Configure secrets provider for a profile
* `get` — Resolve and print one named secret from the active profile mapping
* `bootstrap` — Seed standard secret refs for the selected provider
* `rotate` — Rotate one mapped secret for a profile
* `export-env` — Resolve mapped secrets and write a .env-style file
* `infisical` — Manage Infisical connection profiles (multi-account, multi-instance)



## `edgeplane secrets status`

Inspect current secrets provider config for a profile

**Usage:** `edgeplane secrets status [OPTIONS]`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`



## `edgeplane secrets provider`

Configure secrets provider for a profile

**Usage:** `edgeplane secrets provider <COMMAND>`

###### **Subcommands:**

* `env` — Set provider to env
* `infisical` — Set provider to Infisical and persist connection metadata



## `edgeplane secrets provider env`

Set provider to env

**Usage:** `edgeplane secrets provider env [OPTIONS]`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`



## `edgeplane secrets provider infisical`

Set provider to Infisical and persist connection metadata

**Usage:** `edgeplane secrets provider infisical [OPTIONS]`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`
* `--project-id <PROJECT_ID>`
* `--env <ENV>`
* `--path <PATH>`



## `edgeplane secrets get`

Resolve and print one named secret from the active profile mapping

**Usage:** `edgeplane secrets get [OPTIONS] --name <NAME>`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`
* `--name <NAME>`
* `--reveal` — Show the value (default redacts in human mode)

  Default value: `false`



## `edgeplane secrets bootstrap`

Seed standard secret refs for the selected provider

**Usage:** `edgeplane secrets bootstrap [OPTIONS]`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`
* `--keep-existing` — Do not overwrite existing refs

  Default value: `false`
* `--via-api` — Call backend admin endpoint instead of local file mutation

  Default value: `false`



## `edgeplane secrets rotate`

Rotate one mapped secret for a profile

**Usage:** `edgeplane secrets rotate [OPTIONS] --name <NAME>`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`
* `--name <NAME>`
* `--value <VALUE>`
* `--generator <GENERATOR>`

  Default value: `token`
* `--via-api` — Call backend admin endpoint instead of local mutation

  Default value: `false`



## `edgeplane secrets export-env`

Resolve mapped secrets and write a .env-style file

**Usage:** `edgeplane secrets export-env [OPTIONS] --out <OUT>`

###### **Options:**

* `--profile <PROFILE>`

  Default value: `default`
* `--out <OUT>`



## `edgeplane secrets infisical`

Manage Infisical connection profiles (multi-account, multi-instance)

**Usage:** `edgeplane secrets infisical <COMMAND>`

###### **Subcommands:**

* `add` — Add or update a named Infisical connection profile
* `list` — List all Infisical profiles (marks the active one)
* `use` — Switch the active Infisical profile
* `test` — Test connectivity to the active (or specified) Infisical profile
* `rm` — Remove a named Infisical profile
* `get` — Fetch a secret value from Infisical using the active profile



## `edgeplane secrets infisical add`

Add or update a named Infisical connection profile

**Usage:** `edgeplane secrets infisical add [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>` — Profile name (e.g. "work", "personal")

###### **Options:**

* `--site-url <SITE_URL>` — Infisical instance URL (default: https://app.infisical.com)

  Default value: `https://app.infisical.com`
* `--service-token <SERVICE_TOKEN>` — Service token (mutually exclusive with --client-id / --client-secret)
* `--client-id <CLIENT_ID>` — Universal Auth client ID
* `--client-secret <CLIENT_SECRET>` — Universal Auth client secret
* `--project-id <PROJECT_ID>` — Default project ID
* `--environment <ENVIRONMENT>` — Default environment slug

  Default value: `prod`
* `--activate` — Make this the active profile after adding

  Default value: `true`



## `edgeplane secrets infisical list`

List all Infisical profiles (marks the active one)

**Usage:** `edgeplane secrets infisical list`



## `edgeplane secrets infisical use`

Switch the active Infisical profile

**Usage:** `edgeplane secrets infisical use <NAME>`

###### **Arguments:**

* `<NAME>` — Profile name to activate



## `edgeplane secrets infisical test`

Test connectivity to the active (or specified) Infisical profile

**Usage:** `edgeplane secrets infisical test [NAME]`

###### **Arguments:**

* `<NAME>` — Profile name to test (default: active profile)



## `edgeplane secrets infisical rm`

Remove a named Infisical profile

**Usage:** `edgeplane secrets infisical rm <NAME>`

###### **Arguments:**

* `<NAME>` — Profile name to remove



## `edgeplane secrets infisical get`

Fetch a secret value from Infisical using the active profile

**Usage:** `edgeplane secrets infisical get [OPTIONS] <SECRET_NAME>`

###### **Arguments:**

* `<SECRET_NAME>` — Secret name (key) to fetch

###### **Options:**

* `--profile <PROFILE>` — Override the profile to use (default: active profile)
* `--project-id <PROJECT_ID>` — Override the project ID (default: profile's default_project_id)
* `--environment <ENVIRONMENT>` — Override the environment slug (default: profile's default_environment)
* `--path <PATH>` — Secret path (default: /)

  Default value: `/`
* `--reveal` — Print the raw value without redaction (default: redacted)



## `edgeplane daemon`

edgeplaned daemon control and work-model commands

**Usage:** `edgeplane daemon <COMMAND>`

###### **Subcommands:**

* `up` — Bring edgeplaned up: install if missing, then start the daemon
* `down` — Stop the running edgeplaned daemon (install stays)
* `uninstall` — Remove the edgeplaned binary and systemd unit
* `status` — Show daemon health: backend reachable, runtimes, watchdog state
* `health` — Deep health check with individual component results
* `upgrade` — Upgrade the edgeplaned binary in place
* `version` — Print edgeplaned daemon version
* `runtime` — Manage locally installed agent runtimes
* `agent` — Manage agents in a domain's durable pool
* `mission` — Inspect missions and their task DAGs
* `task` — Manage and observe tasks
* `msg` — Send and tail inter-agent messages
* `attach` — Attach to a running agent, task, or exec (auto-detected)
* `watch` — Unified live feed of progress events and messages
* `profile` — Manage controlplane profiles (add, list, remove, rename)
* `use` — Select the active controlplane profile (or show the current one)



## `edgeplane daemon up`

Bring edgeplaned up: install if missing, then start the daemon

**Usage:** `edgeplane daemon up [OPTIONS]`

###### **Options:**

* `--backend-url <BACKEND_URL>`
* `--yes`



## `edgeplane daemon down`

Stop the running edgeplaned daemon (install stays)

**Usage:** `edgeplane daemon down`



## `edgeplane daemon uninstall`

Remove the edgeplaned binary and systemd unit

**Usage:** `edgeplane daemon uninstall`



## `edgeplane daemon status`

Show daemon health: backend reachable, runtimes, watchdog state

**Usage:** `edgeplane daemon status`



## `edgeplane daemon health`

Deep health check with individual component results

**Usage:** `edgeplane daemon health`



## `edgeplane daemon upgrade`

Upgrade the edgeplaned binary in place

**Usage:** `edgeplane daemon upgrade`

###### **Options:**

* `--version <VERSION>`



## `edgeplane daemon version`

Print edgeplaned daemon version

**Usage:** `edgeplane daemon version`



## `edgeplane daemon runtime`

Manage locally installed agent runtimes

**Usage:** `edgeplane daemon runtime <COMMAND>`

###### **Subcommands:**

* `ls` — 
* `install` — 
* `test` — 



## `edgeplane daemon runtime ls`

**Usage:** `edgeplane daemon runtime ls`



## `edgeplane daemon runtime install`

**Usage:** `edgeplane daemon runtime install <KIND>`

###### **Arguments:**

* `<KIND>`



## `edgeplane daemon runtime test`

**Usage:** `edgeplane daemon runtime test <KIND>`

###### **Arguments:**

* `<KIND>`



## `edgeplane daemon agent`

Manage agents in a domain's durable pool

**Usage:** `edgeplane daemon agent <COMMAND>`

###### **Subcommands:**

* `ls` — List agents. In standalone mode reads the local registry; in federated mode queries the controlplane
* `enroll` — Enroll a new agent. In standalone mode writes to the local registry (~/.ep/registry.db); in federated mode calls the controlplane API
* `enroll-home` — Provision the per-node home domain and enroll a default Goose agent in it. Standalone mirror of the controlplane's auto-provisioning at node-register time. Idempotent
* `import` — Bulk-import agents from a TOML manifest into the local registry. Each `[[profile]]` block is upserted as a zellij_hosted / persistent agent with a matching launch context. Idempotent — re-running updates in place. The daemon picks up changes on its next reconcile tick
* `reassign` — Reassign an agent to a different domain
* `unenroll` — Remove an agent from the registry / controlplane
* `attach` — 
* `profile` — Set or update an agent's profile (role, instructions, scope, constraints)



## `edgeplane daemon agent ls`

List agents. In standalone mode reads the local registry; in federated mode queries the controlplane

**Usage:** `edgeplane daemon agent ls [OPTIONS]`

###### **Options:**

* `--domain <DOMAIN>`
* `--status <STATUS>`



## `edgeplane daemon agent enroll`

Enroll a new agent. In standalone mode writes to the local registry (~/.ep/registry.db); in federated mode calls the controlplane API

**Usage:** `edgeplane daemon agent enroll [OPTIONS] --domain <DOMAIN> --runtime <RUNTIME>`

###### **Options:**

* `--domain <DOMAIN>`
* `--runtime <RUNTIME>`
* `--supervision <SUPERVISION>` — Task (default) or persistent supervision mode

  Default value: `task`
* `--node <NODE>`
* `--profile <PROFILE>` — Path to a YAML or JSON profile file for this agent



## `edgeplane daemon agent enroll-home`

Provision the per-node home domain and enroll a default Goose agent in it. Standalone mirror of the controlplane's auto-provisioning at node-register time. Idempotent

**Usage:** `edgeplane daemon agent enroll-home [OPTIONS]`

###### **Options:**

* `--hostname <HOSTNAME>` — Hostname used to form the home domain slug `home-{slug(hostname)}`. Defaults to the Tailscale FQDN leaf (when Tailscale is running) or the system hostname
* `--runtime <RUNTIME>` — Runtime kind for the default home-domain agent. Goose is the recommended default — cheap local inference for routing/triage

  Default value: `goose`



## `edgeplane daemon agent import`

Bulk-import agents from a TOML manifest into the local registry. Each `[[profile]]` block is upserted as a zellij_hosted / persistent agent with a matching launch context. Idempotent — re-running updates in place. The daemon picks up changes on its next reconcile tick

**Usage:** `edgeplane daemon agent import [OPTIONS] <PATH>`

###### **Arguments:**

* `<PATH>` — Path to a TOML manifest with `[[profile]]` blocks

###### **Options:**

* `--source <SOURCE>` — Source tag to associate with imported agents. Defaults to `manifest_import`. Use a stable tag (e.g. `aria`) so that re-runs update in place rather than accumulating duplicate rows

  Default value: `manifest_import`



## `edgeplane daemon agent reassign`

Reassign an agent to a different domain

**Usage:** `edgeplane daemon agent reassign --domain <DOMAIN> <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>`

###### **Options:**

* `--domain <DOMAIN>` — New domain ID



## `edgeplane daemon agent unenroll`

Remove an agent from the registry / controlplane

**Usage:** `edgeplane daemon agent unenroll <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>`



## `edgeplane daemon agent attach`

**Usage:** `edgeplane daemon agent attach <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>`



## `edgeplane daemon agent profile`

Set or update an agent's profile (role, instructions, scope, constraints)

**Usage:** `edgeplane daemon agent profile [OPTIONS] <AGENT_ID>`

###### **Arguments:**

* `<AGENT_ID>` — Agent ID to update

###### **Options:**

* `--file <FILE>` — Path to a YAML or JSON file containing the profile
* `--name <NAME>` — Quick single-field overrides: --name, --role, --instructions
* `--role <ROLE>`
* `--instructions <INSTRUCTIONS>`



## `edgeplane daemon mission`

Inspect missions and their task DAGs

**Usage:** `edgeplane daemon mission <COMMAND>`

###### **Subcommands:**

* `ls` — 
* `show` — 
* `watch` — 



## `edgeplane daemon mission ls`

**Usage:** `edgeplane daemon mission ls [OPTIONS]`

###### **Options:**

* `--domain <DOMAIN>`



## `edgeplane daemon mission show`

**Usage:** `edgeplane daemon mission show <MISSION_ID>`

###### **Arguments:**

* `<MISSION_ID>`



## `edgeplane daemon mission watch`

**Usage:** `edgeplane daemon mission watch <MISSION_ID>`

###### **Arguments:**

* `<MISSION_ID>`



## `edgeplane daemon task`

Manage and observe tasks

**Usage:** `edgeplane daemon task <COMMAND>`

###### **Subcommands:**

* `run` — 
* `ls` — 
* `show` — 
* `watch` — 
* `attach` — 
* `cancel` — 
* `retry` — 



## `edgeplane daemon task run`

**Usage:** `edgeplane daemon task run [OPTIONS] --title <TITLE> <MISSION_ID>`

###### **Arguments:**

* `<MISSION_ID>`

###### **Options:**

* `--title <TITLE>`
* `--description <DESCRIPTION>`

  Default value: ``
* `--claim-policy <CLAIM_POLICY>`

  Default value: `first_claim`
* `--runtime <RUNTIME>`
* `--depends-on <DEPENDS_ON>`
* `--priority <PRIORITY>`

  Default value: `0`
* `--input-file <INPUT_FILE>`



## `edgeplane daemon task ls`

**Usage:** `edgeplane daemon task ls [OPTIONS]`

###### **Options:**

* `--mission <MISSION>`
* `--domain <DOMAIN>`
* `--status <STATUS>`



## `edgeplane daemon task show`

**Usage:** `edgeplane daemon task show <TASK_ID>`

###### **Arguments:**

* `<TASK_ID>`



## `edgeplane daemon task watch`

**Usage:** `edgeplane daemon task watch [OPTIONS] <TASK_ID>`

###### **Arguments:**

* `<TASK_ID>`

###### **Options:**

* `--interval-secs <INTERVAL_SECS>`

  Default value: `2`



## `edgeplane daemon task attach`

**Usage:** `edgeplane daemon task attach <TASK_ID>`

###### **Arguments:**

* `<TASK_ID>`



## `edgeplane daemon task cancel`

**Usage:** `edgeplane daemon task cancel <TASK_ID>`

###### **Arguments:**

* `<TASK_ID>`



## `edgeplane daemon task retry`

**Usage:** `edgeplane daemon task retry <TASK_ID>`

###### **Arguments:**

* `<TASK_ID>`



## `edgeplane daemon msg`

Send and tail inter-agent messages

**Usage:** `edgeplane daemon msg <COMMAND>`

###### **Subcommands:**

* `send` — 
* `tail` — 



## `edgeplane daemon msg send`

**Usage:** `edgeplane daemon msg send [OPTIONS] <BODY>`

###### **Arguments:**

* `<BODY>`

###### **Options:**

* `--mission <MISSION>`
* `--domain <DOMAIN>`
* `--to <TO>`
* `--channel <CHANNEL>`

  Default value: `coordination`



## `edgeplane daemon msg tail`

**Usage:** `edgeplane daemon msg tail [OPTIONS]`

###### **Options:**

* `--mission <MISSION>`
* `--domain <DOMAIN>`



## `edgeplane daemon attach`

Attach to a running agent, task, or exec (auto-detected)

**Usage:** `edgeplane daemon attach <TARGET>`

###### **Arguments:**

* `<TARGET>`



## `edgeplane daemon watch`

Unified live feed of progress events and messages

**Usage:** `edgeplane daemon watch [OPTIONS]`

###### **Options:**

* `--domain <DOMAIN>`
* `--mission <MISSION>`



## `edgeplane daemon profile`

Manage controlplane profiles (add, list, remove, rename)

**Usage:** `edgeplane daemon profile <COMMAND>`

###### **Subcommands:**

* `add` — Add a controlplane profile. If --join-token is given, registers this node with the controlplane and saves its identity in the profile
* `list` — List saved profiles
* `remove` — Remove a profile (clears active_profile if it was the active one)
* `rename` — Rename a profile (preserves active_profile pointer if needed)
* `show` — Show profile details (auth token is redacted)



## `edgeplane daemon profile add`

Add a controlplane profile. If --join-token is given, registers this node with the controlplane and saves its identity in the profile

**Usage:** `edgeplane daemon profile add [OPTIONS] --url <URL> <NAME>`

###### **Arguments:**

* `<NAME>` — Unique profile name (e.g. "homelab", "work")

###### **Options:**

* `--url <URL>` — Controlplane base URL (e.g. http://edgeplane:8008)
* `--ttl-hours <TTL_HOURS>` — TTL for the OIDC session token in hours (1–8760). Omit to use the server default (8h). Longer values reduce re-auth frequency for edgeplaned
* `--join-token <BOOTSTRAP_TOKEN>` — One-time node join token (from `edgeplane node ... join-token create`). When supplied, this node is registered with the controlplane and its identity (node_id + attach_secret) is saved into the profile
* `--node-name <NODE_NAME>` — Display name for this node (defaults to system hostname)
* `--trust-tier <TRUST_TIER>` — Trust tier label sent at registration (default: "untrusted")

  Default value: `untrusted`
* `--tailscale-fqdn <TAILSCALE_FQDN>` — Tailscale FQDN to register (e.g. epyc.tailnet.ts.net)
* `--activate` — Set this profile as active immediately after adding



## `edgeplane daemon profile list`

List saved profiles

**Usage:** `edgeplane daemon profile list`



## `edgeplane daemon profile remove`

Remove a profile (clears active_profile if it was the active one)

**Usage:** `edgeplane daemon profile remove <NAME>`

###### **Arguments:**

* `<NAME>`



## `edgeplane daemon profile rename`

Rename a profile (preserves active_profile pointer if needed)

**Usage:** `edgeplane daemon profile rename <OLD_NAME> <NEW_NAME>`

###### **Arguments:**

* `<OLD_NAME>`
* `<NEW_NAME>`



## `edgeplane daemon profile show`

Show profile details (auth token is redacted)

**Usage:** `edgeplane daemon profile show <NAME>`

###### **Arguments:**

* `<NAME>`



## `edgeplane daemon use`

Select the active controlplane profile (or show the current one)

**Usage:** `edgeplane daemon use [OPTIONS] [NAME]`

###### **Arguments:**

* `<NAME>` — Profile to activate. Omit to show the currently active profile

###### **Options:**

* `-y`, `--yes` — Restart edgeplaned without prompting after switching. Implies "yes" to the interactive restart prompt. Use in scripts
* `--no-restart` — Don't prompt to restart edgeplaned after switching; just print the command. Mutually exclusive with `--yes`. Useful in CI/scripts that handle the restart themselves



## `edgeplane run`

Launch and manage an agent runtime: claude, codex, gemini, goose, openclaw, custom

**Usage:** `edgeplane run [OPTIONS] <RUNTIME> [ACTION] [-- <PASSTHROUGH>...]`

###### **Arguments:**

* `<RUNTIME>` — Runtime to launch: claude, codex, gemini, goose, openclaw, custom
* `<ACTION>` — Action to perform (default: launch)

  Default value: `launch`

  Possible values:
  - `launch`:
    Launch the runtime (default)
  - `doctor`:
    Inspect and optionally repair runtime readiness
  - `exec`:
    Thin native execution — passes args verbatim to the runtime binary
  - `status`:
    Read-only runtime status (codex only)

* `<PASSTHROUGH>` — Args forwarded verbatim to the runtime binary (after --)

###### **Options:**

* `-p`, `--profile <PROFILE>` — Profile name
* `--new` — Force a new session instead of resuming the last one (launch action)

  Default value: `false`
* `--headless` — Non-interactive mode; fail rather than prompt

  Default value: `false`
* `--domain <DOMAIN>` — Bind to an existing domain — enables mesh participation via SoloSupervisor (launch action)
* `--mission <MISSION>` — Bind to an existing mission (launch action)
* `--task <TASK>` — Bind to an existing task (launch action)
* `--mode <MODE>` — Execution mode (launch action)

  Default value: `interactive`

  Possible values: `interactive`, `headless`, `solo`

* `--fix` — Apply safe deterministic repairs (doctor action)

  Default value: `false`
* `--json` — Emit machine-readable JSON output (doctor/status actions)

  Default value: `false`
* `--with-rtk` — Enable RTK token compression for this agent session (soft: warns if rtk not installed)



## `edgeplane capabilities`

List and describe capability packs available through edgeplaned

**Usage:** `edgeplane capabilities <COMMAND>`

###### **Subcommands:**

* `list` — List available capabilities
* `describe` — Show full schema for a capability



## `edgeplane capabilities list`

List available capabilities

**Usage:** `edgeplane capabilities list [OPTIONS]`

###### **Options:**

* `--tag <TAG>` — Filter by tag (e.g. kubernetes, git)
* `--json` — Output as JSON
* `--route <MODE>` — Routing mode override (auto | local | backend | remote)



## `edgeplane capabilities describe`

Show full schema for a capability

**Usage:** `edgeplane capabilities describe <NAME>`

###### **Arguments:**

* `<NAME>` — The capability name in pack.capability format (e.g. kubectl-observe.kubectl-get-pods)



## `edgeplane exec`

Execute a capability

**Usage:** `edgeplane exec [OPTIONS] <NAME> [-- <ARGS>...]`

###### **Arguments:**

* `<NAME>` — Capability name in pack.capability format (e.g. kubectl-observe.kubectl-get-pods)
* `<ARGS>` — Arguments as key=value pairs or a single JSON string

###### **Options:**

* `--json` — Output as JSON (default when not a TTY)
* `--dry-run` — Validate args without executing
* `--timeout <TIMEOUT>` — Timeout in seconds
* `--domain-id <DOMAIN_ID>` — Domain ID for receipt correlation
* `--agent-id <AGENT_ID>` — Agent ID for receipt correlation
* `--route <ROUTE>` — Override routing mode: auto|local|remote|backend



## `edgeplane receipts`

Inspect capability execution receipts stored in the local SQLite audit log

**Usage:** `edgeplane receipts <COMMAND>`

###### **Subcommands:**

* `last` — Show most recent capability executions
* `get` — Get a specific receipt by ID
* `ls` — List receipts with optional filters



## `edgeplane receipts last`

Show most recent capability executions

**Usage:** `edgeplane receipts last [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`

  Default value: `10`
* `--json`



## `edgeplane receipts get`

Get a specific receipt by ID

**Usage:** `edgeplane receipts get [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>`

###### **Options:**

* `--json`



## `edgeplane receipts ls`

List receipts with optional filters

**Usage:** `edgeplane receipts ls [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`

  Default value: `20`
* `--domain <DOMAIN>`
* `--agent <AGENT>`
* `--json`



## `edgeplane mesh-sync`

Bidirectional git-backed config sync for this node

**Usage:** `edgeplane mesh-sync <COMMAND>`

###### **Subcommands:**

* `pull` — Pull latest config from sync repo (default action)
* `status` — Show sync status
* `push` — Push local node config changes



## `edgeplane mesh-sync pull`

Pull latest config from sync repo (default action)

**Usage:** `edgeplane mesh-sync pull`



## `edgeplane mesh-sync status`

Show sync status

**Usage:** `edgeplane mesh-sync status`



## `edgeplane mesh-sync push`

Push local node config changes

**Usage:** `edgeplane mesh-sync push [OPTIONS]`

###### **Options:**

* `--message <MESSAGE>`

  Default value: `Update node config`



## `edgeplane tui`

Launch the terminal UI (ratatui) for fleet monitoring and management

**Usage:** `edgeplane tui [OPTIONS]`

###### **Options:**

* `--domain <DOMAIN>` — Open the TUI pre-focused on a specific domain by ID



## `edgeplane context`

Manage named controlplane connection contexts

**Usage:** `edgeplane context <COMMAND>`

###### **Subcommands:**

* `list` — List all configured contexts, marking the active one with *
* `current` — Show the active context name and URL
* `use` — Switch the active context
* `add` — Add a new named context
* `remove` — Remove a context (cannot remove the currently active one)
* `discover` — Discover edgeplane-tower nodes and write ~/.edgeplane/servers



## `edgeplane context list`

List all configured contexts, marking the active one with *

**Usage:** `edgeplane context list`



## `edgeplane context current`

Show the active context name and URL

**Usage:** `edgeplane context current`



## `edgeplane context use`

Switch the active context

**Usage:** `edgeplane context use <NAME>`

###### **Arguments:**

* `<NAME>` — Context name to activate



## `edgeplane context add`

Add a new named context

**Usage:** `edgeplane context add [OPTIONS] --url <URL> <NAME>`

###### **Arguments:**

* `<NAME>` — Context name (e.g. "local", "production", "team-alpha")

###### **Options:**

* `--url <URL>` — Controlplane base URL
* `--description <DESCRIPTION>` — Optional human-readable description



## `edgeplane context remove`

Remove a context (cannot remove the currently active one)

**Usage:** `edgeplane context remove <NAME>`

###### **Arguments:**

* `<NAME>` — Context name to remove



## `edgeplane context discover`

Discover edgeplane-tower nodes and write ~/.edgeplane/servers

**Usage:** `edgeplane context discover [OPTIONS]`

###### **Options:**

* `--probe <PROBE>` — Candidate edgeplane-tower URLs to probe (comma-separated or repeated). If omitted, probes the current server list + localhost:8008
* `--dry-run` — Just print what would be written without saving



## `edgeplane domain`

Domain attachment and home-domain management for this agent

**Usage:** `edgeplane domain <COMMAND>`

###### **Subcommands:**

* `home` — Show this agent's home domain
* `attach` — Attach this agent to a domain (sets current_domain_id)
* `detach` — Detach from the current domain and return to the home domain
* `create` — Create a new domain
* `list` — List all visible domains
* `show` — Show a single domain
* `update` — Update a domain's metadata
* `delete` — Delete a domain
* `northstar` — Get or edit the domain's Northstar narrative document



## `edgeplane domain home`

Show this agent's home domain

**Usage:** `edgeplane domain home`



## `edgeplane domain attach`

Attach this agent to a domain (sets current_domain_id)

**Usage:** `edgeplane domain attach <DOMAIN_ID>`

###### **Arguments:**

* `<DOMAIN_ID>` — Domain ID to attach to



## `edgeplane domain detach`

Detach from the current domain and return to the home domain

**Usage:** `edgeplane domain detach`



## `edgeplane domain create`

Create a new domain

**Usage:** `edgeplane domain create [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>` — Domain name

###### **Options:**

* `--description <DESCRIPTION>`
* `--owners <OWNERS>` — Comma-separated owner identities
* `--contributors <CONTRIBUTORS>` — Comma-separated contributor identities
* `--tags <TAGS>` — Comma-separated tags
* `--visibility <VISIBILITY>`
* `--status <STATUS>`



## `edgeplane domain list`

List all visible domains

**Usage:** `edgeplane domain list [OPTIONS]`

###### **Options:**

* `--limit <LIMIT>`

  Default value: `50`



## `edgeplane domain show`

Show a single domain

**Usage:** `edgeplane domain show <ID>`

###### **Arguments:**

* `<ID>` — Domain ID



## `edgeplane domain update`

Update a domain's metadata

**Usage:** `edgeplane domain update [OPTIONS] <ID>`

###### **Arguments:**

* `<ID>` — Domain ID

###### **Options:**

* `--description <DESCRIPTION>`
* `--owners <OWNERS>`
* `--contributors <CONTRIBUTORS>`
* `--tags <TAGS>`
* `--visibility <VISIBILITY>`
* `--status <STATUS>`



## `edgeplane domain delete`

Delete a domain

**Usage:** `edgeplane domain delete <ID>`

###### **Arguments:**

* `<ID>` — Domain ID



## `edgeplane domain northstar`

Get or edit the domain's Northstar narrative document

**Usage:** `edgeplane domain northstar <COMMAND>`

###### **Subcommands:**

* `get` — Print the domain's Northstar document to stdout
* `edit` — Open the domain's Northstar document in $EDITOR and save changes



## `edgeplane domain northstar get`

Print the domain's Northstar document to stdout

**Usage:** `edgeplane domain northstar get [OPTIONS] <DOMAIN_ID>`

###### **Arguments:**

* `<DOMAIN_ID>` — Domain id

###### **Options:**

* `--json` — Emit raw JSON envelope instead of markdown



## `edgeplane domain northstar edit`

Open the domain's Northstar document in $EDITOR and save changes

**Usage:** `edgeplane domain northstar edit <DOMAIN_ID>`

###### **Arguments:**

* `<DOMAIN_ID>` — Domain id



## `edgeplane mission`

Mission (workstream) CRUD — create, list, show, update, delete

**Usage:** `edgeplane mission <COMMAND>`

###### **Subcommands:**

* `create` — Create a new mission
* `list` — List missions, optionally filtered to a domain
* `show` — Show a single mission
* `update` — Update a mission's metadata
* `delete` — Delete a mission
* `brief` — Get or edit the mission's Brief narrative document



## `edgeplane mission create`

Create a new mission

**Usage:** `edgeplane mission create [OPTIONS] --domain-id <DOMAIN_ID> <NAME>`

###### **Arguments:**

* `<NAME>` — Mission name

###### **Options:**

* `--domain-id <DOMAIN_ID>` — Domain this mission belongs to
* `--description <DESCRIPTION>`
* `--owners <OWNERS>` — Comma-separated owner identities
* `--contributors <CONTRIBUTORS>` — Comma-separated contributor identities
* `--tags <TAGS>` — Comma-separated tags
* `--status <STATUS>`
* `--workstream <WORKSTREAM>` — Workstream markdown content



## `edgeplane mission list`

List missions, optionally filtered to a domain

**Usage:** `edgeplane mission list [OPTIONS]`

###### **Options:**

* `--domain-id <DOMAIN_ID>`



## `edgeplane mission show`

Show a single mission

**Usage:** `edgeplane mission show --domain-id <DOMAIN_ID> <ID>`

###### **Arguments:**

* `<ID>` — Mission ID

###### **Options:**

* `--domain-id <DOMAIN_ID>` — Domain this mission belongs to (required — tower only serves domain-scoped paths)



## `edgeplane mission update`

Update a mission's metadata

**Usage:** `edgeplane mission update [OPTIONS] --domain-id <DOMAIN_ID> <ID>`

###### **Arguments:**

* `<ID>` — Mission ID

###### **Options:**

* `--domain-id <DOMAIN_ID>` — Domain this mission belongs to (required — tower only serves domain-scoped paths)
* `--name <NAME>`
* `--description <DESCRIPTION>`
* `--owners <OWNERS>`
* `--contributors <CONTRIBUTORS>`
* `--tags <TAGS>`
* `--status <STATUS>`



## `edgeplane mission delete`

Delete a mission

**Usage:** `edgeplane mission delete --domain-id <DOMAIN_ID> <ID>`

###### **Arguments:**

* `<ID>` — Mission ID

###### **Options:**

* `--domain-id <DOMAIN_ID>` — Domain this mission belongs to (required — tower only serves domain-scoped paths)



## `edgeplane mission brief`

Get or edit the mission's Brief narrative document

**Usage:** `edgeplane mission brief <COMMAND>`

###### **Subcommands:**

* `get` — Print the mission's Brief document to stdout
* `edit` — Open the mission's Brief document in $EDITOR and save changes



## `edgeplane mission brief get`

Print the mission's Brief document to stdout

**Usage:** `edgeplane mission brief get [OPTIONS] <MISSION_ID>`

###### **Arguments:**

* `<MISSION_ID>` — Mission id

###### **Options:**

* `--json` — Emit raw JSON envelope instead of markdown



## `edgeplane mission brief edit`

Open the mission's Brief document in $EDITOR and save changes

**Usage:** `edgeplane mission brief edit <MISSION_ID>`

###### **Arguments:**

* `<MISSION_ID>` — Mission id



## `edgeplane task`

Task CRUD — create, list, show, update, delete

**Usage:** `edgeplane task <COMMAND>`

###### **Subcommands:**

* `create` — Create a new task
* `list` — List tasks for a mission
* `show` — Show a single task
* `update` — Update a task's metadata
* `delete` — Delete a task



## `edgeplane task create`

Create a new task

**Usage:** `edgeplane task create [OPTIONS] --mission-id <MISSION_ID> --domain-id <DOMAIN_ID> <TITLE>`

###### **Arguments:**

* `<TITLE>` — Task title

###### **Options:**

* `--mission-id <MISSION_ID>` — Mission this task belongs to (required)
* `--domain-id <DOMAIN_ID>` — Domain this task belongs to (required — tower only serves domain-scoped paths)
* `--description <DESCRIPTION>`
* `--status <STATUS>`
* `--owner <OWNER>`
* `--contributors <CONTRIBUTORS>` — Comma-separated contributor identities
* `--dod <DOD>` — Definition of done
* `--dependencies <DEPENDENCIES>` — Comma-separated task IDs this task depends on



## `edgeplane task list`

List tasks for a mission

**Usage:** `edgeplane task list --mission-id <MISSION_ID>`

###### **Options:**

* `--mission-id <MISSION_ID>` — Mission ID (required)



## `edgeplane task show`

Show a single task

**Usage:** `edgeplane task show --mission-id <MISSION_ID> --domain-id <DOMAIN_ID> <ID>`

###### **Arguments:**

* `<ID>` — Task ID

###### **Options:**

* `--mission-id <MISSION_ID>`
* `--domain-id <DOMAIN_ID>` — Domain this task belongs to (required — tower only serves domain-scoped paths)



## `edgeplane task update`

Update a task's metadata

**Usage:** `edgeplane task update [OPTIONS] --mission-id <MISSION_ID> --domain-id <DOMAIN_ID> <ID>`

###### **Arguments:**

* `<ID>` — Task ID

###### **Options:**

* `--mission-id <MISSION_ID>`
* `--domain-id <DOMAIN_ID>` — Domain this task belongs to (required — tower only serves domain-scoped paths)
* `--title <TITLE>`
* `--description <DESCRIPTION>`
* `--status <STATUS>`
* `--owner <OWNER>`
* `--contributors <CONTRIBUTORS>`
* `--dod <DOD>`
* `--dependencies <DEPENDENCIES>`



## `edgeplane task delete`

Delete a task

**Usage:** `edgeplane task delete --mission-id <MISSION_ID> --domain-id <DOMAIN_ID> <ID>`

###### **Arguments:**

* `<ID>` — Task ID

###### **Options:**

* `--mission-id <MISSION_ID>`
* `--domain-id <DOMAIN_ID>` — Domain this task belongs to (required — tower only serves domain-scoped paths)



## `edgeplane discover`

Emit the CLI surface as a versioned JSON schema contract; drill into a subtree with [path...]

**Usage:** `edgeplane discover [OPTIONS] [PATH]...`

###### **Arguments:**

* `<PATH>` — Drill into a specific subcommand path (e.g. `agent`, `agent signal`)

###### **Options:**

* `--deep` — Return the full subtree (default: 1 level of subcommands)



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
