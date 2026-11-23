//! An issue's parent, sub-issues and relations through the app driver: the
//! detail's Relations section, adding and removing relations, setting and
//! clearing the parent through the issue picker, the rollback when Linear
//! rejects a write, the blocked glyph on rows and the Blocked filter of Team
//! Issues.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    App, Edit, Effect, Event, Failure, Modal, RelationWrite, State, Write, WriteClass,
};
use linear_rat::linear::model::{Issue, IssueRelation, RelationKind, RelationType};
use linear_rat::linear::{
    ErrorKind, FakeClient, IssueDetail, LinearClient, Patch, RelationCreate, SearchQuery,
    SearchResults, TeamFilter, TeamQuery,
};
use ratatui::backend::TestBackend;

const DRY_RUN: &str = "issue-eng-51";

fn sized(columns: u16, rows: u16) -> App<TestBackend> {
    App::new(TestBackend::new(columns, rows), State::demo()).unwrap()
}

fn key_event(c: char) -> Event {
    let code = match c {
        '\n' => KeyCode::Enter,
        '\x1b' => KeyCode::Esc,
        '\t' => KeyCode::Tab,
        c => KeyCode::Char(c),
    };
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Presses each key of `keys` (`\n` is Enter, `\x1b` Esc, `\t` Tab) and returns every
/// effect they asked for.
fn press(app: &mut App<TestBackend>, keys: &str) -> Vec<Effect> {
    keys.chars()
        .flat_map(|c| app.handle(key_event(c)).unwrap())
        .collect()
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

/// The list row of the issue with `identifier`, if it is on screen.
fn row(app: &App<TestBackend>, identifier: &str) -> Option<String> {
    screen(app)
        .into_iter()
        .find(|line| line.contains(&format!("{identifier} ")))
}

/// Moves the My Issues focus down to the issue with `identifier`.
fn focus(app: &mut App<TestBackend>, identifier: &str) {
    for _ in 0..40 {
        if app.state().focused_issue().unwrap().identifier == identifier {
            return;
        }
        press(app, "j");
    }
    panic!("{identifier} is not in My Issues");
}

/// Feeds the demo detail of `issue_id` as the fake client gives it.
fn detail_loaded(app: &mut App<TestBackend>, issue_id: &str) {
    let result = Ok(IssueDetail::demo(issue_id));
    app.handle(Event::IssueDetailLoaded {
        issue_id: issue_id.to_owned(),
        result,
    })
    .unwrap();
}

/// The app focused on `identifier` with its detail loaded.
fn focused_with_detail(identifier: &str) -> App<TestBackend> {
    let mut app = sized(80, 24);
    focus(&mut app, identifier);
    let id = app.state().focused_issue().unwrap().id.clone();
    detail_loaded(&mut app, &id);
    app
}

/// How each relation of `issue_id` reads from it, with the other issue's id.
fn relations(app: &App<TestBackend>, issue_id: &str) -> Vec<(RelationKind, String)> {
    app.state()
        .store
        .relations(issue_id)
        .into_iter()
        .map(|(kind, _, other)| (kind, other.to_owned()))
        .collect()
}

fn creates(effects: &[Effect]) -> Vec<RelationWrite> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CreateRelation(write) => Some(write.clone()),
            _ => None,
        })
        .collect()
}

fn deletes(effects: &[Effect]) -> Vec<RelationWrite> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DeleteRelation(write) => Some(write.clone()),
            _ => None,
        })
        .collect()
}

/// The only `issueUpdate` among `effects`.
fn only_write(effects: Vec<Effect>) -> Write {
    let writes: Vec<&Write> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::UpdateIssue(write) => Some(write),
            _ => None,
        })
        .collect();
    match writes.as_slice() {
        [write] => (*write).clone(),
        _ => panic!("expected exactly one write, got {effects:?}"),
    }
}

/// Runs `future` on a thread of its own, because the fake client remembers
/// writes per thread and each answer here must start from the demo.
fn on_own_thread<T: Send>(future: impl Future<Output = T> + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(future)
            })
            .join()
            .unwrap()
    })
}

