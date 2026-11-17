//! Stitching the pages of a Relay connection together, apart from the
//! requests that fetch them, so the walk can be checked without a server.

use std::collections::HashSet;

/// The pages of one connection taken in so far, and the cursors already
/// asked for.
#[derive(Debug)]
pub struct Stitch<P> {
    pages: Vec<P>,
    sent: HashSet<String>,
    limit: usize,
}

/// What to do after a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Ask for the page after this cursor.
    After(String),
    /// Stop: every page is in, or `cut` when the limit was reached while
    /// Linear still had more.
    Done { cut: bool },
}

/// Linear said there are more pages but gave no cursor it had not given
/// before, so following it could go round forever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepeatedCursor;

impl<P> Stitch<P> {
    /// A walk that stops after `limit` pages.
    pub fn new(limit: usize) -> Self {
        Self {
            pages: Vec::new(),
            sent: HashSet::new(),
            limit,
        }
    }

    /// Takes in a page and its `pageInfo`, and says what to ask for next.
    /// A page that points at no fresh cursor is refused and ends the walk.
    pub fn add(
        &mut self,
        page: P,
        has_next_page: bool,
        end_cursor: Option<&str>,
    ) -> Result<Next, RepeatedCursor> {
        let next = match (has_next_page, end_cursor) {
            (false, _) => None,
            (true, Some(cursor)) if self.sent.insert(cursor.to_owned()) => Some(cursor.to_owned()),
            (true, _) => return Err(RepeatedCursor),
        };
        self.pages.push(page);
        Ok(match next {
            Some(_) if self.pages.len() >= self.limit => Next::Done { cut: true },
            Some(cursor) => Next::After(cursor),
            None => Next::Done { cut: false },
        })
    }

    /// The pages taken in, in the order they came.
    pub fn into_pages(self) -> Vec<P> {
        self.pages
    }
}
