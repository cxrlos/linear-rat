//! Opening what an issue links to. One picker offers where the issue lives:
//! its Linear page and the GitHub pull requests attached to it. The other
//! offers every other link: those in its description and comments, and its
//! other attachments, each once and labelled with where it was found.

use super::detail::{self, DetailLoad};
use super::picker::{Entry, Note, Picker};
use super::{Effect, Modal, State, Store, Update};
use crate::linear::model::{Attachment, Issue};
use crate::ui::markdown::{self, FoundLink};

/// Which links a picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSet {
    /// The issue's Linear page, then each linked GitHub pull request.
    Issue,
    /// Every other link: description, comments and other attachments.
    Other,
}

/// Where an entry goes, and how the notice names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub name: String,
}

/// An open picker of one issue's links.
#[derive(Debug, Clone, PartialEq)]
pub struct Links {
    pub issue_id: String,
    pub identifier: String,
    pub set: LinkSet,
    pub list: Picker<Link>,
}

impl Links {
    /// What the picker's border says, as `Open ENG-12`.
    pub fn title(&self) -> String {
        match self.set {
            LinkSet::Issue => format!("Open {}", self.identifier),
            LinkSet::Other => format!("Links of {}", self.identifier),
        }
    }

    /// Fills the list from what `state` holds, with a note while the detail
    /// that brings the links is loading or after it failed.
    fn fill(&mut self, state: &State) {
        let Some(issue) = state.store.issue(&self.issue_id) else {
            return;
        };
        self.list
            .set_entries(entries(&state.store, issue, self.set), None);
        self.list.set_note(match state.detail.load(&self.issue_id) {
            Some(DetailLoad::Loading) => Some(Note::Loading),
            Some(DetailLoad::Failed(error)) => Some(Note::Failed(error.clone())),
            Some(DetailLoad::Loaded) | None => None,
        });
    }
}

/// The entries `set` offers for `issue`, from what `store` has loaded.
fn entries(store: &Store, issue: &Issue, set: LinkSet) -> Vec<Entry<Link>> {
    let attachments = store.attachments(&issue.id).unwrap_or_default();
    let pull_requests = attachments.iter().filter(|a| a.is_pull_request());
    match set {
        LinkSet::Issue => {
            let page = Entry {
                label: format!("Linear: {} {}", issue.identifier, issue.title),
                value: Link {
                    url: issue.url.clone(),
                    name: issue.identifier.clone(),
                },
            };
            std::iter::once(page)
                .chain(pull_requests.map(pull_request))
                .collect()
        }
        LinkSet::Other => {
            let mut seen: Vec<String> = pull_requests.map(|a| a.url.clone()).collect();
            let mut entries = Vec::new();
            for (source, link) in other_links(store, issue, attachments) {
                if is_issue_page(&link.url, issue) || seen.contains(&link.url) {
                    continue;
                }
                seen.push(link.url.clone());
                let label = match &link.text {
                    Some(text) => format!("{source} · {text} · {}", link.url),
                    None => format!("{source} · {}", link.url),
                };
                entries.push(Entry {
                    label,
                    value: Link {
                        name: link.url.clone(),
                        url: link.url,
                    },
                });
            }
            entries
        }
    }
}

/// Every link of `issue` beyond its pull requests, repeats included, with
/// where it was found: its description, then its comments oldest first,
/// then its other attachments.
fn other_links(
    store: &Store,
    issue: &Issue,
    attachments: &[Attachment],
) -> Vec<(String, FoundLink)> {
    let description = issue.description.as_deref().unwrap_or_default();
    let mut found: Vec<(String, FoundLink)> = markdown::links(description)
        .into_iter()
        .map(|link| ("description".to_owned(), link))
        .collect();
    for comment in store.comments(&issue.id).unwrap_or_default() {
        let source = comment
            .author_id
            .as_deref()
            .and_then(|id| store.member(id))
            .map_or_else(
                || "comment".to_owned(),
                |author| format!("comment by {}", author.name),
            );
        found.extend(
            markdown::links(&comment.body)
                .into_iter()
                .map(|link| (source.clone(), link)),
        );
    }
    for attachment in attachments.iter().filter(|a| !a.is_pull_request()) {
        let source = match attachment.source_type.as_deref() {
            Some(source) if source != "unknown" => format!("attachment: {source}"),
            _ => "attachment".to_owned(),
        };
        let link = FoundLink {
            url: attachment.url.clone(),
            text: Some(attachment.title.clone()),
        };
        found.push((source, link));
    }
    found
}