const REJECTION: &str = "Linear returned errors: Entity not found (INVALID_INPUT)";

#[test]
fn the_detail_lists_the_parent_sub_issues_and_each_relation() {
    let mut app = App::new(TestBackend::new(80, 40), State::demo()).unwrap();
    focus(&mut app, "ENG-51");
    press(&mut app, "\n");
    detail_loaded(&mut app, DRY_RUN);

    assert!(shows(&app, "Sub-issues    3/4 done"), "{:#?}", screen(&app));
    assert_snapshot!(app.backend());
}

#[test]
fn the_detail_lists_duplicates_both_ways() {
    let mut app = App::new(TestBackend::new(80, 42), State::demo()).unwrap();
    focus(&mut app, "ENG-51");
    press(&mut app, "\n");
    let mut detail = IssueDetail::demo(DRY_RUN);
    detail.relations.extend([
        IssueRelation {
            id: "relation-dup-of".to_owned(),
            kind: RelationType::Duplicate,
            issue_id: DRY_RUN.to_owned(),
            related_issue_id: "issue-eng-8".to_owned(),
        },
        IssueRelation {
            id: "relation-dup-by".to_owned(),
            kind: RelationType::Duplicate,
            issue_id: "issue-eng-27".to_owned(),
            related_issue_id: DRY_RUN.to_owned(),
        },
    ]);
    app.handle(Event::IssueDetailLoaded {
        issue_id: DRY_RUN.to_owned(),
        result: Ok(detail),
    })
    .unwrap();

    let lines = screen(&app);
    let line = |label: &str| lines.iter().find(|line| line.contains(label)).cloned();
    assert!(
        line("Duplicate of").is_some_and(|line| line.contains("ENG-8 Export")),
        "{lines:#?}"
    );
    assert!(
        line("Duplicated by").is_some_and(|line| line.contains("ENG-27 Deduplicate")),
        "{lines:#?}"
    );
    assert_snapshot!(app.backend());
}

#[test]
fn a_sub_issue_shows_its_parent() {
    let mut app = App::new(TestBackend::new(80, 40), State::demo()).unwrap();
    focus(&mut app, "ENG-60");
    press(&mut app, "\n");
    detail_loaded(&mut app, "issue-eng-60");

    let lines = screen(&app);
    let parent = lines.iter().find(|line| line.contains("Parent")).unwrap();
    assert!(parent.contains("ENG-51 Add a dry-run mode"), "{parent}");
}

#[test]
fn an_issue_with_no_family_or_relations_leaves_the_section_out() {
    let mut app = App::new(TestBackend::new(80, 40), State::demo()).unwrap();
    press(&mut app, "\n");
    let id = app.state().focused_issue().unwrap().id.clone();
    detail_loaded(&mut app, &id);

    assert!(!shows(&app, "Relations"), "{:#?}", screen(&app));
}

#[test]
fn adding_a_relation_shows_at_once_and_takes_linears_id() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");

    press(&mut app, "R");
    assert!(shows(&app, "Add a relation"), "{:#?}", screen(&app));
    let effects = press(&mut app, "\nENG-60\n");

    let writes = creates(&effects);
    let [write] = writes.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(write.relation.kind, RelationType::Blocks);
    assert_eq!(write.relation.issue_id, DRY_RUN);
    assert_eq!(write.relation.related_issue_id, "issue-eng-60");
    assert_eq!(
        relations(&app, DRY_RUN),
        [(RelationKind::Blocks, "issue-eng-60".to_owned())]
    );

    let stored = on_own_thread(FakeClient.create_relation(&RelationCreate {
        issue_id: write.relation.issue_id.clone(),
        related_issue_id: write.relation.related_issue_id.clone(),
        kind: write.relation.kind,
    }))
    .unwrap();
    app.handle(Event::RelationCreated {
        write: write.clone(),
        result: Ok(stored.clone()),
    })
    .unwrap();

    let ids: Vec<String> = app
        .state()
        .store
        .relations(DRY_RUN)
        .into_iter()
        .map(|(_, relation, _)| relation.id.clone())
        .collect();
    assert_eq!(ids, [stored.id]);
}

