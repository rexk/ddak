# Naming Decision

## Final Choice: `ddak`

**Binary**: `ddak`
**State directory**: `.ddak/`
**Origin**: Korean onomatopoeia 딱 (ddak) — the sound of snapping into place, doing things decisively and exactly right. From the expression "일을 탁탁/딱딱 한다" — getting work done briskly, one thing after another.

**Why it works**:
- 4 characters, easy to type
- Completely uncontested: available on crates.io, npm, PyPI, Homebrew
- No CLI binary conflicts anywhere
- Distinctive double consonant — impossible to confuse with existing tools
- Captures the tool's purpose: snapping agents, issues, and sessions into place

## Previous Identity

- **TAO** — Terminal Agent Orchestrator (project codename)
- **toa** — Team of Agents (CLI binary)
- Renamed because: `toa` is taken on crates.io (compression CLI tool), and TAO/toa had no strong brand identity

## CLI Pattern

Bare `ddak` launches the TUI (industry standard for TUI+CLI hybrids like OpenCode, lazygit, k9s). Subcommands provide scripted access:

- `ddak` — launch TUI
- `ddak issue list` — CLI issue management
- `ddak project list` — CLI project management
- `ddak mcp serve` — MCP server mode
- `ddak --no-ui` — headless/CI mode

## Rejected Candidates

Short English words (3-4 chars) are nearly all taken across crates.io, npm, and existing CLI tools. Notable conflicts:

| Name | Blocker |
|------|---------|
| bay, den, pit | Weak descriptiveness, crate squatting |
| hub | GitHub's hub tool |
| helm | Kubernetes Helm |
| ark | KDE desktop app + arkworks crate ecosystem |
| hive | Apache Hive |
| nest | NestJS `nest` binary |
| dash | Debian Almquist Shell (`/bin/sh` on Ubuntu) |
| loom | tokio-rs crate |
| deck | Kong decK CLI |
| relay | Facebook Relay + Sentry Relay |
| aboard | aboard.com company (Paul Ford / Rich Ziade) |
| tak | crates.io taken + US military TAK ecosystem (tak.gov) |
| toa | crates.io taken (compression CLI) |
| fore, mast | Available but don't describe what the tool does |
