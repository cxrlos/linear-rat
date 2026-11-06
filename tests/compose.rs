//! Writing comments and creating issues through the app driver: the hand-off
//! to `$EDITOR` as an effect, an empty buffer cancelling, the mutation each
//! sends, and what the screen shows once Linear answers.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{App, Effect, Event, NewIssue, State, Writing, is_local};
use linear_rat::linear::model::Priority;
use linear_rat::linear::{FakeClient, IssueCreate, IssueDetail, LinearClient, demo};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo.
const FIRST: &str = "DES-34";

fn app() -> App<TestBackend> {
    App::new(TestBackend::new(80, 24), State::demo()).unwrap()
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Types `keys`, one character each, and returns every effect asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key(KeyCode::Char(c))).unwrap())
        .collect()
}

/// The only effect, which must open the editor.
fn only_editor(effects: Vec<Effect>) -> (Writing, String) {
    match effects.as_slice() {
        [Effect::OpenEditor { writing, text }] => (writing.clone(), text.clone()),
        other => panic!("expected the editor to open, got {other:?}"),
    }
}

#[test]
fn c_hands_an_empty_buffer_for_a_comment_on_the_focused_issue_to_the_editor() {
    let mut app = app();

    let (writing, text) = only_editor(press(&mut app, "c"));

    let Writing::Comment { identifier, .. } = writing else {
        panic!("{writing:?}");
    };
    assert_eq!(identifier, FIRST);
    assert_eq!(text, "");
}

fn screen(app: &App<TestBackend>) -> Vec<String> {
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn shows(app: &App<TestBackend>, text: &str) -> bool {
    screen(app).iter().any(|line| line.contains(text))
}

/// Opens the focused issue's detail full width and feeds the fake client's
/// comments and history for it.
fn open_detail(app: &mut App<TestBackend>) {
    for effect in app.handle(key(KeyCode::Enter)).unwrap() {
        if let Effect::LoadIssueDetail { issue_id, .. } = effect {
            let result = Ok(IssueDetail::demo(&issue_id));
            app.handle(Event::IssueDetailLoaded { issue_id, result })
                .unwrap();
        }
    }
}

fn saved(writing: Writing, text: &str) -> Event {
    Event::EditorClosed {
        writing,
        result: Ok(text.to_owned()),
    }
}

/// Runs `request` on the fake client on a thread of its own, since the fake
/// remembers what it was sent per thread and each answer must start from the
/// demo workspace.
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
fn a_blank_buffer_cancels_the_comment_without_a_request() {
    let mut app = app();
    let (writing, _) = only_editor(press(&mut app, "c"));

    let effects = app.handle(saved(writing, "  \n\n")).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(
        &app,
        &format!("No comment added to {FIRST}: it was empty")
    ));
}

