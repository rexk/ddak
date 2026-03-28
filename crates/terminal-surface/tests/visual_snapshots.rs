use terminal_surface::ScreenGrid;

fn parse_fixture(fixture: &[u8], cols: u16, rows: u16) -> ScreenGrid {
    let mut grid = ScreenGrid::new(cols, rows);
    grid.process(fixture);
    grid
}

fn assert_round_trip(name: &str, parsed: &ScreenGrid, cols: u16) {
    let rendered_lines = parsed.render_to_sgr(cols);
    let mut round_trip = ScreenGrid::new(cols, parsed.rows());
    for (i, line) in rendered_lines.iter().enumerate() {
        round_trip.process(format!("\x1b[{};1H", i + 1).as_bytes());
        round_trip.process(line.as_bytes());
    }

    for row in 0..parsed.rows() {
        for col in 0..cols {
            let orig = parsed.get_cell(row, col).unwrap();
            let rt = round_trip.get_cell(row, col).unwrap();
            assert_eq!(
                orig.char, rt.char,
                "{name}: char mismatch at ({row},{col}): orig={:?} rt={:?}",
                orig.char, rt.char,
            );
            assert_eq!(
                orig.attr, rt.attr,
                "{name}: attr mismatch at ({row},{col}): orig={:?} rt={:?}",
                orig.attr, rt.attr,
            );
        }
    }

    // Snapshot rendered state for human review
    insta::assert_snapshot!(format!("{name}_rendered"), round_trip.serialize());
}

// ── Colors (indexed) ────────────────────────────────────────────────

#[test]
fn colors_indexed_snapshot() {
    let parsed = parse_fixture(include_bytes!("../test-fixtures/colors_indexed.vt"), 80, 24);
    insta::assert_snapshot!("colors_indexed_parsed", parsed.serialize());
    assert_round_trip("colors_indexed", &parsed, 80);
}

// ── Colors (RGB) ────────────────────────────────────────────────────

#[test]
fn colors_rgb_snapshot() {
    let parsed = parse_fixture(include_bytes!("../test-fixtures/colors_rgb.vt"), 80, 24);
    insta::assert_snapshot!("colors_rgb_parsed", parsed.serialize());
    assert_round_trip("colors_rgb", &parsed, 80);
}

// ── SGR attributes ──────────────────────────────────────────────────

#[test]
fn sgr_attrs_snapshot() {
    let parsed = parse_fixture(include_bytes!("../test-fixtures/sgr_attrs.vt"), 80, 24);
    insta::assert_snapshot!("sgr_attrs_parsed", parsed.serialize());
    assert_round_trip("sgr_attrs", &parsed, 80);
}

// ── Cursor addressing ───────────────────────────────────────────────

#[test]
fn cursor_addressing_snapshot() {
    let parsed = parse_fixture(
        include_bytes!("../test-fixtures/cursor_addressing.vt"),
        80,
        24,
    );
    insta::assert_snapshot!("cursor_addressing_parsed", parsed.serialize());
    assert_round_trip("cursor_addressing", &parsed, 80);
}

// ── Alt screen ──────────────────────────────────────────────────────

#[test]
fn alt_screen_snapshot() {
    let parsed = parse_fixture(include_bytes!("../test-fixtures/alt_screen.vt"), 80, 24);
    // After processing, should be back on main screen with "MainScreen"
    insta::assert_snapshot!("alt_screen_parsed", parsed.serialize());
    assert_round_trip("alt_screen", &parsed, 80);
}
