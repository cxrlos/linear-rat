//! Reactions in the issue detail through the app driver: counts under the
//! description and each comment, and the `+` picker that adds the viewer's
//! reaction or removes it when given again, optimistic with rollback.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    App, Effect, Event, Failure, Modal, ReactionChange, ReactionWrite, State, WriteClass,
};
use linear_rat::linear::model::Reaction;
use linear_rat::linear::{
    ClientError, FakeClient, GraphQlError, IssueDetail, LinearClient, ReactionTarget, Urgency,
};
use ratatui::backend::TestBackend;
use ratatui::style::Color;

const FIRST: &str = "DES-34";
const ISSUE: &str = "issue-des-34";
/// Nell's comment, with the viewer's 👀 and Pia's 👍.
const OPEN: &str = "comment-90";
/// The viewer's 🎉 on DES-34.
const OWN_PARTY: &str = "reaction-des-34-3";

const FORBIDDEN: &str = r#"{
    "message": "You don't have permission to react",
    "extensions": { "type": "forbidden", "code": "FORBIDDEN", "userError": true }
}"#;

fn app_sized(columns: u16, rows: u16) -> App<TestBackend> {
    let mut app = App::new(TestBackend::new(columns, rows), State::demo()).unwrap();
    for effect in app.handle(key(KeyCode::Enter)).unwrap() {
        if let Effect::LoadIssueDetail { issue_id, .. } = effect {
            let result = Ok(IssueDetail::demo(&issue_id));
            app.handle(Event::IssueDetailLoaded { issue_id, result })
                .unwrap();
        }
    }
    app
}

fn app() -> App<TestBackend> {
    app_sized(80, 24)
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(KeyCode::Char(c))).unwrap())
        .collect()
}

fn focus(app: &mut App<TestBackend>, id: &str) {
    for _ in 0..10 {
        if app.state().detail.focused_comment(ISSUE) == Some(id) {
            return;
        }
        press(app, "}");
    }
    panic!("never focused {id}");
}

/// Opens the picker and picks its `nth` emoji, returning the one write.
fn react(app: &mut App<TestBackend>, nth: usize) -> ReactionWrite {
    assert_eq!(press(app, "+"), Vec::new());
    let effects = press(app, &nth.to_string());
    match effects.as_slice() {
        [Effect::React(write)] => write.clone(),
        other => panic!("expected one reaction change, got {other:?}"),
    }
}

fn reactions(app: &App<TestBackend>, target: &ReactionTarget) -> Vec<(String, String)> {
    app.state()
        .store
        .reactions(target)
        .iter()
        .map(|reaction| (reaction.emoji.clone(), reaction.id.clone()))
        .collect()
}

fn emoji(app: &App<TestBackend>, target: &ReactionTarget) -> Vec<String> {
    reactions(app, target).into_iter().map(|(e, _)| e).collect()
}

fn issue() -> ReactionTarget {
    ReactionTarget::Issue(ISSUE.to_owned())
}

fn screen(app: &App<TestBackend>) -> Vec<String> {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    let mut rows = Vec::new();
    for row in buffer.content().chunks(width) {
        let mut line = String::new();
        let mut skip = 0;
        for cell in row {
            // A wide character's second cell is blank in the buffer.
            if skip > 0 {
                skip -= 1;
                continue;
            }
            line.push_str(cell.symbol());
            skip = unicode_width::UnicodeWidthStr::width(cell.symbol()).saturating_sub(1);
        }
        rows.push(line);
    }
    rows
}

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    screen(app).iter().any(|line| line.contains(text))
}

fn forbidden(action: &str) -> Failure {
    let error: GraphQlError = serde_json::from_str(FORBIDDEN).unwrap();
    Failure::from_client(action, &ClientError::GraphQl(vec![error]))
}

fn on_fake<T: Send>(request: impl AsyncFnOnce(FakeClient) -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(request(FakeClient))
            })
            .join()
            .unwrap()
    })
}

#[test]
fn counts_show_under_the_description_and_under_each_comment() {
    let mut app = app();
    assert!(shows(&app, "👍 2  🎉 1"), "{:#?}", screen(&app));

    focus(&mut app, OPEN);

    assert!(shows(&app, "  👀 1  👍 1"), "{:#?}", screen(&app));
    // A wide emoji leaves the old text in the cell it covers in the test
    // backend; a full redraw shows the screen as a terminal would.
    app.redraw_all().unwrap();
    assert_snapshot!(app.backend());
}