#[test]
fn blocked_by_goes_to_linear_from_the_blocking_issue() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");

    // Blocked by is the second kind.
    let effects = press(&mut app, "R2ENG-60\n");

    let writes = creates(&effects);
    let [write] = writes.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(write.relation.kind, RelationType::Blocks);
    assert_eq!(write.relation.issue_id, "issue-eng-60");
    assert_eq!(write.relation.related_issue_id, DRY_RUN);
    assert_eq!(
        relations(&app, DRY_RUN),
        [(RelationKind::BlockedBy, "issue-eng-60".to_owned())]
    );
}

#[test]
fn several_issues_marked_each_get_a_relation() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");

    press(&mut app, "R3");
    let mut effects = Vec::new();
    for key in ["ENG-60", "ENG-61"] {
        press(&mut app, key);
        app.handle(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
            .unwrap();
        for _ in 0..key.len() {
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE,
            )))
            .unwrap();
        }
    }
    effects.extend(press(&mut app, "\n"));

    let others: Vec<String> = creates(&effects)
        .into_iter()
        .map(|write| write.relation.related_issue_id)
        .collect();
    assert_eq!(others, ["issue-eng-60", "issue-eng-61"]);
}

#[test]
fn a_relation_linear_rejects_goes_away_and_says_why() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");
    let effects = press(&mut app, "R\nENG-60\n");
    let write = creates(&effects).remove(0);

    app.handle(Event::RelationCreated {
        write,
        result: Err(Failure::new(ErrorKind::Other, "test", REJECTION)),
    })
    .unwrap();

    assert!(relations(&app, DRY_RUN).is_empty());
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with("Could not add the relation to ENG-51"),
        "{:?}",
        app.state().notice
    );
    assert!(row(&app, "ENG-51").unwrap().contains('✘'));
}

#[test]
fn the_picker_for_a_new_relation_leaves_out_the_issue_and_those_already_related_so() {
    let mut app = focused_with_detail("ENG-51");

    press(&mut app, "R1");

    let Some(Modal::IssuePicker(picker)) = app.state().modals.top() else {
        panic!("{:?}", app.state().modals.top());
    };
    assert!(picker.scope().excluded.contains(&DRY_RUN.to_owned()));
    assert!(picker.scope().excluded.contains(&"issue-eng-27".to_owned()));
    assert!(!picker.scope().excluded.contains(&"issue-eng-6".to_owned()));
}

/// The relation ids the remove-relation picker on top offers, in order.
fn removable(app: &App<TestBackend>) -> Vec<String> {
    match app.state().modals.top() {
        Some(Modal::Choice(choice)) => choice.list.values().flatten().cloned().collect(),
        other => panic!("no remove-relation picker: {other:?}"),
    }
}

/// Relates ENG-51 to ENG-27 a second way, and has Linear confirm it as
/// `relation-9`.
fn second_relation_to_eng_27(app: &mut App<TestBackend>) {
    let effects = press(app, "R3ENG-27\n");
    let [write] = creates(&effects).try_into().unwrap();
    let stored = IssueRelation {
        id: "relation-9".to_owned(),
        ..write.relation.clone()
    };
    app.handle(Event::RelationCreated {
        write,
        result: Ok(stored),
    })
    .unwrap();
}

#[test]
fn removing_lists_each_relation_and_deletes_the_chosen_one() {
    let mut app = focused_with_detail("ENG-51");

    press(&mut app, "Dr");
    assert!(shows(&app, "Remove relations"), "{:#?}", screen(&app));
    assert!(shows(&app, "blocks ENG-27"), "{:#?}", screen(&app));
    assert!(shows(&app, "blocked by ENG-6"), "{:#?}", screen(&app));
    assert!(shows(&app, "related to ENG-8"), "{:#?}", screen(&app));
    assert_eq!(removable(&app), ["relation-2", "relation-1", "relation-3"]);
    let effects = press(&mut app, "ENG-27\n");

    let writes = deletes(&effects);
    let [write] = writes.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(write.relation.id, "relation-2");
    assert!(
        !relations(&app, DRY_RUN)
            .iter()
            .any(|(_, other)| other == "issue-eng-27")
    );

    app.handle(Event::RelationDeleted {
        write: write.clone(),
        result: Ok(()),
    })
    .unwrap();
    assert_eq!(relations(&app, DRY_RUN).len(), 2);
}

