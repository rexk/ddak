#!/usr/bin/env python3
"""Export `ddak` issues into legacy markdown ticket files and board layout."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path


STATUSES = ["backlog", "ready", "in_progress", "review", "done", "blocked"]


def run(cmd: list[str], cwd: Path) -> str:
    proc = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(cmd)}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
        )
    return proc.stdout


def run_ddak(ddak_bin: str, repo_root: Path, args: list[str]) -> str:
    return run([ddak_bin, *args], repo_root).strip()


def slugify(name: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return slug or "ticket"


def parse_legacy_id_and_title(
    title: str, fallback_identifier: str | None
) -> tuple[str, str]:
    m = re.match(r"^\[(TAO-\d{3})\]\s+(.+)$", title.strip())
    if m:
        return m.group(1), m.group(2)
    if fallback_identifier:
        return fallback_identifier, title
    return "TAO-XXX", title


def migration_comment_body(comments_page: dict) -> str | None:
    items = comments_page.get("items", [])
    for item in reversed(items):
        if item.get("author") == "migration" and item.get("body_markdown"):
            return item["body_markdown"]
    return None


def synthesize_ticket_markdown(legacy_id: str, issue: dict) -> str:
    title = issue.get("title", "").strip()
    _, clean_title = parse_legacy_id_and_title(title, issue.get("identifier"))
    status = issue.get("status", "backlog")
    return "\n".join(
        [
            f"# {legacy_id}: {clean_title}",
            "",
            f"- ID: `{legacy_id}`",
            f"- Status: `{status}`",
            "- Priority: `P2`",
            "- Depends on: _(none)_",
            "",
            "## Problem",
            "",
            "Describe the problem.",
            "",
            "## Scope",
            "",
            "- Define scope.",
            "",
            "## Out of Scope",
            "",
            "- Define non-goals.",
            "",
            "## Acceptance Criteria",
            "",
            "- Define acceptance criteria.",
            "",
            "## Verification",
            "",
            "- `cargo fmt --all --check`",
            "- `cargo clippy --workspace --all-targets -- -D warnings`",
            "- `cargo test --workspace`",
            "",
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Export ddak issues to legacy markdown tickets"
    )
    parser.add_argument("--state-file", default=".ddak/tickets.duckdb")
    parser.add_argument("--out-dir", default="tickets")
    parser.add_argument("--ddak-bin", default="target/debug/ddak")
    parser.add_argument("--project", default="TAO")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    out_dir = (repo_root / args.out_dir).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    ddak_bin = args.ddak_bin
    if not Path(ddak_bin).is_absolute():
        ddak_bin = str((repo_root / ddak_bin).resolve())
    state_file = str((repo_root / args.state_file).resolve())

    issues_raw = run_ddak(
        ddak_bin,
        repo_root,
        [
            "issue",
            "--state-file",
            state_file,
            "--output",
            "json",
            "list",
            "--project",
            args.project,
        ],
    )
    issues = json.loads(issues_raw or "[]")

    board_rows: dict[str, list[tuple[str, str]]] = {status: [] for status in STATUSES}
    written = 0

    for issue in sorted(issues, key=lambda i: i.get("identifier") or ""):
        legacy_id, clean_title = parse_legacy_id_and_title(
            issue.get("title", ""), issue.get("identifier")
        )
        issue_ref = issue.get("identifier") or issue.get("id")
        comments_raw = run_ddak(
            ddak_bin,
            repo_root,
            [
                "issue",
                "--state-file",
                state_file,
                "--output",
                "json",
                "comment-list",
                issue_ref,
                "--all",
            ],
        )
        comments_page = json.loads(comments_raw or "{}")
        body = migration_comment_body(comments_page)
        if not body:
            body = synthesize_ticket_markdown(legacy_id, issue)

        file_name = f"{legacy_id}-{slugify(clean_title)}.md"
        (out_dir / file_name).write_text(body, encoding="utf-8")
        written += 1

        status = issue.get("status", "backlog")
        if status not in board_rows:
            status = "backlog"
        board_rows[status].append((legacy_id, clean_title))

    board_lines = [
        "# Board",
        "",
        "This board tracks the canonical issue workflow:",
        "",
        "- `backlog`",
        "- `ready`",
        "- `in_progress`",
        "- `review`",
        "- `done`",
        "- `blocked`",
        "",
    ]
    for status in STATUSES:
        board_lines.append(f"## {status}")
        board_lines.append("")
        items = board_rows[status]
        if not items:
            board_lines.append("- _(empty)_")
        else:
            for legacy_id, clean_title in sorted(items):
                board_lines.append(f"- `{legacy_id}` {clean_title}")
        board_lines.append("")
    (out_dir / "BOARD.md").write_text(
        "\n".join(board_lines).rstrip() + "\n", encoding="utf-8"
    )

    print(f"state-file: {state_file}")
    print(f"out-dir: {out_dir}")
    print(f"files-written: {written}")
    print("board-written: BOARD.md")


if __name__ == "__main__":
    main()
