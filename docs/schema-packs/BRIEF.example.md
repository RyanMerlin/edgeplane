# BRIEF: <mission-name>

## Purpose
- Mission ID: `<mission-id>`
- Domain ID: `<domain-id>`
- Description: <human-readable mission objective and targeted outcome>

## Governance
- Owners: <comma-separated owners>
- Contributors: <comma-separated contributors>
- Rules: <mission-specific rules>
- Allowed Actions: <mission-specific allow/deny policy>

## Policy Overlay
- Inherits Domain Policy: yes|no
- Override Scope: <explicit overrides from NORTHSTAR>

## External Storage
- Object Prefix: `domains/<domain-id>/missions/<mission-id>/`
- Credential Refs: `secretref://infisical/<project>/<path>#<key>` (no plaintext)

## Integrations
- Tools/Connections: <refs>
- Auth Refs: <secret refs only>

## Data Sources
- Databases/APIs: <refs and usage constraints>

## Agent Runtime
- Agent Profiles: <profile names>
- AGENT.md Refs: <paths/ids>
- Required Capabilities: <capability list>

## Versioning
- Version: 1
- Created By: <subject>
- Modified By: <subject>
- Change Summary: <what changed>