#[test]
fn of_two_relations_with_one_issue_removing_one_leaves_the_other() {
    let mut app = focused_with_detail("ENG-51");
    second_relation_to_eng_27(&mut app);

    let effects = press(&mut app, "Drrelated ENG-27\n");

    let ids: Vec<String> = deletes(&effects)
        .into_iter()
        .map(|write| write.relation.id)
        .collect();
    assert_eq!(ids, ["relation-9"]);
    assert_eq!(
        relations(&app, DRY_RUN)
            .into_iter()
            .filter(|(_, other)| other == "issue-eng-27")
            .collect::<Vec<_>>(),
        [(RelationKind::Blocks, "issue-eng-27".to_owned())]
    );
}

#[test]
fn tab_marks_several_relations_and_enter_removes_only_those() {
    let mut app = focused_with_detail("ENG-51");

    let effects = press(&mut app, "Dr\t3\n");

    let ids: Vec<String> = deletes(&effects)
        .into_iter()
        .map(|write| write.relation.id)
        .collect();
    assert_eq!(ids, ["relation-2", "relation-3"]);
    assert_eq!(
        relations(&app, DRY_RUN),
        [(RelationKind::BlockedBy, "issue-eng-6".to_owned())]
    );
}

#[test]
fn an_issue_without_relations_says_so_and_opens_nothing() {
    let mut app = App::new(TestBackend::new(80, 40), State::demo()).unwrap();
    let id = app.state().focused_issue().unwrap().id.clone();
    let identifier = app.state().focused_issue().unwrap().identifier.clone();
    detail_loaded(&mut app, &id);
    assert!(relations(&app, &id).is_empty());

    press(&mut app, "Dr");

    assert!(app.state().modals.top().is_none());
    assert_eq!(
        app.state().notice.as_deref(),
        Some(format!("{identifier} has no relations").as_str())
    );
}

#[test]
fn the_remove_relation_picker() {
    let mut app = focused_with_detail("ENG-51");

    press(&mut app, "Dr\t");

    assert_snapshot!(app.backend());
}

#[test]
fn a_removal_linear_rejects_puts_the_relation_back() {
    let mut app = focused_with_detail("ENG-51");
    let effects = press(&mut app, "DrENG-27\n");
    let write = deletes(&effects).remove(0);

    app.handle(Event::RelationDeleted {
        write,
        result: Err(Failure::new(ErrorKind::Other, "test", REJECTION)),
    })
    .unwrap();

    assert!(relations(&app, DRY_RUN).contains(&(RelationKind::Blocks, "issue-eng-27".to_owned())));
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with("Could not remove the relation from ENG-51")
    );
}

#[test]
fn a_removal_linear_forbids_denies_relations_in_the_team() {
    let mut app = focused_with_detail("ENG-51");
    let effects = press(&mut app, "DrENG-27\n");
    let write = deletes(&effects).remove(0);
    assert!(app.state().write_denied(WriteClass::Relations).is_none());

    app.handle(Event::RelationDeleted {
        write,
        result: Err(Failure::new(
            ErrorKind::Forbidden,
            "remove the relation from ENG-51",
            "this API key may not do that",
        )),
    })
    .unwrap();

    assert!(app.state().write_denied(WriteClass::Relations).is_some());
}

