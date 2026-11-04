// SPDX-License-Identifier: AGPL-3.0-only
//! The CSV dialect of Android's data backup, byte for byte like the app reads its own files:
//! `au.com.bytecode.opencsv` 2.3 via `app/utils/CSVReader.java` — separator `,`, quote `"`,
//! reader escape `\`; a quote inside a quoted field is doubled (`""`, the writer's escape is the
//! quote). `CSVRow.escape(String)` additionally doubles every backslash before writing, so the
//! reader's backslash escape halves them back. Embedded line breaks of any kind come back as
//! `\n` (Java reads physical lines with `BufferedReader.readLine`). A general-purpose CSV crate
//! is deliberately not used: OpenCSV's quirks (a lone backslash inside quotes is dropped, odd
//! mid-field quotes vanish) would differ on malformed rows, and this parser is small and pinned
//! by tests to the upstream behaviour.

use zeroize::{Zeroize as _, Zeroizing};

/// Hard bounds, far above what a backup holds and below what would hurt: a table is refused as
/// a whole beyond them. A file is at most 64 MiB (`MAX_TABLE_BYTES`), but small cells cost far
/// more memory than their bytes, so rows and cells are counted too. For scale: old backups hold
/// a chunk of nonces in one cell (Android's chunks have at most 10 000, about 650 KiB), a nonce
/// row takes 67 bytes and a message row some 30 cells in a few hundred bytes.
const MAX_FIELD_BYTES: usize = 4 << 20;
const MAX_ROW_BYTES: usize = 8 << 20;
const MAX_ROW_FIELDS: usize = 256;
const MAX_ROWS: usize = 2 << 20;
const MAX_CELLS: usize = 16 << 20;

/// A table beyond the bounds above.
#[derive(Debug, thiserror::Error)]
#[error("table beyond the reader's bounds")]
pub struct TooLarge;

/// One logical row per `Vec<String>`; empty input gives no rows. Equivalent to reading every
/// row with `CSVReader.readNext()` (multi-line aware), in time linear in the text. Rows are
/// wiped when dropped: these files hold message text and blob keys.
pub fn parse(text: &str) -> Result<Zeroizing<Vec<Vec<String>>>, TooLarge> {
    let mut reader = Reader::new(text.len());
    for line in physical_lines(text) {
        reader.line(line)?;
    }
    reader.finish()
}

/// Physical lines as `BufferedReader.readLine` sees them: terminated by `\n`, `\r` or `\r\n`
/// (the terminator is stripped; a trailing terminator ends the last line).
fn physical_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
        let line = &rest[..end];
        rest = &rest[end..];
        if rest.starts_with("\r\n") {
            rest = &rest[2..];
        } else if !rest.is_empty() {
            rest = &rest[1..];
        }
        Some(line)
    })
}

const SEPARATOR: char = ',';
const QUOTE: char = '"';
const ESCAPE: char = '\\';

/// `au.com.bytecode.opencsv.CSVParser` (2.3) with its defaults (`strictQuotes` off,
/// `ignoreLeadingWhiteSpace` on), `parseLineMulti` semantics, plus the rows read so far.
struct Reader {
    rows: Zeroizing<Vec<Vec<String>>>,
    row: Zeroizing<Vec<String>>,
    row_bytes: usize,
    cells: usize,
    /// The field being read. Sized once so that it never moves: a growing string would leave
    /// freed copies of what it held. Wiped at the end, finished fields are copied out of it.
    field: Zeroizing<String>,
    /// `field` holds nothing but white space.
    blank: bool,
    /// A quoted field continues on the next physical line.
    pending: bool,
}

impl Reader {
    fn new(text_bytes: usize) -> Self {
        Self {
            rows: Zeroizing::new(Vec::new()),
            row: Zeroizing::new(Vec::new()),
            row_bytes: 0,
            cells: 0,
            // A field is never longer than the text and the break of its last line.
            field: Zeroizing::new(String::with_capacity(
                text_bytes.saturating_add(1).min(MAX_FIELD_BYTES),
            )),
            blank: true,
            pending: false,
        }
    }

    fn push(&mut self, c: char) -> Result<(), TooLarge> {
        if self.field.len() + c.len_utf8() > MAX_FIELD_BYTES {
            return Err(TooLarge);
        }
        self.field.push(c);
        self.blank &= c.is_whitespace();
        Ok(())
    }

