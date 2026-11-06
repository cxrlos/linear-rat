//! Opening what an issue links to, driven through the app with the demo
//! workspace: `o` for the Linear page and linked pull requests, `O` for
//! every other link. Choosing asks the runtime to open the URL; nothing
//! here opens a browser.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, Modal, State};
use linear_rat::linear::{FakeClient, IssueDetail, LinearClient, Urgency};
use ratatui::backend::TestBackend;

const DES_34: &str = "https://linear.app/squeakworks/issue/DES-34";

fn key_event(c: char) -> Event {
    let code = match c {
        '\n' => KeyCode::Enter,
        '\x1b' => KeyCode::Esc,
        c => KeyCode::Char(c),
    };
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Presses each key of `keys` (`\n` is Enter, `\x1b` Esc) and returns every
/// effect they asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key_event(c)).unwrap())
        .collect()
}

/// My Issues on the demo workspace, focused on the issue with `identifier`.
fn focused_on(identifier: &str) -> App<TestBackend> {
    let mut state = State::demo();
    let id = format!("issue-{}", identifier.to_lowercase());
    assert!(
        state.my_issues.focus_id(&id),
        "{identifier} is not in My Issues"
    );
    App::new(TestBackend::new(100, 24), state).unwrap()
}

/// Answers every detail load in `effects` with the demo workspace and
/// returns the other effects.
fn answer(app: &mut App<TestBackend>, effects: Vec<Effect>) -> Vec<Effect> {
    let mut rest = Vec::new();
    for effect in effects {
        match effect {
            Effect::LoadIssueDetail { issue_id, .. } => {
                let result = Ok(IssueDetail::demo(&issue_id));
                rest.extend(
                    app.handle(Event::IssueDetailLoaded { issue_id, result })
                        .unwrap(),
                );
            }
            other => rest.push(other),
        }
    }
    rest
}

/// Presses `keys` and answers the detail loads they ask for.
fn open(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    let effects = press(app, keys);
    answer(app, effects)
}

/// The labels of the open links picker, in order.
fn entries(app: &App<TestBackend>) -> Vec<String> {
    match app.state().modals.top() {
        Some(Modal::Links(links)) => links
            .list
            .visible()
            .into_iter()
            .map(|(_, entry)| entry.label.clone())
            .collect(),
        other => panic!("no links picker is open: {other:?}"),
    }
}

/// The notice rows above the two-row bottom bar, joined and trimmed.
fn notice(app: &App<TestBackend>) -> String {
    let buffer = app.backend().buffer();
    let rows = buffer.area.height;
    (rows.saturating_sub(5)..rows - 2)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|line| !line.starts_with(['│', '└', '┌']))
        .collect::<String>()
        .trim()
        .to_owned()
}

