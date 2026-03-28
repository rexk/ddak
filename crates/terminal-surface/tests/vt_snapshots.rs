use terminal_surface::{CellAttr, CellColor, ScreenGrid};

// ── Helpers ──────────────────────────────────────────────────────────

fn grid_row_text(grid: &ScreenGrid, row: u16) -> String {
    grid.get_row(row)
        .map(|cells| cells.iter().map(|c| c.char).collect::<String>())
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

fn screen_dump_with_attrs(grid: &ScreenGrid) -> String {
    let mut out = String::new();
    for row in 0..grid.rows() {
        if let Some(cells) = grid.get_row(row) {
            for (col, cell) in cells.iter().enumerate() {
                if cell.char != ' ' || cell.attr != CellAttr::default() {
                    out.push_str(&format!(
                        "[r{}c{} '{}' fg={:?} bg={:?} bold={} italic={} ul={}]\n",
                        row,
                        col,
                        cell.char,
                        cell.attr.fg,
                        cell.attr.bg,
                        cell.attr.bold,
                        cell.attr.italic,
                        cell.attr.underline,
                    ));
                }
            }
        }
    }
    out
}

// ── SGR attribute tests ──────────────────────────────────────────────

#[test]
fn sgr_bold() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[1mBold\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.char, 'B');
    assert!(
        cell.attr.bold,
        "expected bold\n{}",
        screen_dump_with_attrs(&grid)
    );

    // After reset, new chars should not be bold
    grid.process(b"X");
    let cell = grid.get_cell(0, 4).unwrap();
    assert!(!cell.attr.bold);
}

#[test]
fn sgr_italic() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[3mItalic\x1b[23m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert!(cell.attr.italic);
    let cell = grid.get_cell(0, 5).unwrap();
    assert!(cell.attr.italic);
}

#[test]
fn sgr_underline() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[4mUnder\x1b[24m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert!(cell.attr.underline);
}

#[test]
fn sgr_foreground_colors() {
    let mut grid = ScreenGrid::new(20, 5);
    // Red foreground (color index 1)
    grid.process(b"\x1b[31mR\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.char, 'R');
    assert_eq!(
        cell.attr.fg,
        CellColor::Indexed(1),
        "expected red fg\n{}",
        screen_dump_with_attrs(&grid)
    );
}

#[test]
fn sgr_256_color() {
    let mut grid = ScreenGrid::new(20, 5);
    // 256-color: ESC[38;5;214m (orange-ish)
    grid.process(b"\x1b[38;5;214mX\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(
        cell.attr.fg,
        CellColor::Indexed(214),
        "256-color should be parsed correctly"
    );
}

#[test]
fn sgr_background_color() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[42mG\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.attr.bg, CellColor::Indexed(2)); // green bg
}

#[test]
fn sgr_bright_colors() {
    let mut grid = ScreenGrid::new(20, 5);
    // Bright red fg = 91 → index 9
    grid.process(b"\x1b[91mB\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.attr.fg, CellColor::Indexed(9));
}

#[test]
fn sgr_reset_clears_all() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[1;3;4;31mX\x1b[0mY");

    let x = grid.get_cell(0, 0).unwrap();
    assert!(x.attr.bold);
    assert!(x.attr.italic);
    assert!(x.attr.underline);
    assert_eq!(x.attr.fg, CellColor::Indexed(1));

    let y = grid.get_cell(0, 1).unwrap();
    assert!(!y.attr.bold);
    assert!(!y.attr.italic);
    assert!(!y.attr.underline);
    assert_eq!(y.attr.fg, CellColor::Default);
}

// ── Erase operations ─────────────────────────────────────────────────

#[test]
fn erase_screen_mode_2() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"Hello");
    grid.process(b"\x1b[2J");

    assert_eq!(grid_row_text(&grid, 0), "");
}

#[test]
fn erase_line_mode_0() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"ABCDEFGHIJ");
    // Move cursor to col 3, erase from cursor to end
    grid.process(b"\x1b[1;4H\x1b[K");

    let row = grid_row_text(&grid, 0);
    assert_eq!(row, "ABC");
}

#[test]
fn erase_line_mode_1() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"ABCDEFGHIJ");
    // Move cursor to col 5 (1-indexed=6), erase from start to cursor
    grid.process(b"\x1b[1;6H\x1b[1K");

    let row = grid_row_text(&grid, 0);
    // Cols 0-5 should be spaces, cols 6-9 remain
    assert!(row.starts_with("      ") || row.trim_start().starts_with("G"));
}

#[test]
fn erase_above_cursor() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"Line0\r\nLine1\r\nLine2");
    // Move to row 2, col 0; erase above (mode 1)
    grid.process(b"\x1b[3;1H\x1b[1J");

    // Row 0 and 1 should be cleared
    assert_eq!(grid_row_text(&grid, 0), "");
    assert_eq!(grid_row_text(&grid, 1), "");
}

// ── Cursor addressing ────────────────────────────────────────────────

#[test]
fn cursor_absolute_positioning() {
    let mut grid = ScreenGrid::new(20, 10);
    // Move to row 5, col 10 (1-indexed)
    grid.process(b"\x1b[5;10HX");

    let cell = grid.get_cell(4, 9).unwrap();
    assert_eq!(cell.char, 'X');
}

// ── Line insert/delete ───────────────────────────────────────────────

