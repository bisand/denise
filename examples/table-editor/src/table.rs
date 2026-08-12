//! The data behind the editor: rows, validation and the file format.
//!
//! Deliberately separate from anything that draws. A record editor is mostly
//! *rules* — what a valid row is, what happens when one is deleted, whether there
//! are unsaved changes — and rules are the part worth testing. None of this file
//! knows a widget exists, which is why all of it is tested below and none of the
//! tests need a display.
//!
//! # Why CSV and not SQLite
//!
//! Because the interesting part of a database editor is the editing, and a table
//! of records reached through an index is the same shape whether it came from a
//! file or a query. Swapping this module for one that runs `SELECT` would not
//! change a line of `app.rs`, which is the point it is really making.

use std::fmt::Write as _;

/// One record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub role: String,
    /// Kept as text, because that is what the user typed. Validation is a
    /// question asked of the text, not a parse that throws it away — a field
    /// holding `3o` has to stay `3o` while somebody fixes the typo.
    pub age: String,
    pub city: String,
}

impl Row {
    /// The columns, in the order they are shown and stored.
    pub const COLUMNS: [&'static str; 4] = ["Name", "Role", "Age", "City"];

    /// One field by column index, for the grid.
    pub fn field(&self, column: usize) -> &str {
        match column {
            0 => &self.name,
            1 => &self.role,
            2 => &self.age,
            _ => &self.city,
        }
    }

    /// Why this row cannot be saved, or `None` if it can.
    ///
    /// One message at a time and the most important first: somebody fixing three
    /// problems wants to be told about the next one after fixing this one, not
    /// all three at once in a paragraph.
    pub fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("Name cannot be empty".into());
        }
        match self.age.trim() {
            "" => Some("Age cannot be empty".into()),
            age => match age.parse::<u32>() {
                Err(_) => Some(format!("Age must be a whole number, not {age:?}")),
                // A bound rather than none at all: the point is that validation
                // is a rule you write, and a rule with no edge cases teaches
                // nothing.
                Ok(years) if years > 130 => Some(format!("{years} is not a plausible age")),
                Ok(_) => None,
            },
        }
    }
}

/// Every row, plus whether the file on disk still matches.
#[derive(Clone, Debug, Default)]
pub struct Table {
    rows: Vec<Row>,
    /// Set by every mutation, cleared by a save. What makes the title bar say so
    /// and what makes quitting ask.
    dirty: bool,
}

impl Table {
    /// The rows, in order.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    /// Whether anything has changed since the last load or save.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Replaces one row.
    pub fn replace(&mut self, index: usize, row: Row) {
        if let Some(slot) = self.rows.get_mut(index) {
            // Only when it differs. Selecting a row, looking at it and selecting
            // another must not mark the file unsaved.
            if *slot != row {
                *slot = row;
                self.dirty = true;
            }
        }
    }

    /// Appends a row and returns its index.
    pub fn push(&mut self, row: Row) -> usize {
        self.rows.push(row);
        self.dirty = true;
        self.rows.len() - 1
    }

    /// Removes a row and reports which one should be selected afterwards.
    ///
    /// The answer is the row that took its place, or the new last row when the
    /// end was deleted, or `None` when nothing is left. Getting this wrong is how
    /// a list selects nothing after every delete and makes the user re-aim.
    pub fn remove(&mut self, index: usize) -> Option<usize> {
        if index >= self.rows.len() {
            return self.rows.len().checked_sub(1);
        }
        self.rows.remove(index);
        self.dirty = true;
        if self.rows.is_empty() {
            None
        } else {
            Some(index.min(self.rows.len() - 1))
        }
    }

    /// The first row that cannot be saved, with its index.
    pub fn first_problem(&self) -> Option<(usize, String)> {
        self.rows()
            .iter()
            .enumerate()
            .find_map(|(index, row)| row.problem().map(|why| (index, why)))
    }

    // ------------------------------------------------------------ the format

    /// Parses the whole file.
    ///
    /// A header line if one is present, then one record per line. Unknown extra
    /// columns are dropped and missing ones are empty, because a file somebody
    /// edited by hand should open rather than refuse.
    pub fn parse(text: &str) -> Self {
        let mut rows = Vec::new();
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fields = split_record(line);
            // A first line naming the columns is a header, not a person.
            if number == 0
                && fields
                    .first()
                    .is_some_and(|f| f.eq_ignore_ascii_case("name"))
            {
                continue;
            }
            let mut field = fields.into_iter();
            rows.push(Row {
                name: field.next().unwrap_or_default(),
                role: field.next().unwrap_or_default(),
                age: field.next().unwrap_or_default(),
                city: field.next().unwrap_or_default(),
            });
        }
        Self { rows, dirty: false }
    }

    /// Formats the whole file, header included.
    pub fn format(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{}", Row::COLUMNS.join(","));
        for row in &self.rows {
            let fields: Vec<String> = (0..Row::COLUMNS.len())
                .map(|column| quote(row.field(column)))
                .collect();
            let _ = writeln!(out, "{}", fields.join(","));
        }
        out
    }

    /// Marks the table as matching what is on disk.
    pub fn saved(&mut self) {
        self.dirty = false;
    }
}

