//! What My Issues shows until its issues arrive: a loading message, or the
//! error screen that says why they cannot load and how to fix it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};

use super::style;
use crate::app::LoadError;
use crate::auth::{AuthError, KeySource};
use crate::linear::ClientError;

const TITLE: &str = "My Issues";

pub fn render_loading(frame: &mut Frame, area: Rect) {
    let block = Block::bordered().title(TITLE);
    let loading = Paragraph::new("Loading your issues from Linear…")
        .style(style::muted())
        .block(block);
    frame.render_widget(loading, area);
}

/// The error screen for `error`. A rejected key is explained with
/// `key_source`, where it came from.
pub fn render(error: &LoadError, key_source: Option<&KeySource>, frame: &mut Frame, area: Rect) {
    let (heading, explanation) = explain(error, key_source);
    let lines = vec![
        Line::from(Span::styled(heading, style::error())),
        Line::default(),
        Line::from(explanation),
    ];
    let block = Block::bordered().title(TITLE).padding(Padding::uniform(1));
    let screen = Paragraph::new(lines).wrap(Wrap { trim: true }).block(block);
    frame.render_widget(screen, area);
}

/// A short heading and the full explanation with its fix. The key problems
/// reuse the messages of `lrat auth`.
fn explain(error: &LoadError, key_source: Option<&KeySource>) -> (&'static str, String) {
    match error {
        LoadError::NoKey => ("Not logged in", sentence(&AuthError::NoKey.to_string())),
        LoadError::KeyRejected => {
            let message = match key_source {
                Some(source) => AuthError::Rejected(source.clone()).to_string(),
                None => ClientError::InvalidKey.to_string(),
            };
            ("API key rejected", sentence(&message))
        }
        LoadError::KeyringUnavailable(message) => ("OS keyring unavailable", sentence(message)),
        LoadError::KeyUnusable(message) => ("Cannot use the API key", sentence(message)),
        LoadError::Unreachable(message) => (
            "Cannot reach Linear",
            format!(
                "{} Check the network connection and any proxy, then restart lrat.",
                sentence(message)
            ),
        ),
        LoadError::Failed(message) => (
            "Could not load your issues",
            format!(
                "{} Restart lrat to try again; the log has the details.",
                sentence(message)
            ),
        ),
    }
}

/// `message` with a capital first letter and a closing full stop.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    let mut text: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}
