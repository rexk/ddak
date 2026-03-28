# Work Tracking with `ddak`

Work tracking in this repository is managed with the product itself via `ddak issue` and `ddak project`.

## Canonical State File

- `.ddak/tickets.duckdb`

Use `--state-file .ddak/tickets.duckdb` for explicit control, or omit `--state-file` to use the default.

## Core Commands

List projects:

```bash
ddak project --state-file .ddak/tickets.duckdb list
```

List issues:

```bash
ddak issue --state-file .ddak/tickets.duckdb list
```

Create issue:

```bash
ddak issue --state-file .ddak/tickets.duckdb create --title "Example title" --project DDAK
```

Move issue status:

```bash
ddak issue --state-file .ddak/tickets.duckdb move DDAK-0001 --status in_progress
```

Add issue progress note:

```bash
ddak issue --state-file .ddak/tickets.duckdb comment-add DDAK-0001 --body "Started implementation"
```

List issue notes:

```bash
ddak issue --state-file .ddak/tickets.duckdb comment-list DDAK-0001 --all
```

## Status Vocabulary

- `backlog`
- `ready`
- `in_progress`
- `review`
- `done`
- `blocked`

## Migration Note

Legacy markdown ticket files were migrated into `ddak` issues with titles prefixed by the legacy ticket ID.

## Migration Scripts

One-off migration from legacy markdown files to `ddak`:

```bash
python scripts/migrate_tickets_to_ddak.py --state-file .ddak/tickets.duckdb --git-ref HEAD~1
```

If legacy files are still present in the working tree, omit `--git-ref`.

Re-export `ddak` issues back into legacy markdown format:

```bash
python scripts/export_to_markdown_tickets_ddak.py --state-file .ddak/tickets.duckdb --out-dir tickets
```