#[test]
fn a_saved_comment_is_sent_once_and_shows_in_the_detail_when_linear_stores_it() {
    let mut app = app();
    open_detail(&mut app);
    let (writing, _) = only_editor(press(&mut app, "c"));

    let effects = app.handle(saved(writing, "Rind looks *great*\n")).unwrap();

    let [Effect::CreateComment(comment)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(comment.identifier, FIRST);
    assert_eq!(comment.body, "Rind looks *great*");
    let posted = on_fake(async |client| {
        client
            .create_comment(&comment.issue_id, &comment.body)
            .await
            .unwrap()
    });
    app.handle(Event::CommentCreated {
        comment: comment.clone(),
        result: Ok(posted),
    })
    .unwrap();
    assert!(shows(&app, &format!("Commented on {FIRST}")));
    assert!(shows(&app, "Rind looks great"), "{:#?}", screen(&app));
}

#[test]
fn a_rejected_comment_says_why_and_the_next_editor_starts_from_it() {
    let mut app = app();
    let (writing, _) = only_editor(press(&mut app, "c"));
    let effects = app.handle(saved(writing, "Second batch?")).unwrap();
    let [Effect::CreateComment(comment)] = effects.as_slice() else {
        panic!("{effects:?}");
    };

    app.handle(Event::CommentCreated {
        comment: comment.clone(),
        result: Err("Linear could not be reached".to_owned()),
    })
    .unwrap();

    assert!(shows(&app, &format!("Could not comment on {FIRST}")));
    let (_, text) = only_editor(press(&mut app, "c"));
    assert_eq!(text, "Second batch?");
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

/// Carries out every load in `effects` with the fake client, and the loads
/// their answers ask for, until none is left. Returns the other effects.
fn settle(app: &mut App<TestBackend>, effects: Vec<Effect>) -> Vec<Effect> {
    let client = FakeClient;
    let mut queue = VecDeque::from(effects);
    let mut rest = Vec::new();
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::LoadWorkflowStates { team_id } => Event::WorkflowStatesLoaded {
                result: Ok(block_on(client.workflow_states(&team_id)).unwrap()),
                team_id,
            },
            Effect::LoadTeamMembers { team_id } => Event::TeamMembersLoaded {
                result: Ok(block_on(client.team_members(&team_id)).unwrap()),
                team_id,
            },
            Effect::LoadTeams => Event::TeamsLoaded(Ok(block_on(client.teams()).unwrap())),
            Effect::LoadTeamOptions(team_id) => {
                Event::TeamOptionsLoaded(Ok(block_on(client.team_options(&team_id)).unwrap()))
            }
            Effect::LoadTeamIssues { query, after } => Event::TeamIssuesLoaded {
                result: Ok(block_on(client.team_issues(&query, after.as_deref())).unwrap()),
                query,
                after,
            },
            Effect::CountTeamIssues(query) => Event::TeamIssuesCounted {
                result: Ok(block_on(client.count_team_issues(&query)).unwrap()),
                query,
            },
            Effect::LoadIssueDetail { .. } => continue,
            other => {
                rest.push(other);
                continue;
            }
        };
        queue.extend(app.handle(event).unwrap());
    }
    rest
}

/// Feeds `events` and settles the loads they ask for.
fn walk(app: &mut App<TestBackend>, events: &[Event]) -> Vec<Effect> {
    let mut rest = Vec::new();
    for event in events {
        let effects = app.handle(event.clone()).unwrap();
        rest.extend(settle(app, effects));
    }
    rest
}

fn chars(text: &str) -> Vec<Event> {
    text.chars().map(|c| key(KeyCode::Char(c))).collect()
}

/// The only effect, which must create an issue.
fn only_create(effects: Vec<Effect>) -> NewIssue {
    match effects.as_slice() {
        [Effect::CreateIssue(issue)] => issue.clone(),
        other => panic!("expected exactly one issueCreate, got {other:?}"),
    }
}

#[test]
fn n_opens_the_form_on_the_team_of_the_focused_issue() {
    let mut app = app();

    let effects = press(&mut app, "n");

    assert!(effects.is_empty(), "{effects:?}");
    assert_snapshot!(app.backend());
}

#[test]
fn the_form_asks_for_each_field_and_hands_the_issue_to_the_editor() {
    let mut app = app();
    let mut events = chars("n");
    events.extend(chars("ops"));
    events.push(key(KeyCode::Enter));
    // States by type: Triage, Backlog, Todo, In Progress, ...
    events.extend(chars("4"));
    events.extend(chars("omar"));
    events.push(key(KeyCode::Enter));
    // The priority starts on "No priority", the last; one up is Low.
    events.push(key(KeyCode::Up));
    events.push(key(KeyCode::Enter));
    events.extend(chars("Restock the brie cave"));

    let effects = walk(&mut app, &events);

    assert!(effects.is_empty(), "{effects:?}");
    assert_snapshot!(app.backend());
    let (writing, text) = only_editor(app.handle(key(KeyCode::Enter)).unwrap());
    assert_eq!(text, "");
    let Writing::Description(draft) = writing.clone() else {
        panic!("{writing:?}");
    };
    assert_eq!(draft.team_id, "team-ops");
    assert_eq!(draft.state.name, "In Progress");
    assert_eq!(draft.assignee_id.as_deref(), Some("member-omar"));
    assert_eq!(draft.priority, Priority::Low);
    assert_eq!(draft.title, "Restock the brie cave");

    let issue = only_create(app.handle(saved(writing, "Shelf 3 is *empty*.\n")).unwrap());

    assert_eq!(
        issue.input,
        IssueCreate {
            team_id: "team-ops".to_owned(),
            state_id: "team-ops-state-in-progress".to_owned(),
            assignee_id: Some("member-omar".to_owned()),
            priority: Priority::Low,
            title: "Restock the brie cave".to_owned(),
            description: "Shelf 3 is *empty*.".to_owned(),
        }
    );
}