#[test]
fn o_offers_the_linear_page_then_each_linked_pull_request() {
    let mut app = focused_on("DES-34");

    open(&mut app, "o");

    assert_eq!(
        entries(&app),
        [
            "Linear: DES-34 Design QA for the autumn release",
            "Pull request nibble#412: Autumn release spacing fixes (merged)",
            "Pull request nibble#418: Tidy the release screenshots (open)",
        ]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn o_offers_the_linear_page_alone_when_no_pull_request_is_linked() {
    let mut app = focused_on("DES-40");

    open(&mut app, "o");

    assert_eq!(
        entries(&app),
        ["Linear: DES-40 Color-blind safe workflow markers"]
    );
}

#[test]
fn o_shows_the_linear_page_at_once_and_adds_pull_requests_when_the_detail_loads() {
    let mut app = focused_on("DES-34");

    let effects = press(&mut app, "o");

    assert_eq!(
        effects,
        [Effect::LoadIssueDetail {
            issue_id: "issue-des-34".to_owned(),
            urgency: Urgency::Urgent,
        }]
    );
    assert_eq!(
        entries(&app),
        ["Linear: DES-34 Design QA for the autumn release"]
    );
    answer(&mut app, effects);
    assert_eq!(entries(&app).len(), 3);
}

#[test]
fn a_failed_load_says_why_in_the_picker_and_the_next_o_tries_again() {
    let mut app = focused_on("DES-34");
    press(&mut app, "o");

    app.handle(Event::IssueDetailLoaded {
        issue_id: "issue-des-34".to_owned(),
        result: Err("Linear could not be reached".to_owned()),
    })
    .unwrap();

    let shown = format!("{:?}", app.backend());
    assert!(
        shown.contains("Could not load: Linear could not be reached"),
        "{shown}"
    );
    assert_eq!(entries(&app).len(), 1);
    press(&mut app, "\x1b");
    let effects = press(&mut app, "o");
    assert_eq!(effects.len(), 1, "{effects:?}");
}

#[test]
fn choosing_an_entry_by_number_asks_to_open_it() {
    let mut app = focused_on("DES-34");
    open(&mut app, "o");

    let effects = press(&mut app, "2");

    assert_eq!(
        effects,
        [Effect::OpenUrl(
            "https://github.com/squeakworks/nibble/pull/412".to_owned()
        )]
    );
    assert!(app.state().modals.top().is_none());
    assert_eq!(notice(&app), "Opening nibble#412 in the browser");
}

#[test]
fn enter_opens_the_linear_page_first() {
    let mut app = focused_on("DES-34");
    open(&mut app, "o");

    let effects = press(&mut app, "\n");

    assert_eq!(effects, [Effect::OpenUrl(DES_34.to_owned())]);
    assert_eq!(notice(&app), "Opening DES-34 in the browser");
}

#[test]
fn typing_filters_the_picker_and_enter_opens_what_is_left() {
    let mut app = focused_on("DES-34");
    open(&mut app, "o");

    press(&mut app, "tidy");

    assert_eq!(
        entries(&app),
        ["Pull request nibble#418: Tidy the release screenshots (open)"]
    );
    assert_eq!(
        press(&mut app, "\n"),
        [Effect::OpenUrl(
            "https://github.com/squeakworks/nibble/pull/418".to_owned()
        )]
    );
}

#[test]
fn esc_closes_the_picker_without_opening_anything() {
    let mut app = focused_on("DES-34");
    open(&mut app, "O");

    let effects = press(&mut app, "\x1b");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.state().modals.top().is_none());
}

#[test]
fn shift_o_offers_the_other_attachments_labelled_by_source() {
    let mut app = focused_on("DES-34");

    open(&mut app, "O");

    assert_eq!(
        entries(&app),
        [
            "attachment: figma · Autumn QA board · https://www.figma.com/file/nibble-autumn-qa",
            "attachment: slack · Thread in #design · \
             https://squeakworks.slack.com/archives/C042DESIGN/p1793400000",
            "attachment: github · Screenshots are blurry on tablets · \
             https://github.com/squeakworks/nibble/issues/377",
        ]
    );
    assert_snapshot!(app.backend());
}

#[test]
fn shift_o_lists_description_and_comment_links_once_without_the_issue_or_its_pull_requests() {
    let mut app = focused_on("DES-16");
    open(&mut app, "O");

    assert_eq!(
        entries(&app),
        [
            "description · the focus appearance criterion · \
             https://www.w3.org/WAI/WCAG22/Understanding/focus-appearance",
            "description · https://www.figma.com/file/whisker-focus-rings",
            "description · https://linear.app/squeakworks/issue/DES-31",
            "description · https://whisker.squeakworks.example/focus",
            "comment by Pia Brie · https://whisker.squeakworks.example/focus-rings",
        ]
    );
    let effects = press(&mut app, "4");
    assert_eq!(
        effects,
        [Effect::OpenUrl(
            "https://whisker.squeakworks.example/focus".to_owned()
        )]
    );
}

#[test]
fn o_on_an_issue_with_a_draft_pull_request_offers_it_with_its_state() {
    let mut app = focused_on("DES-16");

    open(&mut app, "o");

    assert_eq!(
        entries(&app),
        [
            "Linear: DES-16 Accessible focus rings across whisker",
            "Pull request whisker#88: Focus ring tokens (draft)",
        ]
    );
}

#[test]
fn both_pickers_open_from_the_detail_too() {
    let mut app = focused_on("DES-34");
    let effects = press(&mut app, "\n");
    answer(&mut app, effects);

    let effects = press(&mut app, "o");

    assert!(
        effects.is_empty(),
        "the detail has loaded already: {effects:?}"
    );
    assert_eq!(entries(&app).len(), 3);
    press(&mut app, "\x1bO");
    assert_eq!(entries(&app).len(), 3);
}

#[tokio::test]
async fn both_pickers_work_on_team_issues() {
    let mut app = App::new(TestBackend::new(100, 24), State::demo()).unwrap();
    let mut effects = press(&mut app, "2");
    // Carry out the Team Issues loads until the list is in.
    while let Some(effect) = effects.pop() {
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(FakeClient.teams().await.unwrap())),
            Effect::LoadTeamOptions(team_id) => {
                Event::TeamOptionsLoaded(Ok(FakeClient.team_options(&team_id).await.unwrap()))
            }
            Effect::LoadTeamIssues { query, after } => Event::TeamIssuesLoaded {
                result: Ok(FakeClient
                    .team_issues(&query, after.as_deref())
                    .await
                    .unwrap()),
                query,
                after,
            },
            Effect::CountTeamIssues(query) => Event::TeamIssuesCounted {
                result: Ok(FakeClient.count_team_issues(&query).await.unwrap()),
                query,
            },
            _ => continue,
        };
        effects.extend(app.handle(event).unwrap());
    }
    let focused = app.state().focused_issue().unwrap().clone();

    open(&mut app, "o");
    assert_eq!(
        entries(&app),
        [format!("Linear: {} {}", focused.identifier, focused.title)]
    );
    assert_eq!(
        press(&mut app, "\n"),
        [Effect::OpenUrl(focused.url.clone())]
    );
    open(&mut app, "O");
    assert!(matches!(app.state().modals.top(), Some(Modal::Links(_))));
}

#[test]
fn nothing_opens_without_a_focused_issue() {
    let mut app = App::new(TestBackend::new(80, 24), State::default()).unwrap();

    let effects = press(&mut app, "oO");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.state().modals.top().is_none());
}
