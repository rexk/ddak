pub const CRATE_NAME: &str = "terminal-surface";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttr {
    pub fg: CellColor,
    pub bg: CellColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub reverse: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub char: char,
    pub attr: CellAttr,
}

impl Cell {
    pub fn new(char: char) -> Self {
        Self {
            char,
            attr: CellAttr::default(),
        }
    }
}

pub struct ScreenGrid {
    cols: u16,
    rows: u16,
    grid: Vec<Vec<Cell>>,
    cursor_row: u16,
    cursor_col: u16,
    saved_cursor_row: u16,
    saved_cursor_col: u16,
    scroll_top: u16,
    scroll_bottom: u16,
    current_attr: CellAttr,
    alt_screen: Option<Vec<Vec<Cell>>>,
    in_alt_screen: bool,
    vte_parser: Option<vte::Parser>,
}

impl std::fmt::Debug for ScreenGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenGrid")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("cursor_row", &self.cursor_row)
            .field("cursor_col", &self.cursor_col)
            .field("in_alt_screen", &self.in_alt_screen)
            .finish_non_exhaustive()
    }
}

impl Clone for ScreenGrid {
    fn clone(&self) -> Self {
        Self {
            cols: self.cols,
            rows: self.rows,
            grid: self.grid.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            saved_cursor_row: self.saved_cursor_row,
            saved_cursor_col: self.saved_cursor_col,
            scroll_top: self.scroll_top,
            scroll_bottom: self.scroll_bottom,
            current_attr: self.current_attr,
            alt_screen: self.alt_screen.clone(),
            in_alt_screen: self.in_alt_screen,
            vte_parser: Some(vte::Parser::new()),
        }
    }
}

impl ScreenGrid {
    pub fn new(cols: u16, rows: u16) -> Self {
        let grid = Self::create_empty_grid(cols, rows);
        Self {
            cols,
            rows,
            grid,
            cursor_row: 0,
            cursor_col: 0,
            saved_cursor_row: 0,
            saved_cursor_col: 0,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            current_attr: CellAttr::default(),
            alt_screen: None,
            in_alt_screen: false,
            vte_parser: Some(vte::Parser::new()),
        }
    }

    fn create_empty_grid(cols: u16, rows: u16) -> Vec<Vec<Cell>> {
        (0..rows)
            .map(|_| vec![Cell::new(' '); cols as usize])
            .collect()
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cursor(&self) -> (u16, u16) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn resize(&mut self, new_cols: u16, new_rows: u16) {
        let mut new_grid = Self::create_empty_grid(new_cols, new_rows);

        for (row_idx, row) in self.grid.iter().enumerate() {
            if row_idx >= new_rows as usize {
                break;
            }
            let new_row = &mut new_grid[row_idx];
            for (col_idx, cell) in row.iter().enumerate() {
                if col_idx >= new_cols as usize {
                    break;
                }
                new_row[col_idx] = cell.clone();
            }
        }

        self.grid = new_grid;
        self.cols = new_cols;
        self.rows = new_rows;
        self.scroll_bottom = new_rows.saturating_sub(1);
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));

        if let Some(ref mut alt) = self.alt_screen {
            let mut new_alt = Self::create_empty_grid(new_cols, new_rows);
            for (row_idx, row) in alt.iter().enumerate() {
                if row_idx >= new_rows as usize {
                    break;
                }
                let new_row = &mut new_alt[row_idx];
                for (col_idx, cell) in row.iter().enumerate() {
                    if col_idx >= new_cols as usize {
                        break;
                    }
                    new_row[col_idx] = cell.clone();
                }
            }
            *alt = new_alt;
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let mut parser = self.vte_parser.take().unwrap_or_default();
        for byte in bytes {
            parser.advance(self, *byte);
        }
        self.vte_parser = Some(parser);
    }

    pub fn get_row(&self, row: u16) -> Option<&[Cell]> {
        self.grid.get(row as usize).map(|r| r.as_slice())
    }

    pub fn get_cell(&self, row: u16, col: u16) -> Option<&Cell> {
        self.grid.get(row as usize)?.get(col as usize)
    }