/// Runs the form with every default: the team in view, Todo, the viewer
/// and no priority.
fn create_with_defaults(app: &mut App<TestBackend>, title: &str) -> NewIssue {
    let mut events = chars("n");
    events.extend([
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
    ]);
    events.extend(chars(title));
    let effects = walk(app, &events);
    assert!(effects.is_empty(), "{effects:?}");
    let (writing, _) = only_editor(app.handle(key(KeyCode::Enter)).unwrap());
    only_create(app.handle(saved(writing, "Why and how.")).unwrap())
}

#[test]
fn a_created_issue_shows_in_my_issues_at_once_and_linears_copy_replaces_it() {
    let mut app = app();

    let issue = create_with_defaults(&mut app, "Label the new wheels");

    assert_eq!(issue.input.team_id, "team-des");
    assert_eq!(issue.input.state_id, "team-des-state-todo");
    assert_eq!(issue.input.assignee_id.as_deref(), Some("member-mira"));
    assert_eq!(issue.input.priority, Priority::None);
    let row = screen(&app)
        .into_iter()
        .find(|line| line.contains("Label the new wheels"))
        .unwrap();
    assert!(row.contains("DES-new"), "{row}");

    let created = on_fake(async |client| client.create_issue(&issue.input).await.unwrap());
    let identifier = created.identifier.clone();
    app.handle(Event::IssueCreated {
        issue,
        result: Ok(Box::new(created)),
    })
    .unwrap();

    let rows: Vec<String> = screen(&app)
        .into_iter()
        .filter(|line| line.contains("Label the new wheels"))
        .collect();
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert!(rows[0].contains(&format!("{identifier} ")), "{rows:#?}");
    assert!(shows(&app, &format!("Created {identifier}")));
}

#[test]
fn an_issue_linear_rejects_leaves_the_list_and_keeps_its_description() {
    let mut app = app();
    let issue = create_with_defaults(&mut app, "Label the new wheels");

    app.handle(Event::IssueCreated {
        issue,
        result: Err("Linear returned errors: title too long".to_owned()),
    })
    .unwrap();

    assert!(!shows(&app, "DES-new"));
    assert!(shows(&app, "Could not create Label the new wheels"));
    let mut events = chars("n");
    events.extend([
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
    ]);
    events.extend(chars("Again"));
    walk(&mut app, &events);
    let (_, text) = only_editor(app.handle(key(KeyCode::Enter)).unwrap());
    assert_eq!(text, "Why and how.");
}

#[test]
fn a_created_issue_tops_team_issues_when_it_is_of_the_team_shown() {
    let mut app = app();
    walk(&mut app, &chars("2"));

    let issue = create_with_defaults(&mut app, "Calibrate the cave sensors");

    assert_eq!(issue.input.team_id, "team-eng");
    let focused = app.state().focused_issue().unwrap();
    assert_eq!(focused.identifier, "ENG-new");
    let created = on_fake(async |client| client.create_issue(&issue.input).await.unwrap());
    let identifier = created.identifier.clone();
    app.handle(Event::IssueCreated {
        issue,
        result: Ok(Box::new(created)),
    })
    .unwrap();
    assert_eq!(app.state().focused_issue().unwrap().identifier, identifier);
}