/// A pull request's entry, as `Pull request nibble#412: Title (merged)`.
fn pull_request(attachment: &Attachment) -> Entry<Link> {
    let reference = attachment
        .url
        .split_once("/pull/")
        .and_then(|(repo, rest)| {
            let repo = repo.rsplit('/').next()?;
            let number = rest.split(['/', '?', '#']).next()?;
            (!repo.is_empty() && !number.is_empty()).then(|| format!("{repo}#{number}"))
        });
    let mut label = match &reference {
        Some(reference) => format!("Pull request {reference}: {}", attachment.title),
        None => format!("Pull request: {}", attachment.title),
    };
    if let Some(status) = &attachment.status {
        label.push_str(&format!(" ({status})"));
    }
    Entry {
        label,
        value: Link {
            url: attachment.url.clone(),
            name: reference.unwrap_or_else(|| attachment.url.clone()),
        },
    }
}

/// Whether `url` is `issue`'s own Linear page, with or without the title
/// slug, an anchor or a query after its key.
fn is_issue_page(url: &str, issue: &Issue) -> bool {
    let key = format!("/issue/{}", issue.identifier);
    let page = issue
        .url
        .find(&key)
        .map_or(issue.url.as_str(), |at| &issue.url[..at + key.len()]);
    url.strip_prefix(page)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '#', '?']))
}

/// Opens the picker of `set` for the focused issue, and loads its detail
/// first if it has not loaded, refilling the picker when it arrives.
pub(super) fn open(state: &mut State, set: LinkSet) -> Update {
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    // A stand-in has no page and no detail to load until Linear confirms it.
    if super::is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; open it once Linear confirms it",
            issue.identifier
        ));
        return Update::redraw();
    }
    let issue_id = issue.id.clone();
    let mut links = Links {
        issue_id: issue_id.clone(),
        identifier: issue.identifier.clone(),
        set,
        list: Picker::new(Vec::new(), None),
    };
    let effects = detail::load_now(state, &issue_id).into_iter().collect();
    links.fill(state);
    state.modals.push(Modal::Links(links));
    Update {
        redraw: true,
        effects,
    }
}

/// Moves the open picker's selection by `step`.
pub(super) fn select(state: &mut State, step: isize) -> Update {
    match state.modals.top_mut() {
        Some(Modal::Links(links)) => Update {
            redraw: links.list.select_by(step),
            effects: Vec::new(),
        },
        _ => Update::default(),
    }
}

/// Closes the open picker on the selected entry, or the `nth` visible one
/// when given, and asks to open its URL.
pub(super) fn choose(state: &mut State, nth: Option<usize>) -> Update {
    let Some(Modal::Links(links)) = state.modals.top() else {
        return Update::default();
    };
    let chosen = match nth {
        Some(n) => links.list.nth(n),
        None => links.list.selected(),
    };
    let Some(Link { url, name }) = chosen.cloned() else {
        return Update::default();
    };
    state.modals.pop();
    state.notice = Some(format!("Opening {name} in the browser"));
    Update {
        redraw: true,
        effects: vec![Effect::OpenUrl(url)],
    }
}

/// Refills the open pickers of `issue_id` once its detail has loaded or
/// failed to.
pub(super) fn loaded(state: &mut State, issue_id: &str) {
    let mut modals = std::mem::take(&mut state.modals);
    for modal in modals.iter_mut() {
        if let Modal::Links(links) = modal
            && links.issue_id == issue_id
        {
            links.fill(state);
        }
    }
    state.modals = modals;
}