#[test]
fn a_detail_reload_keeps_relation_writes_still_in_flight() {
    let mut app = focused_with_detail("ENG-51");
    let removal = press(&mut app, "DrENG-6\n");
    let [removal] = deletes(&removal).try_into().unwrap();
    let addition = press(&mut app, "R1ENG-60\n");
    let [addition] = creates(&addition).try_into().unwrap();

    detail_loaded(&mut app, DRY_RUN);

    let after_reload = relations(&app, DRY_RUN);
    assert!(
        !after_reload.contains(&(RelationKind::BlockedBy, "issue-eng-6".to_owned())),
        "{after_reload:?}"
    );
    assert!(
        after_reload.contains(&(RelationKind::Blocks, "issue-eng-60".to_owned())),
        "{after_reload:?}"
    );

    app.handle(Event::RelationDeleted {
        write: removal,
        result: Ok(()),
    })
    .unwrap();
    app.handle(Event::RelationCreated {
        write: addition,
        result: Err(Failure::new(ErrorKind::Other, "test", REJECTION)),
    })
    .unwrap();
    detail_loaded(&mut app, DRY_RUN);

    // Nothing is in flight, so the detail is taken as Linear gives it.
    let settled = relations(&app, DRY_RUN);
    assert!(
        settled.contains(&(RelationKind::BlockedBy, "issue-eng-6".to_owned())),
        "{settled:?}"
    );
    assert!(
        !settled.contains(&(RelationKind::Blocks, "issue-eng-60".to_owned())),
        "{settled:?}"
    );
}

#[test]
fn removing_before_the_relations_loaded_asks_for_them_first() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");

    let effects = press(&mut app, "Dr");

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadIssueDetail { issue_id, .. } if issue_id == DRY_RUN
        )),
        "{effects:?}"
    );
    assert!(app.state().modals.top().is_none());
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with("Loading the relations of ENG-51")
    );
}

#[test]
fn setting_a_parent_shows_at_once_and_rolls_back_when_linear_rejects_it() {
    let mut app = focused_with_detail("ENG-6");

    let write = only_write(press(&mut app, "HENG-60\n"));

    assert_eq!(write.edit, Edit::Parent(Some("issue-eng-60".to_owned())));
    assert_eq!(write.previous, Edit::Parent(None));
    assert_eq!(
        write.update().parent_id,
        Patch::Set("issue-eng-60".to_owned())
    );
    let parent = |app: &App<TestBackend>| {
        app.state()
            .store
            .issue("issue-eng-6")
            .unwrap()
            .parent_id
            .clone()
    };
    assert_eq!(parent(&app).as_deref(), Some("issue-eng-60"));

    app.handle(Event::IssueUpdated {
        write,
        result: Err(Failure::new(ErrorKind::Other, "test", REJECTION)),
    })
    .unwrap();

    assert_eq!(parent(&app), None);
    assert!(row(&app, "ENG-6").unwrap().contains('✘'));
}

#[test]
fn a_parent_linear_accepts_stays() {
    let mut app = focused_with_detail("ENG-6");
    let write = only_write(press(&mut app, "HENG-60\n"));

    let issue = on_own_thread(FakeClient.update_issue(&write.issue_id, &write.update())).unwrap();
    app.handle(Event::IssueUpdated {
        write,
        result: Ok(Box::new(issue)),
    })
    .unwrap();

    let stored = app.state().store.issue("issue-eng-6").unwrap();
    assert_eq!(stored.parent_id.as_deref(), Some("issue-eng-60"));
    assert!(app.state().notice.is_none(), "{:?}", app.state().notice);
}

/// The ids the picker on top offers.
fn offered(app: &App<TestBackend>) -> Vec<String> {
    let Some(Modal::IssuePicker(picker)) = app.state().modals.top() else {
        panic!("{:?}", app.state().modals.top());
    };
    picker.list.values().cloned().collect()
}

/// A copy of the stored issue `like`, as `identifier`, with `parent_id` as
/// its parent: an issue only Linear's search knows about.
fn found_issue(app: &App<TestBackend>, like: &str, identifier: &str, parent_id: &str) -> Issue {
    let mut issue = app.state().store.issue(like).unwrap().clone();
    issue.id = format!("issue-{}", identifier.to_lowercase());
    issue.identifier = identifier.to_owned();
    issue.parent_id = Some(parent_id.to_owned());
    issue
}

