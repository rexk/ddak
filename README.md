# ddak

Terminal agent orchestrator for managing interactive OpenCode/Claude Code sessions and issue progress.

## Status

Planning and bootstrap phase.

## Repo Layout

- `PROJECT_SPEC.md` - product and architecture spec
- `docs/work-tracking.md` - how to manage work with `ddak` issues
- `crates/` - Rust workspace crates

## Development Environment

This project standardizes on `devenv` + `direnv` for reproducible setup.

Prerequisites:

- Nix
- direnv
- devenv

Install devenv if needed:

```bash
nix profile install nixpkgs#devenv
```

Enable environment in this repo:

```bash
direnv allow
```

Then run:

```bash
devenv shell
cargo check
```

Notes:

- `.envrc` enforces `devenv` presence.
- Rust toolchain is pinned via `rust-toolchain.toml` and consumed by `devenv`.
