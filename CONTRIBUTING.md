# Contributing to DegenView

Thanks for helping. Bug reports, fixes, features and translations are all welcome.

DegenView is a one-person project, so a review can take some days.

## Before you start

- For a large feature or a redesign, open an issue or a discussion first, so the work is not wasted.
- Report security problems privately, never in an issue or a pull request: see
  [SECURITY.md](SECURITY.md).

## Build and check

[BUILDING.md](BUILDING.md) lists what you need. On Linux, the packages CI installs are in
[ci.yml](.github/workflows/ci.yml).

```bash
cargo run
```

Run what CI runs before you push:

```bash
cargo test --locked --lib
```

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --locked --all-targets --features alloc-count,dev-unsealed-mcp,dev-mock-broker -- -D warnings
```

## Pull requests

- Branch from `main` and keep one change per pull request.
- Say what changed, why, and how you checked it. Add a screenshot for a visible change.
- Fix the cause of a bug, not its symptom, and add a test that fails without the fix.
- A new feature or a bug fix that people using the app would notice gets a line in `changes/`
  (`new:` or `fixed:`): see [changes/README.md](changes/README.md).
- Write user-facing text with `tr!`, and translate new or reworded messages in every
  `assets/locales/*.po`: see [Localization](docs/localization.md).

## Where to read more

- [Architecture](docs/architecture.md): module map and data flow.
- [AGENTS.md](AGENTS.md): the project's rules, with a guide for each area. It is written for AI
  coding agents and applies to people just the same.

## License

Contributions are licensed under the same terms as DegenView,
[GPL-3.0-only](LICENSE.md).