#[test]
fn line_insert() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"AAA\r\nBBB\r\nCCC\r\nDDD\r\nEEE");
    // Move to row 2 (1-indexed), insert a line
    grid.process(b"\x1b[2;1H\x1b[L");

    let content = grid.get_content_trimmed();
    assert_eq!(content[0], "AAA");
    assert_eq!(content[1], ""); // inserted blank line
    assert_eq!(content[2], "BBB");
    assert_eq!(content[3], "CCC");
    assert_eq!(content[4], "DDD");
    // EEE scrolled off
}

#[test]
fn line_delete() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"AAA\r\nBBB\r\nCCC\r\nDDD\r\nEEE");
    // Move to row 2, delete a line
    grid.process(b"\x1b[2;1H\x1b[M");

    let content = grid.get_content_trimmed();
    assert_eq!(content[0], "AAA");
    assert_eq!(content[1], "CCC"); // BBB deleted, lines shift up
    assert_eq!(content[2], "DDD");
    assert_eq!(content[3], "EEE");
    assert_eq!(content[4], ""); // blank line at bottom
}

// ── Scroll regions ───────────────────────────────────────────────────

#[test]
fn scroll_region_isolates_scrolling() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"Row0\r\nRow1\r\nRow2\r\nRow3\r\nRow4");
    // Set scroll region to rows 2-4 (1-indexed)
    grid.process(b"\x1b[2;4r");
    // Move to row 4 (bottom of region) and add a new line to trigger scroll
    grid.process(b"\x1b[4;1H");
    grid.process(b"\nNEW");

    let content = grid.get_content_trimmed();
    // Row 0 should be untouched (outside scroll region)
    assert_eq!(content[0], "Row0");
    // Row 4 should be untouched (outside scroll region)
    assert_eq!(content[4], "Row4");
}

// ── Cursor save/restore ──────────────────────────────────────────────

#[test]
fn cursor_save_restore() {
    let mut grid = ScreenGrid::new(20, 10);
    grid.process(b"\x1b[5;10H"); // move to (4,9)
    grid.process(b"\x1b[s"); // save
    grid.process(b"\x1b[1;1H"); // move to (0,0)
    grid.process(b"\x1b[u"); // restore

    assert_eq!(grid.cursor(), (4, 9));
}

// ── Tab stops ────────────────────────────────────────────────────────

#[test]
fn tab_advances_to_next_8_column_boundary() {
    let mut grid = ScreenGrid::new(40, 5);
    grid.process(b"AB\tX");

    // AB at cols 0-1, tab to col 8, X at col 8
    let cell = grid.get_cell(0, 8).unwrap();
    assert_eq!(cell.char, 'X', "tab should advance to col 8");
}

// ── Wrap behavior ────────────────────────────────────────────────────

#[test]
fn printing_at_last_column_does_not_advance_past_grid() {
    let mut grid = ScreenGrid::new(5, 3);
    grid.process(b"ABCDE");

    // Cursor should stay at last column (4), not advance to 5
    let (row, col) = grid.cursor();
    assert_eq!(row, 0);
    assert_eq!(col, 4, "cursor should clamp at last column");

    // All 5 chars should be placed
    assert_eq!(grid_row_text(&grid, 0), "ABCDE");
}

// ── Scroll from output overflow ──────────────────────────────────────

#[test]
fn output_overflow_scrolls_content() {
    let mut grid = ScreenGrid::new(10, 3);
    grid.process(b"Line0\r\nLine1\r\nLine2\r\nLine3");

    let content = grid.get_content_trimmed();
    // Line0 should have scrolled off, bottom row has Line3
    assert_eq!(content[2], "Line3");
    // Line0 should be gone
    assert_ne!(content[0], "Line0");
}

// ── Parser persistence across process() calls ──────────────────────

#[test]
fn split_escape_sequence_across_process_calls() {
    let mut grid = ScreenGrid::new(20, 5);
    // Split \x1b[38;5;214m across two process() calls
    grid.process(b"\x1b[38;5");
    grid.process(b";214mX");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.char, 'X');
    assert_eq!(
        cell.attr.fg,
        CellColor::Indexed(214),
        "256-color should parse correctly across split process() calls"
    );
}

// ── Truecolor (RGB) ─────────────────────────────────────────────────

#[test]
fn sgr_truecolor_fg() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[38;2;255;128;0mX\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.attr.fg, CellColor::Rgb(255, 128, 0));
}

#[test]
fn sgr_truecolor_bg() {
    let mut grid = ScreenGrid::new(20, 5);
    grid.process(b"\x1b[48;2;0;128;255mX\x1b[0m");

    let cell = grid.get_cell(0, 0).unwrap();
    assert_eq!(cell.attr.bg, CellColor::Rgb(0, 128, 255));
}

// ── Alt-screen CSI handling ─────────────────────────────────────────

#[test]
fn alt_screen_csi_enter_exit() {
    let mut grid = ScreenGrid::new(10, 5);
    grid.process(b"Hello");
    // Enter alt screen via CSI ?1049h
    grid.process(b"\x1b[?1049h");
    grid.process(b"Alt");

    let content = grid.get_content_trimmed();
    assert_eq!(content[0], "Alt");

    // Exit alt screen via CSI ?1049l
    grid.process(b"\x1b[?1049l");
    let content = grid.get_content_trimmed();
    assert_eq!(content[0], "Hello");
}

// ── Diff computation ─────────────────────────────────────────────────

#[test]
fn diff_detects_changed_cells() {
    let mut prev = ScreenGrid::new(10, 3);
    prev.process(b"Hello");

    let mut curr = prev.clone();
    curr.process(b"\x1b[1;1HWorld");

    let diffs = curr.compute_diff(&prev);
    assert!(!diffs.is_empty(), "should detect differences");
    assert_eq!(diffs[0].row_start, 0);
}
