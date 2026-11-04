# Security policy

## Reporting a vulnerability

Please report it privately through
[GitHub's private vulnerability reporting](https://github.com/degenview/degenview/security/advisories/new)
(Security tab, "Report a vulnerability"). Do not open a public issue or discussion for it.

Include what you found, how to reproduce it, and the version (Settings > About). You will get a
reply as soon as the maintainer can; this is a one-person project, so please allow some days.

## What counts

- Anything that lets someone run code on a user's machine or read their keys, portfolio or API
  tokens: the app, its MCP server, Pine scripts, imported files, the auto-updater.
- Anything that lets someone put a file in front of the updater, or into a release.
- Weaknesses in how releases are built and published (the workflows in `.github/`, `tools/`).

## How releases are protected

Every release is built by a GitHub Actions workflow from a signed tag on `main`, published as an
immutable GitHub release with a build provenance attestation. Check a download with:

    gh attestation verify <file> --repo degenview/degenview

macOS builds are also signed with a Developer ID and notarized by Apple. The Windows and Linux
builds are not code-signed; the attestation and the release's SHA-256 digest are what bind them
to the workflow run. Details: [docs/agent-guides/releases-updates.md](docs/agent-guides/releases-updates.md).

Dependencies are checked for known vulnerabilities, yanked versions and unexpected sources on every
pull request (`deny.toml`).
