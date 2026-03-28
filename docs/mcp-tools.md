# ddak MCP Reference

This document describes the current MCP stdio surface exposed by `ddak mcp serve`.

## Start the MCP server

```bash
ddak mcp serve --state-file .ddak/poc-state.json
```

The server reads JSON-RPC messages from stdin and writes JSON-RPC responses to stdout.

## Supported MCP methods

- `initialize`
- `notifications/initialized`
- `tools/list`
- `tools/call`
- `resources/list`
- `resources/read`

## Tool catalog (curated)

MCP is intentionally minimal. CLI remains the primary surface for advanced and bulk operations.

Issue tools:

- `issue_get` (`issue`)
- `issue_create` (`title`, `project?`)
- `issue_move` (`issue`, `status`)
- `issue_assign_project` (`issue`, `project`)

Project tools:

- `project_get` (`project`)
- `project_create` (`name`, `key?`)
- `project_set_key` (`project`, `key`)

Comment tools:

- `comment_add` (`entity_type`, `entity`, `body_markdown`, `author?`)
- `comment_list` (`entity_type`, `entity`, `all?`, `cursor?`, `limit?`, `order?`)

Deprecated MCP tools (CLI-first policy):

- `issue_list`, `issue_set_cwd`, `issue_clear_cwd`, `issue_delete`
- `project_list`, `project_set_repo_path`, `project_clear_repo_path`

These now return an MCP error directing users to the corresponding `ddak` CLI subcommands.

Notes:

- `issue` values accept either internal `issue_id` or human key like `DEV-0001`.
- `project` values accept internal `project_id`, project key (e.g. `DEV`), or exact project name.
- Mutating tools persist state to the configured `--state-file`.
- For read-heavy workflows, use resources (`ddak://issues`, `ddak://projects`) rather than expanding tool count.

## Resources

Available resources via `resources/list`:

- `ddak://projects` -> all projects
- `ddak://issues` -> all issues
- `ddak://health` -> health/version/capabilities snapshot
- `ddak://issues/{issue}/comments` -> comments for issue id/key
- `ddak://projects/{project}/comments` -> comments for project id/key/name

All resources are returned as `application/json` text payloads.

## Example requests

Initialize:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
```

List tools:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
```

Create issue:

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "issue_create",
    "arguments": {
      "title": "Add MCP docs",
      "project": "DEV"
    }
  }
}
```

Read health resource:

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "resources/read",
  "params": {
    "uri": "ddak://health"
  }
}
```
