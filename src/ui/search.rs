//! The search overlay: the query, its options, and how far its search is.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{palette, style};
use crate::app::{Action, Phase, Search, State};
use crate::keys::Context;

pub fn render(state: &State, search: &Search, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let list = palette::finder(state, " Search Linear ", "/", &search.query, frame, area);
    let mut lines = vec![option(
        state,
        search.include_comments,
        "Include comments".to_owned(),
        Action::ToggleEntry,
    )];
    if let Some(scope) = &search.scope {
        lines.push(option(
            state,
            scope.on,
            scope.label(),
            Action::ToggleSearchScope,
        ));
    }
    lines.push(Line::default());
    lines.push(match &search.phase {
        Phase::Typing {
            problem: Some(problem),
        } => Line::styled(format!(" {problem}"), style::error(theme)),
        Phase::Typing { problem: None } => {
            let within = search.scope.as_ref().filter(|scope| scope.on);
            let text = match within {
                Some(scope) if scope.filters.is_empty() => {
                    format!(" Enter searches every {} issue.", scope.team_key)
                }
                Some(scope) => format!(
                    " Enter searches the {} issues these filters match.",
                    scope.team_key
                ),
                None => " Enter searches every issue in the workspace.".to_owned(),
            };
            Line::styled(text, style::muted(theme))
        }
        Phase::Searching { query } => Line::styled(
            format!(" Searching for \"{}\"...", query.term),
            style::muted(theme),
        ),
        Phase::NothingFound { term } => {
            Line::styled(format!(" Nothing matches \"{term}\"."), style::muted(theme))
        }
    });
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list);
}

/// One option under the query: its mark and label, accented when on and
/// muted when off, then the key that turns it on or off.
fn option(state: &State, on: bool, label: String, toggle: Action) -> Line<'static> {
    let theme = &state.theme;
    let (mark, look) = if on {
        ("[x]", style::accent(theme))
    } else {
        ("[ ]", style::muted(theme))
    };
    let mut spans = vec![
        Span::styled(format!(" {mark} "), look),
        Span::styled(label, if on { Default::default() } else { look }),
    ];
    if let Some(keys) = state
        .keymap
        .keys_for(toggle, &[Context::Overlay])
        .into_iter()
        .find(|keys| keys.keys().first().is_none_or(|key| key.typed().is_none()))
    {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(keys.to_string(), style::accent(theme)));
    }
    Line::from(spans)
}