    pub fn get_content(&self) -> Vec<String> {
        self.grid
            .iter()
            .map(|row| row.iter().map(|c| c.char).collect::<String>())
            .collect()
    }

    pub fn get_content_trimmed(&self) -> Vec<String> {
        self.grid
            .iter()
            .map(|row| {
                let s: String = row.iter().map(|c| c.char).collect();
                s.trim_end().to_string()
            })
            .collect()
    }

    pub fn debug_print_state(&self) -> String {
        format!(
            "cols={} rows={} cursor=({},{}) scroll=({},{}) in_alt_screen={}",
            self.cols,
            self.rows,
            self.cursor_row,
            self.cursor_col,
            self.scroll_top,
            self.scroll_bottom,
            self.in_alt_screen
        )
    }
}

impl vte::Perform for ScreenGrid {
    fn print(&mut self, c: char) {
        if self.cursor_col >= self.cols || self.cursor_row >= self.rows {
            return;
        }

        if let Some(row) = self.grid.get_mut(self.cursor_row as usize)
            && let Some(cell) = row.get_mut(self.cursor_col as usize)
        {
            cell.char = c;
            cell.attr = self.current_attr;
        }

        self.cursor_col = self.cursor_col.saturating_add(1);
        if self.cursor_col >= self.cols {
            self.cursor_col = self.cols.saturating_sub(1);
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                self.cursor_col = self.cursor_col.saturating_sub(1);
            }
            0x09 => {
                let next_tab = ((self.cursor_col / 8) + 1) * 8;
                self.cursor_col = next_tab.min(self.cols.saturating_sub(1));
            }
            0x0A..=0x0C => {
                self.cursor_row = self.cursor_row.saturating_add(1);
                if self.cursor_row > self.scroll_bottom {
                    self.scroll_up(1);
                    self.cursor_row = self.scroll_bottom;
                }
            }
            0x0D => {
                self.cursor_col = 0;
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        match action {
            'A' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_row = self.cursor_row.saturating_sub(count);
            }
            'B' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_row = (self.cursor_row + count).min(self.rows.saturating_sub(1));
            }
            'C' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_col = (self.cursor_col + count).min(self.cols.saturating_sub(1));
            }
            'D' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_col = self.cursor_col.saturating_sub(count);
            }
            'E' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_col = 0;
                self.cursor_row = (self.cursor_row + count).min(self.rows.saturating_sub(1));
            }
            'F' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_col = 0;
                self.cursor_row = self.cursor_row.saturating_sub(count);
            }
            'G' => {
                let col = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_col = col.saturating_sub(1).min(self.cols.saturating_sub(1));
            }
            'H' | 'f' => {
                let mut iter = params.iter();
                let row = iter.next().map(|p| p[0]).unwrap_or(1);
                let col = iter.next().map(|p| p[0]).unwrap_or(1);
                self.cursor_row = row.saturating_sub(1).min(self.rows.saturating_sub(1));
                self.cursor_col = col.saturating_sub(1).min(self.cols.saturating_sub(1));
            }
            'J' => {
                let mode = params.iter().next().map(|p| p[0]).unwrap_or(0);
                match mode {
                    0 => {
                        if let Some(row) = self.grid.get_mut(self.cursor_row as usize) {
                            for cell in row.iter_mut().skip(self.cursor_col as usize) {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                        for row in self.grid.iter_mut().skip(self.cursor_row as usize + 1) {
                            for cell in row.iter_mut() {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                    }
                    1 => {
                        for row in self.grid.iter_mut().take(self.cursor_row as usize) {
                            for cell in row.iter_mut() {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                        if let Some(row) = self.grid.get_mut(self.cursor_row as usize) {
                            for cell in row.iter_mut().take(self.cursor_col as usize + 1) {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                    }
                    2 | 3 => {
                        self.grid = Self::create_empty_grid(self.cols, self.rows);
                    }
                    _ => {}
                }
            }
            'K' => {
                let mode = params.iter().next().map(|p| p[0]).unwrap_or(0);
                if let Some(row) = self.grid.get_mut(self.cursor_row as usize) {
                    match mode {
                        0 => {
                            for cell in row.iter_mut().skip(self.cursor_col as usize) {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                        1 => {
                            for cell in row.iter_mut().take(self.cursor_col as usize + 1) {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                        2 => {
                            for cell in row.iter_mut() {
                                cell.char = ' ';
                                cell.attr = CellAttr::default();
                            }
                        }
                        _ => {}
                    }
                }
            }
            'L' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1).max(1);
                for _ in 0..count {
                    if self.cursor_row <= self.scroll_bottom {
                        self.grid.remove(self.scroll_bottom as usize);
                        self.grid.insert(
                            self.cursor_row as usize,
                            vec![Cell::new(' '); self.cols as usize],
                        );
                    }
                }
            }
            'M' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1).max(1);
                for _ in 0..count {
                    if self.cursor_row <= self.scroll_bottom {
                        self.grid.remove(self.cursor_row as usize);
                        self.grid.insert(
                            self.scroll_bottom as usize,
                            vec![Cell::new(' '); self.cols as usize],
                        );
                    }
                }
            }
            'P' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                if let Some(row) = self.grid.get_mut(self.cursor_row as usize) {
                    let start = self.cursor_col as usize;
                    let end = (start + count as usize).min(self.cols as usize);
                    row.drain(start..end);
                    for _ in 0..count.min(self.cols - self.cursor_col) {
                        row.push(Cell::new(' '));
                    }
                }
            }
            '@' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                if let Some(row) = self.grid.get_mut(self.cursor_row as usize) {
                    let start = self.cursor_col as usize;
                    for _ in 0..count {
                        if row.len() < self.cols as usize {
                            row.insert(start, Cell::new(' '));
                        }
                    }
                }
            }
            'S' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.scroll_up(count);
            }
            'T' => {
                let count = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.scroll_down(count);
            }
            'd' => {
                let row = params.iter().next().map(|p| p[0]).unwrap_or(1);
                self.cursor_row = row.saturating_sub(1).min(self.rows.saturating_sub(1));
            }
            'm' => {
                let flat: Vec<u16> = params.iter().flat_map(|p| p.iter().copied()).collect();
                let mut i = 0;
                while i < flat.len() {
                    match flat[i] {
                        0 => {
                            self.current_attr = CellAttr::default();
                            i += 1;
                        }
                        1 => {
                            self.current_attr.bold = true;
                            i += 1;
                        }
                        2 => {
                            self.current_attr.dim = true;
                            i += 1;
                        }
                        3 => {
                            self.current_attr.italic = true;
                            i += 1;
                        }
                        4 => {
                            self.current_attr.underline = true;
                            i += 1;
                        }
                        7 => {
                            self.current_attr.reverse = true;
                            i += 1;
                        }
                        22 => {
                            self.current_attr.bold = false;
                            self.current_attr.dim = false;
                            i += 1;
                        }
                        23 => {
                            self.current_attr.italic = false;
                            i += 1;
                        }
                        24 => {
                            self.current_attr.underline = false;
                            i += 1;
                        }
                        27 => {
                            self.current_attr.reverse = false;
                            i += 1;
                        }
                        30..=37 => {
                            self.current_attr.fg = CellColor::Indexed(flat[i] as u8 - 30);
                            i += 1;
                        }
                        38 => {
                            if i + 2 < flat.len() && flat[i + 1] == 5 {
                                self.current_attr.fg = CellColor::Indexed(flat[i + 2] as u8);
                                i += 3;
                            } else if i + 4 < flat.len() && flat[i + 1] == 2 {
                                self.current_attr.fg = CellColor::Rgb(
                                    flat[i + 2] as u8,
                                    flat[i + 3] as u8,
                                    flat[i + 4] as u8,
                                );
                                i += 5;
                            } else {
                                i += 1;
                            }
                        }
                        39 => {
                            self.current_attr.fg = CellColor::Default;
                            i += 1;
                        }
                        40..=47 => {
                            self.current_attr.bg = CellColor::Indexed(flat[i] as u8 - 40);
                            i += 1;
                        }
                        48 => {
                            if i + 2 < flat.len() && flat[i + 1] == 5 {
                                self.current_attr.bg = CellColor::Indexed(flat[i + 2] as u8);
                                i += 3;
                            } else if i + 4 < flat.len() && flat[i + 1] == 2 {
                                self.current_attr.bg = CellColor::Rgb(
                                    flat[i + 2] as u8,
                                    flat[i + 3] as u8,
                                    flat[i + 4] as u8,
                                );
                                i += 5;
                            } else {
                                i += 1;
                            }
                        }
                        49 => {
                            self.current_attr.bg = CellColor::Default;
                            i += 1;
                        }
                        90..=97 => {
                            self.current_attr.fg = CellColor::Indexed(flat[i] as u8 - 82);
                            i += 1;
                        }
                        100..=107 => {
                            self.current_attr.bg = CellColor::Indexed(flat[i] as u8 - 92);
                            i += 1;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
            }
            'r' => {
                let mut iter = params.iter();
                let top = iter.next().map(|p| p[0]).unwrap_or(1);
                let bottom = iter.next().map(|p| p[0]).unwrap_or(self.rows);
                self.scroll_top = top.saturating_sub(1);
                self.scroll_bottom = bottom.saturating_sub(1).min(self.rows.saturating_sub(1));
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            'h' | 'l' => {
                if _intermediates == b"?" {
                    let mode = params.iter().next().map(|p| p[0]).unwrap_or(0);
                    if mode == 1049 {
                        if action == 'h' {
                            self.alt_screen = Some(std::mem::take(&mut self.grid));
                            self.grid = Self::create_empty_grid(self.cols, self.rows);
                            self.in_alt_screen = true;
                            self.saved_cursor_row = self.cursor_row;
                            self.saved_cursor_col = self.cursor_col;
                            self.cursor_row = 0;
                            self.cursor_col = 0;
                        } else {
                            if let Some(alt) = self.alt_screen.take() {
                                self.grid = alt;
                            }
                            self.in_alt_screen = false;
                            self.cursor_row = self.saved_cursor_row;
                            self.cursor_col = self.saved_cursor_col;
                        }
                    }
                }
            }
            's' => {
                self.saved_cursor_row = self.cursor_row;
                self.saved_cursor_col = self.cursor_col;
            }
            'u' => {
                self.cursor_row = self.saved_cursor_row;
                self.cursor_col = self.saved_cursor_col;
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
    }

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}
}

impl ScreenGrid {
    fn scroll_up(&mut self, count: u16) {
        for _ in 0..count {
            if self.scroll_top < self.scroll_bottom {
                self.grid.remove(self.scroll_top as usize);
                self.grid.insert(
                    self.scroll_bottom as usize,
                    vec![Cell::new(' '); self.cols as usize],
                );
            }
        }
    }

    fn scroll_down(&mut self, count: u16) {
        for _ in 0..count {
            if self.scroll_top < self.scroll_bottom {
                self.grid.remove(self.scroll_bottom as usize);
                self.grid.insert(
                    self.scroll_top as usize,
                    vec![Cell::new(' '); self.cols as usize],
                );
            }
        }
    }
}

impl ScreenGrid {
    /// Deterministic, diffable text serialization of screen state.
    /// Omits trailing whitespace-only rows and attr lines for default-only rows.
    pub fn serialize(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();
        let _ = writeln!(out, "[{}x{}]", self.cols, self.rows);

        // Find last non-blank row
        let last_row = self
            .grid
            .iter()
            .rposition(|row| {
                row.iter()
                    .any(|c| c.char != ' ' || c.attr != CellAttr::default())
            })
            .map(|i| i + 1)
            .unwrap_or(0);

        for row_idx in 0..last_row {
            let row = &self.grid[row_idx];
            // Text line (trimmed)
            let text: String = row.iter().map(|c| c.char).collect::<String>();
            let trimmed = text.trim_end();
            let _ = writeln!(out, "{:>3}: {:?}", row_idx, trimmed);

            // Attr spans — run-length encode
            let has_non_default = row.iter().any(|c| c.attr != CellAttr::default());
            if has_non_default {
                let mut spans = Vec::new();
                let mut span_start = 0;
                let mut span_attr = &row[0].attr;

                for (col, cell) in row.iter().enumerate().skip(1) {
                    if &cell.attr != span_attr {
                        if *span_attr != CellAttr::default() {
                            spans.push((span_start, col, *span_attr));
                        }
                        span_start = col;
                        span_attr = &cell.attr;
                    }
                }
                if *span_attr != CellAttr::default() {
                    spans.push((span_start, row.len(), *span_attr));
                }

                if !spans.is_empty() {
                    let mut attr_line = format!("{:>3}: attrs", row_idx);
                    for (start, end, attr) in &spans {
                        let _ = write!(attr_line, " [{start}..{end}]=");
                        let mut parts = Vec::new();
                        match attr.fg {
                            CellColor::Default => {}
                            CellColor::Indexed(i) => parts.push(format!("fg:Idx({i})")),
                            CellColor::Rgb(r, g, b) => parts.push(format!("fg:Rgb({r},{g},{b})")),
                        }
                        match attr.bg {
                            CellColor::Default => {}
                            CellColor::Indexed(i) => parts.push(format!("bg:Idx({i})")),
                            CellColor::Rgb(r, g, b) => parts.push(format!("bg:Rgb({r},{g},{b})")),
                        }
                        if attr.bold {
                            parts.push("bold".into());
                        }
                        if attr.dim {
                            parts.push("dim".into());
                        }
                        if attr.italic {
                            parts.push("italic".into());
                        }
                        if attr.underline {
                            parts.push("underline".into());
                        }
                        if attr.reverse {
                            parts.push("reverse".into());
                        }
                        let _ = write!(attr_line, "{}", parts.join(","));
                    }
                    let _ = writeln!(out, "{attr_line}");
                }
            }
        }

        out
    }

    /// Render each row as a string with SGR escape sequences reconstructing
    /// the full `CellAttr` state (fg, bg, bold, dim, italic, underline, reverse).
    pub fn render_to_sgr(&self, width: u16) -> Vec<String> {
        use std::fmt::Write;

        let mut lines = Vec::with_capacity(self.rows as usize);
        for row in &self.grid {
            let mut line = String::new();
            let mut prev_attr = CellAttr::default();

            for (col, cell) in row.iter().enumerate() {
                if col >= width as usize {
                    break;
                }
                if cell.attr != prev_attr {
                    // Reset and re-emit all active attributes
                    line.push_str("\x1b[0m");
                    // Background
                    match cell.attr.bg {
                        CellColor::Default => {}
                        CellColor::Indexed(i) => {
                            let _ = write!(line, "\x1b[48;5;{i}m");
                        }
                        CellColor::Rgb(r, g, b) => {
                            let _ = write!(line, "\x1b[48;2;{r};{g};{b}m");
                        }
                    }
                    // Foreground
                    match cell.attr.fg {
                        CellColor::Default => {}
                        CellColor::Indexed(i) => {
                            let _ = write!(line, "\x1b[38;5;{i}m");
                        }
                        CellColor::Rgb(r, g, b) => {
                            let _ = write!(line, "\x1b[38;2;{r};{g};{b}m");
                        }
                    }
                    if cell.attr.bold {
                        line.push_str("\x1b[1m");
                    }
                    if cell.attr.dim {
                        line.push_str("\x1b[2m");
                    }
                    if cell.attr.italic {
                        line.push_str("\x1b[3m");
                    }
                    if cell.attr.underline {
                        line.push_str("\x1b[4m");
                    }
                    if cell.attr.reverse {
                        line.push_str("\x1b[7m");
                    }
                    prev_attr = cell.attr;
                }
                line.push(cell.char);
            }
            line.push_str("\x1b[0m");
            lines.push(line);
        }
        lines
    }
}

pub struct DiffRegion {
    pub row_start: u16,
    pub row_end: u16,
    pub col_start: u16,
    pub col_end: u16,
}

impl ScreenGrid {
    pub fn compute_diff(&self, prev: &ScreenGrid) -> Vec<DiffRegion> {
        let mut regions = Vec::new();

        for row in 0..self.rows.min(prev.rows) {
            let current_row = self.get_row(row);
            let prev_row = prev.get_row(row);

            if let (Some(curr), Some(prv)) = (current_row, prev_row) {
                let mut col_start: Option<u16> = None;

                for col in 0..self.cols.min(prev.cols) as usize {
                    let changed = curr.get(col).map(|c| c.char) != prv.get(col).map(|c| c.char);

                    if changed && col_start.is_none() {
                        col_start = Some(col as u16);
                    } else if !changed && col_start.is_some() {
                        regions.push(DiffRegion {
                            row_start: row,
                            row_end: row,
                            col_start: col_start.unwrap(),
                            col_end: col as u16,
                        });
                        col_start = None;
                    }
                }

                if let Some(start) = col_start {
                    regions.push(DiffRegion {
                        row_start: row,
                        row_end: row,
                        col_start: start,
                        col_end: self.cols,
                    });
                }
            }
        }

        if self.rows > prev.rows {
            regions.push(DiffRegion {
                row_start: prev.rows,
                row_end: self.rows,
                col_start: 0,
                col_end: self.cols,
            });
        }

        regions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_grid_with_correct_dimensions() {
        let grid = ScreenGrid::new(80, 24);
        assert_eq!(grid.cols(), 80);
        assert_eq!(grid.rows(), 24);
        assert_eq!(grid.cursor(), (0, 0));
    }

    #[test]
    fn prints_characters_and_advances_cursor() {
        let mut grid = ScreenGrid::new(10, 5);
        grid.process(b"Hello");

        assert_eq!(grid.cursor_col, 5);
        assert_eq!(grid.get_cell(0, 0).unwrap().char, 'H');
        assert_eq!(grid.get_cell(0, 4).unwrap().char, 'o');
    }

    #[test]
    fn handles_newline() {
        let mut grid = ScreenGrid::new(10, 5);
        grid.process(b"AB\nCD");

        assert_eq!(grid.cursor(), (1, 4));
        assert_eq!(grid.get_cell(0, 0).unwrap().char, 'A');
        assert_eq!(grid.get_cell(0, 1).unwrap().char, 'B');
        assert_eq!(grid.get_cell(1, 2).unwrap().char, 'C');
        assert_eq!(grid.get_cell(1, 3).unwrap().char, 'D');
    }

    #[test]
    fn handles_cursor_movement() {
        let mut grid = ScreenGrid::new(10, 5);
        grid.process(b"Hello\x1b[3D");
        assert_eq!(grid.cursor_col, 2);

        grid.process(b"\x1b[2A");
        assert_eq!(grid.cursor_row, 0);
    }

    #[test]
    fn handles_clear_screen() {
        let mut grid = ScreenGrid::new(10, 5);
        grid.process(b"Hello\x1b[2J");

        assert_eq!(grid.get_cell(0, 0).unwrap().char, ' ');
    }

    #[test]
    fn handles_resize() {
        let mut grid = ScreenGrid::new(10, 5);
        grid.process(b"Hello");

        grid.resize(20, 10);

        assert_eq!(grid.cols(), 20);
        assert_eq!(grid.rows(), 10);
        assert_eq!(grid.get_cell(0, 0).unwrap().char, 'H');
    }

    #[test]
    fn get_content_returns_all_rows() {
        let mut grid = ScreenGrid::new(10, 3);
        grid.process(b"AB\r\nCD\r\nEF");

        let content = grid.get_content();
        assert_eq!(content.len(), 3);
        assert!(content[0].starts_with("AB"));
        assert!(content[1].starts_with("CD"));
        assert!(content[2].starts_with("EF"));
    }

    #[test]
    fn handles_scroll_region() {
        let mut grid = ScreenGrid::new(10, 5);
        grid.process(b"Line1\nLine2\nLine3\nLine4\nLine5");
        grid.process(b"\x1b[1;3r");
        grid.process(b"\x1b[2;1H");
        grid.process(b"\x1b[L");

        let content = grid.get_content_trimmed();
        assert_eq!(content[0], "Line1");
    }
}