/// Types `query` into the picker on top, lets typing pause and answers its
/// search with `issues`.
fn search_answers(app: &mut App<TestBackend>, query: &str, issues: Vec<Issue>) {
    press(app, query);
    let effects = app.handle(Event::TypingPaused(query.to_owned())).unwrap();
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::Search(sent) if *sent == SearchQuery::new(query))
        ),
        "{effects:?}"
    );
    let result = Ok(SearchResults {
        issues,
        cycles: Vec::new(),
        labels: Vec::new(),
        members: Vec::new(),
        teams: Vec::new(),
        problems: Vec::new(),
    });
    app.handle(Event::SearchDone {
        query: SearchQuery::new(query),
        result,
    })
    .unwrap();
}

#[test]
fn the_parent_picker_leaves_out_the_issue_and_its_sub_issues() {
    let mut app = focused_with_detail("ENG-51");

    press(&mut app, "H");

    let offered = offered(&app);
    for id in [
        "issue-eng-17",
        "issue-eng-29",
        "issue-eng-51",
        "issue-eng-60",
        "issue-eng-69",
    ] {
        assert!(!offered.contains(&id.to_owned()), "{id} in {offered:?}");
    }
    assert!(offered.contains(&"issue-eng-6".to_owned()), "{offered:?}");
}

#[test]
fn sub_issues_at_any_depth_found_by_search_are_never_offered_as_parent() {
    let mut app = focused_with_detail("ENG-51");
    let child = found_issue(&app, "issue-eng-6", "ENG-91", DRY_RUN);
    let grandchild = found_issue(&app, "issue-eng-6", "ENG-92", "issue-eng-60");
    let great_grandchild = found_issue(&app, "issue-eng-6", "ENG-93", "issue-eng-92");
    let elsewhere = found_issue(&app, "issue-eng-6", "ENG-94", "issue-eng-6");

    press(&mut app, "H");
    search_answers(
        &mut app,
        "ENG-9",
        vec![child, grandchild, great_grandchild, elsewhere],
    );

    let offered = offered(&app);
    assert!(offered.contains(&"issue-eng-94".to_owned()), "{offered:?}");
    for id in ["issue-eng-91", "issue-eng-92", "issue-eng-93"] {
        assert!(!offered.contains(&id.to_owned()), "{id} in {offered:?}");
    }
}

#[test]
fn parents_that_loop_do_not_hang_the_parent_picker() {
    let mut app = focused_with_detail("ENG-51");
    let one = found_issue(&app, "issue-eng-6", "ENG-95", "issue-eng-96");
    let other = found_issue(&app, "issue-eng-6", "ENG-96", "issue-eng-95");

    press(&mut app, "H");
    search_answers(&mut app, "ENG-9", vec![one, other]);

    let offered = offered(&app);
    for id in ["issue-eng-95", "issue-eng-96"] {
        assert!(offered.contains(&id.to_owned()), "{id} not in {offered:?}");
    }
}

#[test]
fn setting_a_parent_before_the_sub_issues_loaded_asks_for_them_first() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");

    let effects = press(&mut app, "H");

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadIssueDetail { issue_id, .. } if issue_id == DRY_RUN
        )),
        "{effects:?}"
    );
    assert!(app.state().modals.top().is_none());
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap()
            .starts_with("Loading the relations of ENG-51")
    );
}

#[test]
fn clearing_the_parent_shows_at_once_and_rolls_back_when_linear_rejects_it() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-60");

    let write = only_write(press(&mut app, "Dp"));

    assert_eq!(write.edit, Edit::Parent(None));
    assert_eq!(write.update().parent_id, Patch::Clear);
    let parent = |app: &App<TestBackend>| {
        app.state()
            .store
            .issue("issue-eng-60")
            .unwrap()
            .parent_id
            .clone()
    };
    assert_eq!(parent(&app), None);

    app.handle(Event::IssueUpdated {
        write,
        result: Err(Failure::new(ErrorKind::Other, "test", REJECTION)),
    })
    .unwrap();

    assert_eq!(parent(&app).as_deref(), Some(DRY_RUN));
}