/// Splits one record, honouring quotes.
///
/// A name with a comma in it is the reason this is not `line.split(',')`, and a
/// name with a quote in it is the reason `""` means one quote.
fn split_record(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if quoted => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' => quoted = true,
            ',' if !quoted => fields.push(core::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields.into_iter().map(|f| f.trim().to_string()).collect()
}

/// Quotes a field if it needs it, and only then.
fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// What a new file starts with, so the example has something to show.
pub const SAMPLE: &str = "\
Name,Role,Age,City
Jay Miner,Chip designer,52,Sunnyvale
Carolyn Scheppner,Systems programmer,44,Los Gatos
RJ Mical,Intuition,41,\"Palo Alto, CA\"
Dale Luck,Graphics,39,San Jose
Glenn Keller,Denise,37,Santa Clara
";

#[cfg(test)]
mod tests {
    use super::*;

    /// A file somebody edited by hand should open. Extra columns, missing
    /// columns, blank lines and a header that may or may not be there — all of it
    /// is ordinary, and refusing to load is the one unhelpful answer.
    #[test]
    fn a_hand_edited_file_still_opens() {
        let table =
            Table::parse("Name,Role,Age,City\nAda,,36\n\nGrace,Admiral,85,Arlington,extra\n");
        assert_eq!(table.len(), 2);
        assert_eq!(table.get(0).map(|r| r.name.as_str()), Some("Ada"));
        // A missing column is empty, not an error.
        assert_eq!(table.get(0).map(|r| r.city.as_str()), Some(""));
        // An extra one is dropped.
        assert_eq!(table.get(1).map(|r| r.city.as_str()), Some("Arlington"));
    }

    /// The header is only a header on the first line, and only when it looks like
    /// one. Somebody actually called "Name" is a person on line four.
    #[test]
    fn a_header_is_skipped_and_a_person_is_not() {
        assert_eq!(
            Table::parse("Name,Role,Age,City\nAda,,36,London\n").len(),
            1
        );
        assert_eq!(Table::parse("Ada,,36,London\n").len(), 1);
        assert_eq!(
            Table::parse("Name,Role,Age,City\nName,Clerk,40,Hull\n").len(),
            1
        );
    }

    /// The whole reason this is not `split(',')`: a field containing the
    /// separator, and a field containing the quote that protects it.
    #[test]
    fn a_comma_inside_a_field_survives_the_round_trip() {
        let mut table = Table::default();
        table.push(Row {
            name: "Doe, Jane".into(),
            role: "Says \"hello\"".into(),
            age: "40".into(),
            city: "Hull".into(),
        });

        let reparsed = Table::parse(&table.format());
        assert_eq!(reparsed.rows(), table.rows());
        assert_eq!(reparsed.get(0).map(|r| r.name.as_str()), Some("Doe, Jane"));
        assert_eq!(
            reparsed.get(0).map(|r| r.role.as_str()),
            Some("Says \"hello\"")
        );
    }

    #[test]
    fn the_sample_round_trips() {
        let table = Table::parse(SAMPLE);
        assert_eq!(table.len(), 5);
        assert!(!table.is_dirty(), "loading is not a change");
        assert_eq!(Table::parse(&table.format()).rows(), table.rows());
    }

    /// Selecting a row and looking at it is not an edit. A table that marks
    /// itself unsaved for that asks to save on the way out of a read.
    #[test]
    fn replacing_a_row_with_itself_is_not_a_change() {
        let mut table = Table::parse(SAMPLE);
        let unchanged = table.get(1).cloned().expect("a row");
        table.replace(1, unchanged);
        assert!(!table.is_dirty());

        let mut edited = table.get(1).cloned().expect("a row");
        edited.city = "Cupertino".into();
        table.replace(1, edited);
        assert!(table.is_dirty());
    }

    /// After a delete something sensible has to be selected, or the user re-aims
    /// after every one. Deleting from the middle selects what moved up; deleting
    /// the last selects the new last; deleting the only row selects nothing.
    #[test]
    fn deleting_leaves_a_sensible_selection() {
        let mut table = Table::parse(SAMPLE);
        assert_eq!(table.remove(1), Some(1));
        assert_eq!(table.remove(table.len() - 1), Some(table.len() - 1));

        let mut one = Table::default();
        one.push(Row::default());
        assert_eq!(one.remove(0), None);
        assert!(one.is_empty());
    }

    /// Validation is a question asked of the text. The field keeps what was
    /// typed — including a typo somebody is halfway through fixing.
    #[test]
    fn a_row_says_what_is_wrong_with_it_one_thing_at_a_time() {
        let ok = Row {
            name: "Ada".into(),
            age: "36".into(),
            ..Row::default()
        };
        assert_eq!(ok.problem(), None);

        let nameless = Row {
            age: "36".into(),
            ..Row::default()
        };
        assert!(nameless.problem().is_some_and(|p| p.contains("Name")));

        // The name is fixed, so the next problem is the one that surfaces.
        let typo = Row {
            name: "Ada".into(),
            age: "3o".into(),
            ..Row::default()
        };
        assert!(typo.problem().is_some_and(|p| p.contains("3o")));

        let ancient = Row {
            name: "Ada".into(),
            age: "900".into(),
            ..Row::default()
        };
        assert!(ancient.problem().is_some());
    }

    #[test]
    fn the_first_unsaveable_row_is_reported_with_its_index() {
        let mut table = Table::parse(SAMPLE);
        assert_eq!(table.first_problem(), None);
        table.push(Row {
            name: "".into(),
            age: "1".into(),
            ..Row::default()
        });
        assert_eq!(table.first_problem().map(|(index, _)| index), Some(5));
    }
}
