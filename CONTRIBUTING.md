# Contributing to Edgeplane

## Ground Rules
- Be respectful and constructive.
- Keep changes scoped and testable.
- Include docs updates for user-facing behavior changes.

## Development Workflow
1. Fork and create a feature branch.
2. Implement changes with tests.
3. Run local checks relevant to changed areas.
4. Open a pull request using the PR template.

## Required Sign-off (DCO)
All commits must include a Signed-off-by trailer:

```text
Signed-off-by: Your Name <you@example.com>
```

Use:

```bash
git commit -s -m "your message"
```

By signing off, you certify the Developer Certificate of Origin (DCO).

## Pull Request Expectations
- Explain problem, approach, and risk.
- Add or update tests for behavioral changes.
- Highlight migrations, config changes, or operational impact.
- Keep PRs focused; large refactors should be staged.

## Commit Message Guidance
Use clear, imperative messages. Example:

```text
mcp: add idempotency check for create_mission
```

## Security-Sensitive Changes
If a change impacts auth, policy, secrets, or tenant isolation, call it out in PR
notes and ensure security tests are included.