#[test]
fn a_parent_cleared_and_accepted_stays_cleared() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-60");
    let write = only_write(press(&mut app, "Dp"));

    let issue = on_own_thread(FakeClient.update_issue(&write.issue_id, &write.update())).unwrap();
    app.handle(Event::IssueUpdated {
        write,
        result: Ok(Box::new(issue)),
    })
    .unwrap();

    assert_eq!(
        app.state().store.issue("issue-eng-60").unwrap().parent_id,
        None
    );
}

#[test]
fn going_to_a_related_issue_opens_it_like_a_palette_jump() {
    let mut app = focused_with_detail("ENG-51");

    press(&mut app, "gr");
    assert!(shows(&app, "Go from ENG-51"), "{:#?}", screen(&app));
    assert!(shows(&app, "Sub-issue · ENG-60"), "{:#?}", screen(&app));
    assert!(shows(&app, "Blocked by · ENG-6 "), "{:#?}", screen(&app));
    press(&mut app, "ENG-6\n");

    assert!(app.state().modals.top().is_none());
    assert_eq!(app.state().focused_issue().unwrap().identifier, "ENG-6");
}

#[test]
fn a_row_blocked_by_an_open_issue_shows_the_blocked_glyph() {
    let mut app = sized(80, 24);
    assert!(!row(&app, "ENG-51").unwrap().contains('▲'));

    focus(&mut app, "ENG-51");
    detail_loaded(&mut app, DRY_RUN);
    press(&mut app, "k");

    assert!(row(&app, "ENG-51").unwrap().contains("▲ Add a dry-run"));
    assert!(!row(&app, "ENG-60").unwrap().contains('▲'));
}

#[test]
fn only_an_open_blocker_blocks() {
    // ENG-51's detail brings the completed ENG-27 and the open ENG-6.
    let mut app = focused_with_detail("ENG-51");
    focus(&mut app, "ENG-60");

    press(&mut app, "R2ENG-27\n");
    assert!(!app.state().store.is_blocked("issue-eng-60"));

    let effects = press(&mut app, "R2ENG-6\n");
    assert!(
        app.state().store.is_blocked("issue-eng-60"),
        "{effects:?} {:?} {:?}",
        relations(&app, "issue-eng-60"),
        app.state().notice
    );
}

/// Carries out every Team Issues load in `effects` with the fake client.
async fn settle(app: &mut App<TestBackend>, effects: Vec<Effect>) {
    let client = FakeClient;
    let mut queue = std::collections::VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        let event = match effect {
            Effect::LoadTeams => Event::TeamsLoaded(Ok(client.teams().await.unwrap())),
            Effect::LoadTeamOptions(team_id) => {
                Event::TeamOptionsLoaded(Ok(client.team_options(&team_id).await.unwrap()))
            }
            Effect::LoadTeamIssues { query, after } => {
                let result = Ok(client.team_issues(&query, after.as_deref()).await.unwrap());
                Event::TeamIssuesLoaded {
                    query,
                    after,
                    result,
                }
            }
            Effect::CountTeamIssues(query) => {
                let result = Ok(client.count_team_issues(&query).await.unwrap());
                Event::TeamIssuesCounted { query, result }
            }
            _ => continue,
        };
        queue.extend(app.handle(event).unwrap());
    }
}

#[tokio::test]
async fn the_blocked_filter_narrows_team_issues_and_toggles_off() {
    let mut app = sized(80, 24);
    let effects = press(&mut app, "2");
    settle(&mut app, effects).await;
    let all = app.state().team_issues.ids().len();

    let effects = press(&mut app, "fb");

    let Some(Effect::LoadTeamIssues { query, after: None }) = effects.first() else {
        panic!("{effects:?}");
    };
    assert_eq!(
        query.filter,
        TeamFilter {
            blocked: true,
            ..TeamFilter::default()
        }
    );
    settle(&mut app, effects).await;
    let keys: Vec<String> = app
        .state()
        .team_issues
        .ids()
        .iter()
        .map(|id| app.state().store.issue(id).unwrap().identifier.clone())
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys.contains(&"ENG-51".to_owned()), "{keys:?}");
    assert!(keys.contains(&"ENG-27".to_owned()), "{keys:?}");
    assert!(screen(&app)[1].trim_end().ends_with("Filters Blocked"));
    assert_snapshot!(app.backend());

    let effects = press(&mut app, "fb");
    settle(&mut app, effects).await;
    assert_eq!(app.state().team_issues.ids().len(), all);
    assert!(screen(&app)[1].trim_end().ends_with("Filters none"));
}