    fn clear_field(&mut self) {
        self.field.clear();
        self.blank = true;
    }

    fn end_field(&mut self) -> Result<(), TooLarge> {
        self.row_bytes += self.field.len();
        self.cells += 1;
        if self.row.len() >= MAX_ROW_FIELDS
            || self.row_bytes > MAX_ROW_BYTES
            || self.cells > MAX_CELLS
        {
            return Err(TooLarge);
        }
        self.row.push(self.field.as_str().to_owned());
        self.clear_field();
        Ok(())
    }

    fn end_row(&mut self) -> Result<(), TooLarge> {
        if !self.row.is_empty() {
            if self.rows.len() >= MAX_ROWS {
                return Err(TooLarge);
            }
            self.rows.push(std::mem::take(&mut *self.row));
            self.row_bytes = 0;
        }
        Ok(())
    }

    fn line(&mut self, line: &str) -> Result<(), TooLarge> {
        let mut in_quotes = std::mem::take(&mut self.pending);
        let mut in_field = false;
        let mut chars = line.chars().peekable();
        let mut previous = None;
        let mut index = 0_usize;
        while let Some(c) = chars.next() {
            let next = chars.peek().copied();
            let mut last = c;
            if c == ESCAPE {
                // Escapable (a quote or another backslash follows, inside a field): take the
                // next character literally. Otherwise the backslash is swallowed, like OpenCSV.
                if let Some(escaped @ (QUOTE | ESCAPE)) = next
                    && (in_quotes || in_field)
                {
                    self.push(escaped)?;
                    chars.next();
                    index += 1;
                    last = escaped;
                }
            } else if c == QUOTE {
                if (in_quotes || in_field) && next == Some(QUOTE) {
                    // A doubled quote inside a field: one literal quote.
                    self.push(QUOTE)?;
                    chars.next();
                    index += 1;
                } else {
                    // OpenCSV keeps a quote that sits in the middle of a field (not next to a
                    // separator); leading white space before it is discarded.
                    if index > 2
                        && previous != Some(SEPARATOR)
                        && next.is_some_and(|next| next != SEPARATOR)
                    {
                        if !self.field.is_empty() && self.blank {
                            self.clear_field();
                        } else {
                            self.push(QUOTE)?;
                        }
                    }
                    in_quotes = !in_quotes;
                }
                in_field = !in_field;
            } else if c == SEPARATOR && !in_quotes {
                self.end_field()?;
                in_field = false;
            } else {
                self.push(c)?;
                in_field = true;
            }
            previous = Some(last);
            index += 1;
        }
        if in_quotes {
            self.push('\n')?;
            self.pending = true;
            Ok(())
        } else {
            self.end_field()?;
            self.end_row()
        }
    }

    fn finish(mut self) -> Result<Zeroizing<Vec<Vec<String>>>, TooLarge> {
        // An unterminated quote at the end of the file: Java would keep waiting for more lines
        // and return the pending text as a final field once the stream ends.
        if self.pending {
            self.end_field()?;
        }
        self.end_row()?;
        Ok(self.rows)
    }
}

/// A header-keyed CSV file (every data file but `settings`): fields are found by column name,
/// never by position (`CSVRow.getValuePosition`); missing columns are `None`, so version gates
/// decide what must exist.
#[derive(Default)]
pub struct Table {
    header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Drop for Table {
    fn drop(&mut self) {
        self.header.zeroize();
        self.rows.zeroize();
    }
}

impl Table {
    /// The first row is the header; files without rows are valid and empty.
    pub fn parse(text: &str) -> Result<Self, TooLarge> {
        let mut rows = parse(text)?;
        let header = if rows.is_empty() {
            Vec::new()
        } else {
            rows.remove(0)
        };
        Ok(Self {
            header,
            rows: std::mem::take(&mut *rows),
        })
    }

    pub fn has_column(&self, name: &str) -> bool {
        self.header.iter().any(|column| column == name)
    }

