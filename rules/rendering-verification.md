# Rendering Verification Workflow

LLM-executable procedure for verifying rendering changes. Follow this after any change to `terminal-surface`, `tui-app` rendering, or related code.

## A. Iteration loop

After every rendering change:

1. Run: `cargo test -p terminal-surface`
2. If all pass → done.
3. If a round-trip assertion fails (cell-by-cell mismatch in `assert_round_trip`) → this is always a bug. Fix the parser or renderer.
4. If only insta snapshot mismatches → read the `.snap.new` files, decide accept or reject (see section C).

## B. Accepting snapshots (non-interactive)

Never use `cargo insta review` — it launches an interactive TUI that LLM agents cannot operate.

**Primary method** — re-run tests with auto-accept:

```bash
INSTA_UPDATE=always cargo test -p terminal-surface --test visual_snapshots
```

**Fallback** — manually promote pending snapshots:

```bash
for f in crates/terminal-surface/tests/snapshots/*.snap.new; do mv "$f" "${f%.new}"; done
```

To generate initial snapshots for new tests without overwriting existing ones:

```bash
INSTA_UPDATE=new cargo test -p terminal-surface --test visual_snapshots
```

## C. Reading serialize() diffs

Snapshot diffs use the `serialize()` format. Here is how to interpret common changes:

**Text content changed** (row content differs):
```diff
-  0: "Hello"
+  0: "World"
```

**Attribute changed** (bold, italic, etc.):
```diff
-  0: attrs [0..5]=bold
+  0: attrs [0..5]=bold,italic
```

**Color changed:**
```diff
-  0: attrs [0..5]=fg:Idx(1)
+  0: attrs [0..5]=fg:Rgb(255,0,0)
```

**Decision rule:** If every changed line in the snapshot corresponds to a change you intentionally made → accept. If any line changed that you did not intend → it is a regression, fix it before accepting.

## D. Creating .vt fixtures

The `Write` tool cannot produce raw ESC (`\x1b`) bytes. You must use Bash `printf`:

```bash
printf '\x1b[1;31mBold Red\x1b[0m' > crates/terminal-surface/test-fixtures/my_feature.vt
```

Verify the file contains real escape bytes:

```bash
xxd crates/terminal-surface/test-fixtures/my_feature.vt | head
```

You should see `1b5b` for ESC + `[`.

## E. Adding a new snapshot test

1. Create a `.vt` fixture using `printf` (see section D).
2. Add a test function in `visual_snapshots.rs` that calls `parse_fixture()` + `assert_round_trip()`.
3. Generate initial snapshots:
   ```bash
   INSTA_UPDATE=new cargo test -p terminal-surface --test visual_snapshots
   ```
4. Read the generated `.snap` files to confirm they are correct.
5. Commit the `.snap` files alongside the test and fixture.

## F. What to run when

| What changed | Run |
|---|---|
| `terminal-surface/src/lib.rs` (parser/renderer) | `cargo test -p terminal-surface` |
| `tui-app` rendering code | `cargo test -p tui-app --test render_pipeline --test visual_snapshots` |
| Both | `cargo test --workspace` |