#[test]
fn the_demo_blocked_filter_follows_relations_made_and_removed_in_the_demo() {
    let blocked = TeamQuery {
        team_id: "team-eng".to_owned(),
        filter: TeamFilter {
            blocked: true,
            ..TeamFilter::default()
        },
        ..TeamQuery::default()
    };
    let (before, after_create, after_delete, count) = on_own_thread(async {
        let keys = |page: linear_rat::linear::TeamIssuesPage| -> Vec<String> {
            page.issues
                .into_iter()
                .map(|issue| issue.identifier)
                .collect()
        };
        let before = keys(FakeClient.team_issues(&blocked, None).await.unwrap());
        FakeClient
            .create_relation(&RelationCreate {
                issue_id: "issue-eng-6".to_owned(),
                related_issue_id: "issue-eng-60".to_owned(),
                kind: RelationType::Blocks,
            })
            .await
            .unwrap();
        let after_create = keys(FakeClient.team_issues(&blocked, None).await.unwrap());
        let count = FakeClient.count_team_issues(&blocked).await.unwrap();
        // relation-1 is the demo's ENG-6 blocks ENG-51.
        FakeClient.delete_relation("relation-1").await.unwrap();
        let after_delete = keys(FakeClient.team_issues(&blocked, None).await.unwrap());
        (before, after_create, after_delete, count)
    });

    assert!(!before.contains(&"ENG-60".to_owned()), "{before:?}");
    assert!(
        after_create.contains(&"ENG-60".to_owned()),
        "{after_create:?}"
    );
    assert_eq!(count, after_create.len());
    assert!(
        !after_delete.contains(&"ENG-51".to_owned()),
        "{after_delete:?}"
    );
    assert!(
        after_delete.contains(&"ENG-60".to_owned()),
        "{after_delete:?}"
    );
}

#[test]
fn a_stand_in_relation_is_not_offered_for_removal() {
    let mut app = focused_with_detail("ENG-51");
    let created = press(&mut app, "R3ENG-60\n");
    assert_eq!(creates(&created).len(), 1);

    press(&mut app, "Dr");

    assert_eq!(removable(&app), ["relation-2", "relation-1", "relation-3"]);
    assert!(!shows(&app, "related to ENG-60"), "{:#?}", screen(&app));
}

#[test]
fn the_kind_chooser() {
    let mut app = sized(80, 24);
    focus(&mut app, "ENG-51");

    press(&mut app, "R");

    assert_snapshot!(app.backend());
}

#[test]
fn the_related_issue_picker() {
    let mut app = focused_with_detail("ENG-51");

    press(&mut app, "gr");

    assert_snapshot!(app.backend());
}

#[test]
fn a_newer_copy_a_detail_brings_shows_in_the_list_at_once() {
    let mut app = sized(120, 30);
    focus(&mut app, "ENG-51");
    let id = app.state().focused_issue().unwrap().id.clone();
    let mut detail = IssueDetail::demo(&id);
    let sub_issue = detail
        .related_issues
        .iter_mut()
        .find(|issue| issue.identifier == "ENG-60")
        .unwrap();
    sub_issue.title = "Name the cheese in every sync error".to_owned();
    sub_issue.updated_at += jiff::SignedDuration::from_secs(60);

    app.handle(Event::IssueDetailLoaded {
        issue_id: id,
        result: Ok(detail),
    })
    .unwrap();

    let row = row(&app, "ENG-60").unwrap();
    assert!(row.contains("ENG-60 Name the cheese"), "{row}");
}