    /// The value of `column` in `row`, if the column exists and the row reaches it.
    pub fn get<'a>(&self, row: &'a [String], column: &str) -> Option<&'a str> {
        let position = self.header.iter().position(|c| c == column)?;
        row.get(position).map(String::as_str)
    }

    /// Like Java's `CSVRow.getBoolean`: exactly `"1"` is true.
    pub fn get_bool(&self, row: &[String], column: &str) -> bool {
        self.get(row, column) == Some("1")
    }

    /// Epoch milliseconds (`CSVRow.getInstant`): empty or missing is `None`.
    pub fn get_ms(&self, row: &[String], column: &str) -> Option<i64> {
        self.get(row, column)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse().ok())
    }

    /// A semicolon-separated list (`CSVRow.getStrings`, e.g. group members); empty cells give
    /// an empty list.
    pub fn get_list<'a>(&self, row: &'a [String], column: &str) -> Vec<&'a str> {
        self.get(row, column)
            .filter(|value| !value.is_empty())
            .map(|value| value.split(';').collect())
            .unwrap_or_default()
    }
}

/// What `CSVRow`+`CSVWriter` produce for one row: every field quoted, quotes doubled,
/// backslashes doubled by `CSVRow.escape` first. The export writes its files with this, so
/// they read back exactly like Android's own.
pub fn write_row(fields: &[&str]) -> String {
    let mut line = String::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            line.push(SEPARATOR);
        }
        line.push(QUOTE);
        for c in field.replace('\\', "\\\\").chars() {
            if c == QUOTE {
                line.push(QUOTE);
            }
            line.push(c);
        }
        line.push(QUOTE);
    }
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of a text within the bounds.
    fn rows(text: &str) -> Vec<Vec<String>> {
        std::mem::take(&mut *parse(text).unwrap())
    }

    #[test]
    fn plain_rows() {
        let parsed = rows("\"a\",\"b\"\n\"c\",\"d\"\n");
        assert_eq!(parsed, vec![vec!["a", "b"], vec!["c", "d"]]);
        // Unquoted values and empty cells work too (settings uses plain two-column rows).
        assert_eq!(rows("version,27\n"), vec![vec!["version", "27"]]);
        assert_eq!(rows("\"a\",,\"c\"\n"), vec![vec!["a", "", "c"]]);
        assert!(rows("").is_empty());
    }

    #[test]
    fn quotes_commas_newlines_backslashes() {
        // What CSVRow/CSVWriter make of tricky content, parsed back to the original.
        let original = [
            "with \"quotes\"",
            "comma, semicolon;",
            "line\nbreak",
            "back\\slash and \\\" mix",
            "emoji 🎉 äöü",
            "",
        ];
        let mut file = String::new();
        file.push_str(&write_row(&original));
        let parsed = rows(&file);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], original);
    }

    #[test]
    fn crlf_and_multiline() {
        // CRLF line ends; an embedded break always comes back as plain \n (Java readLine).
        let file = "\"a\r\nb\",\"c\"\r\n\"next\"\r\n";
        let parsed = rows(file);
        assert_eq!(parsed, vec![vec!["a\nb", "c"], vec!["next"]]);
        // Unterminated quote at the end of the file: the pending text (with the continuation
        // break OpenCSV already appended) is the last field.
        assert_eq!(rows("\"open"), vec![vec!["open\n"]]);
    }

    #[test]
    fn opencsv_quirks() {
        // A lone backslash inside quotes is swallowed (OpenCSV's escape handling); the writer
        // never produces one, but a parser difference here would corrupt text silently.
        assert_eq!(rows("\"a\\b\""), vec![vec!["ab"]]);
        // A quote in the middle of a field stays.
        assert_eq!(rows("\"midd\"\"le\""), vec![vec!["midd\"le"]]);
        assert_eq!(rows("abc\"de\"f"), vec![vec!["abc\"de\"f"]]);
    }

    #[test]
    fn tables_by_header() {
        let mut file = String::new();
        file.push_str(&write_row(&[
            "identity",
            "members",
            "deleted",
            "created_at",
        ]));
        file.push_str(&write_row(&["00000BOB", "A;B;C", "1", "1700000000000"]));
        file.push_str(&write_row(&["00000ANN", "", "0", ""]));
        let table = Table::parse(&file).unwrap();
        assert!(table.has_column("members"));
        assert!(!table.has_column("nope"));
        let rows = &table.rows;
        assert_eq!(table.get(&rows[0], "identity"), Some("00000BOB"));
        assert_eq!(table.get_list(&rows[0], "members"), vec!["A", "B", "C"]);
        assert!(table.get_bool(&rows[0], "deleted"));
        assert_eq!(
            table.get_ms(&rows[0], "created_at"),
            Some(1_700_000_000_000)
        );
        assert!(table.get_list(&rows[1], "members").is_empty());
        assert!(!table.get_bool(&rows[1], "deleted"));
        assert_eq!(table.get_ms(&rows[1], "created_at"), None);
        assert_eq!(table.get(&rows[1], "nope"), None);
        // An empty file is an empty table.
        let empty = Table::parse("").unwrap();
        assert!(empty.rows.is_empty());
    }

    #[test]
    fn a_crafted_line_stays_linear() {
        // White space, then a quote per few characters: rescanning the field for each quote
        // took hours at this size.
        let mut line = " ".repeat(3 << 20);
        line.push('x');
        for _ in 0..300_000 {
            line.push_str("\"a");
        }
        let start = std::time::Instant::now();
        let parsed = rows(&line);
        assert_eq!(parsed.len(), 1);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn white_space_before_a_quote_is_dropped_once_and_across_lines() {
        // The blank state survives a line break inside a quoted field, like the rescan did.
        assert_eq!(rows("abc   \"d\"e"), vec![vec!["abc   \"d\"e"]]);
        assert_eq!(rows("\"    \"x\"y"), vec![vec!["x\"y\n"]]);
        assert_eq!(rows("\"  \n   \"x\"y"), vec![vec!["x\"y\n"]]);
    }

    #[test]
    fn fields_are_bounded() {
        let at_the_bound = "x".repeat(MAX_FIELD_BYTES);
        assert_eq!(rows(&at_the_bound)[0][0].len(), MAX_FIELD_BYTES);
        assert!(parse(&format!("{at_the_bound}x")).is_err());
        // Also when the field runs over several lines, and with escapes on the way.
        let lines = "\"".to_owned() + &"xxxxxxx\n".repeat(MAX_FIELD_BYTES / 8 + 1);
        assert!(parse(&lines).is_err());
        let doubled = "\"\"".repeat(MAX_FIELD_BYTES + 1);
        assert!(parse(&format!("\"{doubled}\"")).is_err());
    }

    #[test]
    fn rows_are_bounded() {
        let at_the_bound = ",".repeat(MAX_ROW_FIELDS - 1);
        assert_eq!(rows(&at_the_bound)[0].len(), MAX_ROW_FIELDS);
        assert!(parse(&format!("{at_the_bound},")).is_err());
        // Three fields within the field bound, over the row's.
        let field = "x".repeat(MAX_FIELD_BYTES);
        assert!(parse(&format!("{field},{field},{field}")).is_err());
        // Across lines as well, and the next row starts afresh.
        let field = "x".repeat(MAX_ROW_BYTES / 2);
        assert!(parse(&format!("{field}\n{field}\n{field}")).is_ok());
        assert!(parse(&format!("{field},\"{field}\n\",{field}")).is_err());
    }

    #[test]
    fn small_cells_are_bounded() {
        // Empty lines: rows cost far more memory than the one byte each takes here.
        assert_eq!(rows(&"\n".repeat(MAX_ROWS)).len(), MAX_ROWS);
        assert!(parse(&"\n".repeat(MAX_ROWS + 1)).is_err());
        // Empty cells, in rows of allowed width.
        let row = ",".repeat(MAX_ROW_FIELDS - 1) + "\n";
        assert!(parse(&row.repeat(MAX_CELLS / MAX_ROW_FIELDS)).is_ok());
        assert!(parse(&row.repeat(MAX_CELLS / MAX_ROW_FIELDS + 1)).is_err());
    }

    #[test]
    fn the_field_buffer_never_moves() {
        // Sized for the longest field the text can hold, whatever that field is.
        for text in [
            "",
            "a",
            "\"open",
            "\"\n\n\n",
            "\"a\nb\nc",
            "abc\ndef",
            "\u{1f389}",
        ] {
            let mut reader = Reader::new(text.len());
            let capacity = reader.field.capacity();
            for line in physical_lines(text) {
                reader.line(line).unwrap();
            }
            assert_eq!(reader.field.capacity(), capacity, "{text:?}");
        }
    }
}
