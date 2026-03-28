#!/usr/bin/env python3
"""Migrate legacy markdown tickets into `ddak` issue tracking.

Supports reading from the working tree or from a historical git ref.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path


VALID_STATUSES = {"backlog", "ready", "in_progress", "review", "done", "blocked"}


def run(cmd: list[str], cwd: Path) -> str:
    proc = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(cmd)}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
        )
    return proc.stdout


def run_ddak(ddak_bin: str, repo_root: Path, args: list[str]) -> str:
    return run([ddak_bin, *args], repo_root).strip()


def read_source_file(repo_root: Path, file_path: str, git_ref: str | None) -> str:
    if git_ref:
        return run(["git", "show", f"{git_ref}:{file_path}"], repo_root)
    return (repo_root / file_path).read_text(encoding="utf-8")


def list_ticket_paths(
    repo_root: Path, tickets_dir: str, git_ref: str | None
) -> list[str]:
    if git_ref:
        out = run(
            ["git", "ls-tree", "-r", "--name-only", git_ref, tickets_dir], repo_root
        )
        return sorted(
            p.strip()
            for p in out.splitlines()
            if re.match(rf"^{re.escape(tickets_dir)}/TAO-\d{{3}}-.*\.md$", p.strip())
        )

    base = repo_root / tickets_dir
    return sorted(str(p.relative_to(repo_root)) for p in base.glob("TAO-*.md"))


def parse_board_status_map(board_md: str) -> dict[str, str]:
    result: dict[str, str] = {}
    status = None
    for raw in board_md.splitlines():
        line = raw.strip()
        m = re.match(r"^##\s+([a-z_]+)$", line)
        if m:
            maybe = m.group(1)
            status = maybe if maybe in VALID_STATUSES else None
            continue
        m = re.search(r"`(TAO-\d{3})`", line)
        if m and status:
            result[m.group(1)] = status
    return result


def parse_ticket_identity(md_text: str, file_path: str) -> tuple[str, str]:
    file_name = Path(file_path).name
    m = re.match(r"(TAO-\d{3})-", file_name)
    if not m:
        raise RuntimeError(f"cannot parse TAO id from {file_path}")
    legacy_id = m.group(1)

    header = md_text.splitlines()[0] if md_text.splitlines() else legacy_id
    header = header.lstrip("#").strip()
    if ":" in header:
        header = header.split(":", 1)[1].strip()
    return legacy_id, header


def ensure_project(
    ddak_bin: str, repo_root: Path, state_file: str, key: str, name: str
) -> None:
    projects_raw = run_ddak(
        ddak_bin,
        repo_root,
        ["project", "--state-file", state_file, "--output", "json", "list"],
    )
    projects = json.loads(projects_raw or "[]")
    if any((p.get("identifier") or "").upper() == key.upper() for p in projects):
        return
    run_ddak(
        ddak_bin,
        repo_root,
        [
            "project",
            "--state-file",
            state_file,
            "--output",
            "json",
            "create",
            "--name",
            name,
            "--key",
            key,
        ],
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Migrate legacy tickets into ddak issues"
    )
    parser.add_argument("--state-file", default=".ddak/tickets.duckdb")
    parser.add_argument("--tickets-dir", default="tickets")
    parser.add_argument("--board-file", default="tickets/BOARD.md")
    parser.add_argument("--project-key", default="TAO")
    parser.add_argument("--project-name", default="Terminal Agent Orchestrator")
    parser.add_argument("--ddak-bin", default="target/debug/ddak")
    parser.add_argument("--author", default="migration")
    parser.add_argument(
        "--git-ref",
        default=None,
        help="Read ticket markdown from this git ref (example: HEAD~1)",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    state_file = str((repo_root / args.state_file).resolve())
    ddak_bin = args.ddak_bin
    if not os.path.isabs(ddak_bin):
        ddak_bin = str((repo_root / ddak_bin).resolve())

    ensure_project(ddak_bin, repo_root, state_file, args.project_key, args.project_name)

    board_text = read_source_file(repo_root, args.board_file, args.git_ref)
    status_map = parse_board_status_map(board_text)
    ticket_paths = list_ticket_paths(repo_root, args.tickets_dir, args.git_ref)

    issues_raw = run_ddak(
        ddak_bin,
        repo_root,
        ["issue", "--state-file", state_file, "--output", "json", "list"],
    )
    issues = json.loads(issues_raw or "[]")
    by_title = {i.get("title", ""): i for i in issues}

    created = 0
    moved = 0
    commented = 0

    for ticket_path in ticket_paths:
        md_text = read_source_file(repo_root, ticket_path, args.git_ref)
        legacy_id, parsed_title = parse_ticket_identity(md_text, ticket_path)
        issue_title = f"[{legacy_id}] {parsed_title}"

        issue = by_title.get(issue_title)
        if issue is None:
            created_raw = run_ddak(
                ddak_bin,
                repo_root,
                [
                    "issue",
                    "--state-file",
                    state_file,
                    "--output",
                    "json",
                    "create",
                    "--title",
                    issue_title,
                    "--project",
                    args.project_key,
                ],
            )
            issue = json.loads(created_raw)
            by_title[issue_title] = issue
            created += 1

        issue_ref = issue.get("identifier") or issue.get("id")
        target_status = status_map.get(legacy_id)
        if target_status and issue.get("status") != target_status:
            run_ddak(
                ddak_bin,
                repo_root,
                [
                    "issue",
                    "--state-file",
                    state_file,
                    "--output",
                    "json",
                    "move",
                    issue_ref,
                    "--status",
                    target_status,
                ],
            )
            moved += 1

        page_raw = run_ddak(
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
        page = json.loads(page_raw or "{}")
        if page.get("items"):
            continue

        with tempfile.NamedTemporaryFile("w", delete=False, encoding="utf-8") as tf:
            tf.write(md_text)
            temp_path = tf.name
        try:
            run_ddak(
                ddak_bin,
                repo_root,
                [
                    "issue",
                    "--state-file",
                    state_file,
                    "comment-add",
                    issue_ref,
                    "--body-file",
                    temp_path,
                    "--author",
                    args.author,
                ],
            )
            commented += 1
        finally:
            Path(temp_path).unlink(missing_ok=True)

    total = json.loads(
        run_ddak(
            ddak_bin,
            repo_root,
            ["issue", "--state-file", state_file, "--output", "json", "list"],
        )
        or "[]"
    )
    print(f"state-file: {state_file}")
    print(f"created: {created}")
    print(f"status-updated: {moved}")
    print(f"comments-added: {commented}")
    print(f"total-issues: {len(total)}")


if __name__ == "__main__":
    main()