#[test]
fn an_empty_title_cancels_the_form() {
    let mut app = app();
    let mut events = chars("n");
    events.extend([
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
    ]);
    events.extend(chars("  "));

    walk(&mut app, &events);
    let effects = app.handle(key(KeyCode::Enter)).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(&app, "No issue created: the title was empty"));
}

#[test]
fn an_empty_description_cancels_the_issue_without_a_request() {
    let mut app = app();
    let mut events = chars("n");
    events.extend([
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
        key(KeyCode::Enter),
    ]);
    events.extend(chars("Taste test"));
    walk(&mut app, &events);
    let (writing, _) = only_editor(app.handle(key(KeyCode::Enter)).unwrap());

    let effects = app.handle(saved(writing, "\n \n")).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(shows(&app, "No issue created: the description was empty"));
    assert!(!shows(&app, "Taste test"));
}

#[test]
fn escape_leaves_the_form_without_a_request() {
    let mut app = app();
    let mut events = chars("n");
    events.push(key(KeyCode::Enter));

    walk(&mut app, &events);
    let effects = app.handle(key(KeyCode::Esc)).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(!shows(&app, "New DES issue"));
}

#[test]
fn the_reconciliation_never_asks_linear_about_a_stand_in() {
    let mut app = app();
    create_with_defaults(&mut app, "Label the new wheels");

    app.handle(Event::Tick(demo::NOW)).unwrap();
    let effects = app
        .handle(Event::Polled {
            changes: Vec::new(),
            failure: None,
        })
        .unwrap();

    let [Effect::Reconcile(checked)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert!(!checked.is_empty());
    assert!(!checked.iter().any(|id| is_local(id)), "{checked:?}");
}

#[test]
fn a_stand_in_opens_no_links_and_asks_linear_nothing() {
    let mut app = app();
    create_with_defaults(&mut app, "Label the new wheels");
    while !is_local(&app.state().focused_issue().unwrap().id) {
        app.handle(key(KeyCode::Char('k'))).unwrap();
    }

    for set in ["o", "O"] {
        let effects = press(&mut app, set);

        assert!(effects.is_empty(), "{set}: {effects:?}");
        assert!(app.state().modals.top().is_none(), "{set}");
        assert!(shows(&app, "not created yet"), "{set}");
    }
}

#[test]
fn a_stand_in_left_in_the_snapshot_does_not_come_back() {
    let mut app = app();
    create_with_defaults(&mut app, "Label the new wheels");
    let saved = app.state().store.clone();

    let state = State::loading(None).with_snapshot(saved, None);

    assert!(!state.store.issues().any(|issue| is_local(&issue.id)));
    assert!(state.store.issues().next().is_some());
}

#[test]
fn the_demo_keeps_what_was_written_for_the_rest_of_the_session() {
    let issue_id = app().state().focused_issue().unwrap().id.clone();

    let (detail, created, existing) = on_fake(async |client| {
        client.create_comment(&issue_id, "Noted.").await.unwrap();
        let detail = client
            .issue_detail(&issue_id, linear_rat::linear::Urgency::Urgent)
            .await
            .unwrap();
        let mut input = IssueCreate {
            team_id: "team-des".to_owned(),
            state_id: "team-des-state-todo".to_owned(),
            assignee_id: None,
            priority: Priority::None,
            title: "Two new wheels".to_owned(),
            description: "Gouda.".to_owned(),
        };
        let first = client.create_issue(&input).await.unwrap();
        input.title = "Third wheel".to_owned();
        let second = client.create_issue(&input).await.unwrap();
        let existing = client
            .existing_issues(&[first.id.clone(), second.id.clone()])
            .await
            .unwrap();
        (detail, [first, second], existing)
    });

    assert_eq!(detail.comments.last().unwrap().body, "Noted.");
    assert_ne!(created[0].identifier, created[1].identifier);
    assert!(created[0].identifier.starts_with("DES-"));
    assert_eq!(existing.len(), 2);
}
