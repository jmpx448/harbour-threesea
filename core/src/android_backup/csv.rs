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

/// One logical row per `Vec<String>`; empty input gives no rows. Equivalent to reading every
/// row with `CSVReader.readNext()` (multi-line aware).
pub fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut parser = Parser::default();
    let mut row: Vec<String> = Vec::new();
    for line in physical_lines(text) {
        row.extend(parser.parse_line_multi(line));
        if !parser.is_pending() {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            row = Vec::new();
        }
    }
    // An unterminated quote at the end of the file: Java would keep waiting for more lines and
    // return the pending text as a final field once the stream ends.
    if let Some(pending) = parser.pending.take() {
        row.push(pending);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
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
/// `ignoreLeadingWhiteSpace` on), `parseLineMulti` semantics.
#[derive(Default)]
struct Parser {
    pending: Option<String>,
}

impl Parser {
    fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    #[allow(clippy::too_many_lines)]
    fn parse_line_multi(&mut self, line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        let mut tokens: Vec<String> = Vec::new();
        let mut sb = String::new();
        let mut in_quotes = false;
        if let Some(pending) = self.pending.take() {
            sb = pending;
            in_quotes = true;
        }
        let mut in_field = false;
        let next_is =
            |i: usize, wanted: &[char]| chars.get(i + 1).is_some_and(|next| wanted.contains(next));
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == ESCAPE {
                // Escapable (a quote or another backslash follows, inside a field): take the
                // next character literally. Otherwise the backslash is swallowed, like OpenCSV.
                if (in_quotes || in_field) && next_is(i, &[QUOTE, ESCAPE]) {
                    sb.push(chars[i + 1]);
                    i += 1;
                }
            } else if c == QUOTE {
                if (in_quotes || in_field) && next_is(i, &[QUOTE]) {
                    // A doubled quote inside a field: one literal quote.
                    sb.push(chars[i + 1]);
                    i += 1;
                } else {
                    // OpenCSV keeps a quote that sits in the middle of a field (not next to a
                    // separator); leading white space before it is discarded.
                    if i > 2
                        && chars[i - 1] != SEPARATOR
                        && chars.len() > i + 1
                        && chars[i + 1] != SEPARATOR
                    {
                        if !sb.is_empty() && sb.chars().all(char::is_whitespace) {
                            sb.clear();
                        } else {
                            sb.push(c);
                        }
                    }
                    in_quotes = !in_quotes;
                }
                in_field = !in_field;
            } else if c == SEPARATOR && !in_quotes {
                tokens.push(std::mem::take(&mut sb));
                in_field = false;
            } else {
                sb.push(c);
                in_field = true;
            }
            i += 1;
        }
        if in_quotes {
            // The quoted field continues on the next physical line.
            sb.push('\n');
            self.pending = Some(sb);
        } else {
            tokens.push(sb);
        }
        tokens
    }
}

/// A header-keyed CSV file (every data file but `settings`): fields are found by column name,
/// never by position (`CSVRow.getValuePosition`); missing columns are `None`, so version gates
/// decide what must exist.
pub struct Table {
    header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// The first row is the header; files without rows are valid and empty.
    pub fn parse(text: &str) -> Self {
        let mut rows = parse(text);
        let header = if rows.is_empty() {
            Vec::new()
        } else {
            rows.remove(0)
        };
        Self { header, rows }
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

    #[test]
    fn plain_rows() {
        let rows = parse("\"a\",\"b\"\n\"c\",\"d\"\n");
        assert_eq!(rows, vec![vec!["a", "b"], vec!["c", "d"]]);
        // Unquoted values and empty cells work too (settings uses plain two-column rows).
        assert_eq!(parse("version,27\n"), vec![vec!["version", "27"]]);
        assert_eq!(parse("\"a\",,\"c\"\n"), vec![vec!["a", "", "c"]]);
        assert!(parse("").is_empty());
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
        let rows = parse(&file);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], original);
    }

    #[test]
    fn crlf_and_multiline() {
        // CRLF line ends; an embedded break always comes back as plain \n (Java readLine).
        let file = "\"a\r\nb\",\"c\"\r\n\"next\"\r\n";
        let rows = parse(file);
        assert_eq!(rows, vec![vec!["a\nb", "c"], vec!["next"]]);
        // Unterminated quote at the end of the file: the pending text (with the continuation
        // break OpenCSV already appended) is the last field.
        assert_eq!(parse("\"open"), vec![vec!["open\n"]]);
    }

    #[test]
    fn opencsv_quirks() {
        // A lone backslash inside quotes is swallowed (OpenCSV's escape handling); the writer
        // never produces one, but a parser difference here would corrupt text silently.
        assert_eq!(parse("\"a\\b\""), vec![vec!["ab"]]);
        // A quote in the middle of a field stays.
        assert_eq!(parse("\"midd\"\"le\""), vec![vec!["midd\"le"]]);
        assert_eq!(parse("abc\"de\"f"), vec![vec!["abc\"de\"f"]]);
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
        let table = Table::parse(&file);
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
        let empty = Table::parse("");
        assert!(empty.rows.is_empty());
    }
}