/// The color of `emoji` on the first row that shows `line`.
fn emoji_fg(app: &App<TestBackend>, line: &str, emoji: &str) -> Color {
    let y = screen(app)
        .iter()
        .position(|row| row.contains(line))
        .unwrap();
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer.content()[y * width..(y + 1) * width]
        .iter()
        .find(|cell| cell.symbol() == emoji)
        .unwrap()
        .fg
}

#[test]
fn the_viewers_reactions_are_accented_and_everyone_elses_muted() {
    let mut app = app();
    let theme = app.state().theme.clone();

    // The viewer gave the 🎉 on the issue and neither 👍.
    assert_eq!(emoji_fg(&app, "👍 2  🎉 1", "🎉"), theme.accent);
    assert_eq!(emoji_fg(&app, "👍 2  🎉 1", "👍"), theme.muted);

    // Once the viewer adds 👍 too, its count is theirs as well.
    react(&mut app, 1);
    assert_eq!(emoji_fg(&app, "👍 3  🎉 1", "👍"), theme.accent);
}

#[test]
fn counts_cut_at_a_two_cell_emoji_end_in_an_ellipsis() {
    let mut app = app_sized(40, 24);
    // 👎 😄 😕 ❤️ 🚀 after the demo's 👍 and 🎉, so the seventh emoji
    // starts one cell before the edge.
    for nth in [2, 3, 5, 6, 7] {
        react(&mut app, nth);
    }

    assert!(
        shows(&app, "👍 2  🎉 1  👎 1  😄 1  😕 1  ❤️ 1  …"),
        "{:#?}",
        screen(&app)
    );
    // A wide emoji leaves the old text in the cell it covers in the test
    // backend; a full redraw shows the screen as a terminal would.
    app.redraw_all().unwrap();
    assert_snapshot!(app.backend());
}

#[test]
fn the_picker_lists_eight_emoji_and_marks_the_viewers() {
    let mut app = app();

    press(&mut app, "+");

    assert!(matches!(app.state().modals.top(), Some(Modal::Choice(_))));
    assert!(shows(&app, "🎉 party · yours, removes it"));
    assert!(shows(&app, "👍 thumbs up"));
    assert!(!shows(&app, "👍 thumbs up · yours"));
    // A wide emoji leaves the old text in the cell it covers in the test
    // backend; a full redraw shows the screen as a terminal would.
    app.redraw_all().unwrap();
    assert_snapshot!(app.backend());
}

#[test]
fn picking_one_the_viewer_gave_removes_it_from_the_issue_at_once() {
    let mut app = app();

    let write = react(&mut app, 4);

    let ReactionChange::Remove(removed) = &write.change else {
        panic!("{write:?}");
    };
    assert_eq!(removed.id, OWN_PARTY);
    assert_eq!(write.target, issue());
    assert_eq!(emoji(&app, &issue()), ["👍", "👍"]);
    app.handle(Event::Reacted {
        write,
        result: Ok(None),
    })
    .unwrap();
    assert_eq!(emoji(&app, &issue()), ["👍", "👍"]);
}

#[test]
fn a_removal_linear_refuses_puts_the_reaction_back() {
    let mut app = app();
    let write = react(&mut app, 4);
    let before = app.state().errors.len();

    app.handle(Event::Reacted {
        write,
        result: Err(forbidden(&format!("react on {FIRST}"))),
    })
    .unwrap();

    assert_eq!(emoji(&app, &issue()), ["👍", "👍", "🎉"]);
    assert_eq!(app.state().errors.len(), before + 1);
    let team_id = &app.state().store.issue(ISSUE).unwrap().team_id;
    assert!(
        app.state()
            .store
            .capabilities()
            .write(team_id, WriteClass::React)
            .is_denied()
    );
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with(&format!("Could not remove your 🎉 on {FIRST}"))
    );
}

#[test]
fn picking_a_new_one_adds_it_to_the_focused_comment_until_linear_stores_it() {
    let mut app = app();
    focus(&mut app, OPEN);
    let comment = ReactionTarget::Comment(OPEN.to_owned());

    let write = react(&mut app, 4);

    let ReactionChange::Add(stand_in) = &write.change else {
        panic!("{write:?}");
    };
    assert_eq!(write.target, comment);
    assert_eq!(emoji(&app, &comment), ["👀", "👍", "🎉"]);
    let stored = on_fake(async |client| {
        client
            .add_reaction(&write.target, &stand_in.emoji)
            .await
            .unwrap()
    });
    let stored_id = stored.id.clone();
    app.handle(Event::Reacted {
        write,
        result: Ok(Some(stored)),
    })
    .unwrap();

    let ids: Vec<String> = reactions(&app, &comment)
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    assert!(ids.contains(&stored_id), "{ids:?}");
    assert_eq!(ids.len(), 3);
}

#[test]
fn an_addition_linear_refuses_goes() {
    let mut app = app();
    focus(&mut app, OPEN);
    let comment = ReactionTarget::Comment(OPEN.to_owned());
    let write = react(&mut app, 7);

    app.handle(Event::Reacted {
        write,
        result: Err(forbidden(&format!("react on {FIRST}"))),
    })
    .unwrap();

    assert_eq!(emoji(&app, &comment), ["👀", "👍"]);
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with(&format!("Could not add 🚀 on {FIRST}"))
    );
}

#[test]
fn the_viewers_reaction_on_a_comment_toggles_off() {
    let mut app = app();
    focus(&mut app, OPEN);

    let write = react(&mut app, 8);

    assert!(matches!(write.change, ReactionChange::Remove(_)));
    assert_eq!(
        emoji(&app, &ReactionTarget::Comment(OPEN.to_owned())),
        ["👍"]
    );
}

/// Loads DES-34's detail again, as Linear would answer before it has
/// taken the reactions in flight.
fn reload(app: &mut App<TestBackend>) {
    app.handle(Event::IssueDetailLoaded {
        issue_id: ISSUE.to_owned(),
        result: Ok(IssueDetail::demo(ISSUE)),
    })
    .unwrap();
}

#[test]
fn a_detail_loaded_while_an_addition_is_in_flight_keeps_it() {
    let mut app = app();
    focus(&mut app, OPEN);
    let comment = ReactionTarget::Comment(OPEN.to_owned());
    let write = react(&mut app, 7);

    reload(&mut app);
    assert_eq!(emoji(&app, &comment), ["👀", "👍", "🚀"]);

    let stored = on_fake(async |client| client.add_reaction(&write.target, "🚀").await.unwrap());
    app.handle(Event::Reacted {
        write,
        result: Ok(Some(stored)),
    })
    .unwrap();
    assert_eq!(
        emoji(&app, &comment),
        ["👀", "👍", "🚀"],
        "no stand-in left"
    );
}

#[test]
fn a_detail_loaded_while_a_removal_is_in_flight_keeps_it_gone() {
    let mut app = app();
    react(&mut app, 4);

    reload(&mut app);

    assert_eq!(emoji(&app, &issue()), ["👍", "👍"]);
}

#[test]
fn a_shortcode_from_linear_counts_as_the_pickers_emoji() {
    let mut app = app();
    let mut detail = IssueDetail::demo(ISSUE);
    let viewer = app.state().store.viewer().unwrap().id.clone();
    detail.reactions = vec![Reaction {
        id: "reaction-short".to_owned(),
        emoji: "+1".to_owned(),
        user_id: Some(viewer),
        created_at: jiff::Timestamp::UNIX_EPOCH,
    }];
    app.handle(Event::IssueDetailLoaded {
        issue_id: ISSUE.to_owned(),
        result: Ok(detail),
    })
    .unwrap();

    assert!(shows(&app, "👍 1"));
    let write = react(&mut app, 1);
    assert!(matches!(write.change, ReactionChange::Remove(r) if r.id == "reaction-short"));
}

#[test]
fn the_fake_client_keeps_reactions_for_the_session() {
    let detail = on_fake(async |client| {
        let added = client.add_reaction(&issue(), "🚀").await.unwrap();
        client.remove_reaction(OWN_PARTY).await.unwrap();
        client
            .add_reaction(&ReactionTarget::Comment(OPEN.to_owned()), "🎉")
            .await
            .unwrap();
        assert!(client.remove_reaction("reaction-gone").await.is_err());
        let detail = client.issue_detail(ISSUE, Urgency::Urgent).await.unwrap();
        assert!(detail.reactions.iter().any(|r| r.id == added.id));
        detail
    });

    let issue_emoji: Vec<&str> = detail.reactions.iter().map(|r| r.emoji.as_str()).collect();
    assert_eq!(issue_emoji, ["👍", "👍", "🚀"]);
    let comment = detail.comments.iter().find(|c| c.id == OPEN).unwrap();
    assert!(comment.reactions.iter().any(|r| r.emoji == "🎉"));
}
