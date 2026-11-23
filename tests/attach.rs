//! Attaching a file or a link to the focused issue through the app driver:
//! the prompts and what they refuse, Tab completing a path, the upload's
//! progress in the indicator, and what the screen shows once Linear
//! answers. The tests run from the crate root, so its files make the
//! paths.

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use insta::assert_snapshot;
use linear_rat::app::{
    Action, App, Attaching, Effect, Event, Failure, Modal, NewAttachment, State, WriteClass,
};
use linear_rat::keys;
use linear_rat::linear::model::Attachment;
use linear_rat::linear::{ErrorKind, FakeClient, IssueDetail, LinearClient, Urgency};
use ratatui::backend::TestBackend;

/// The issue My Issues focuses first in the demo.
const FIRST: &str = "DES-34";
const FIRST_ID: &str = "issue-des-34";

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

fn enter(app: &mut App<TestBackend>) -> Vec<Effect> {
    app.handle(key(KeyCode::Enter)).unwrap()
}

fn tab(app: &mut App<TestBackend>) -> Vec<Effect> {
    app.handle(key(KeyCode::Tab)).unwrap()
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

/// What the open attach prompt holds as typed.
fn typed(app: &App<TestBackend>) -> String {
    match app.state().modals.top() {
        Some(Modal::Attach(prompt)) => prompt.text.clone(),
        other => panic!("expected the attach prompt, got {other:?}"),
    }
}

fn error(app: &App<TestBackend>) -> Option<String> {
    match app.state().modals.top() {
        Some(Modal::Attach(prompt)) => prompt.error.clone(),
        other => panic!("expected the attach prompt, got {other:?}"),
    }
}

/// The only effect, which must attach something.
fn only_attach(effects: Vec<Effect>) -> NewAttachment {
    match effects.as_slice() {
        [Effect::Attach(attachment)] => attachment.clone(),
        other => panic!("expected one attachment, got {other:?}"),
    }
}

/// The demo app with the first issue's detail loaded, so its attachments
/// are known.
fn with_detail() -> App<TestBackend> {
    let mut app = app();
    app.handle(Event::IssueDetailLoaded {
        issue_id: FIRST_ID.to_owned(),
        result: Ok(IssueDetail::demo(FIRST_ID)),
    })
    .unwrap();
    app
}

fn attachments(app: &App<TestBackend>) -> Vec<Attachment> {
    app.state()
        .store
        .attachments(FIRST_ID)
        .unwrap_or_default()
        .to_vec()
}

fn stored(url: &str, title: &str) -> Attachment {
    Attachment {
        id: "attachment-new".to_owned(),
        issue_id: FIRST_ID.to_owned(),
        url: url.to_owned(),
        title: title.to_owned(),
        subtitle: None,
        source_type: None,
        status: None,
    }
}

fn run<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

// The file prompt.

#[test]
fn shift_f_opens_the_file_prompt_with_the_size_limit() {
    let mut app = app();

    let effects = press(&mut app, "F");

    assert!(effects.is_empty(), "{effects:?}");
    assert!(matches!(app.state().modals.top(), Some(Modal::Attach(_))));
    assert_snapshot!(app.backend());
}

#[test]
fn a_missing_file_keeps_the_prompt_open_with_the_reason() {
    let mut app = app();
    press(&mut app, "F/nonexistent/rind.log");

    let effects = enter(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        error(&app).as_deref(),
        Some("No file at /nonexistent/rind.log")
    );
    assert_snapshot!(app.backend());
    press(&mut app, "x");
    assert_eq!(error(&app), None);
}

#[test]
fn a_directory_is_refused() {
    let mut app = app();
    press(&mut app, "Fsrc");

    let effects = enter(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(error(&app).as_deref(), Some("src is a directory"));
}

#[test]
fn a_file_over_the_limit_is_refused_before_asking_linear() {
    let dir = std::env::temp_dir().join(format!("lrat-attach-big-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let big = dir.join("wheel.bin");
    // Sparse, so the test writes almost nothing.
    fs::File::create(&big).unwrap().set_len(60_000_000).unwrap();
    let mut app = app();
    press(&mut app, &format!("F{}", big.display()));

    let effects = enter(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        error(&app).as_deref(),
        Some("wheel.bin is 60.0 MB; the limit is 50 MB")
    );
}

#[test]
fn a_home_path_expands_before_it_is_checked() {
    if !std::env::var_os("HOME").is_some_and(|home| Path::new(&home).is_dir()) {
        return;
    }
    let mut app = app();
    press(&mut app, "F~");

    enter(&mut app);

    assert_eq!(error(&app).as_deref(), Some("~ is a directory"));
}

#[test]
fn tab_completes_the_only_entry_that_starts_with_the_typed_name() {
    let mut app = app();
    press(&mut app, "FCargo.l");

    let effects = tab(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(typed(&app), "Cargo.lock");
}

#[test]
fn tab_completes_a_directory_with_its_slash() {
    let mut app = app();
    press(&mut app, "Fsr");

    tab(&mut app);

    assert_eq!(typed(&app), "src/");
}

#[test]
fn tab_completes_as_far_as_the_entries_agree_and_lists_them() {
    let mut app = app();
    press(&mut app, "FCar");

    tab(&mut app);

    assert_eq!(typed(&app), "Cargo.");
    assert!(shows(&app, "Cargo.lock"), "{:#?}", screen(&app));
    assert!(shows(&app, "Cargo.toml"), "{:#?}", screen(&app));
}

#[test]
fn tab_leaves_hidden_entries_out_unless_the_name_starts_with_a_dot() {
    let dir = std::env::temp_dir().join(format!("lrat-attach-hidden-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".notes"), "").unwrap();
    fs::write(dir.join("notes.txt"), "").unwrap();
    let dir = dir.display();
    let mut app = app();
    press(&mut app, &format!("F{dir}/"));

    tab(&mut app);

    assert_eq!(typed(&app), format!("{dir}/notes.txt"));

    let mut app = self::app();
    press(&mut app, &format!("F{dir}/."));

    tab(&mut app);

    assert_eq!(typed(&app), format!("{dir}/.notes"));
}

#[test]
fn tab_with_nothing_to_complete_says_so() {
    let mut app = app();
    press(&mut app, "Fzz-nothing-");

    tab(&mut app);

    assert_eq!(typed(&app), "zz-nothing-");
    assert_eq!(
        error(&app).as_deref(),
        Some("Nothing here starts with zz-nothing-")
    );
}

#[test]
fn tab_is_available_in_the_file_prompt() {
    let mut app = app();
    let toggle = keys::spec(Action::ToggleEntry).unwrap();

    press(&mut app, "F");

    assert!((toggle.available)(app.state()));
}

#[test]
fn a_readable_file_starts_its_upload_and_closes_the_prompt() {
    let mut app = app();
    press(&mut app, "FCargo.toml");

    let attachment = only_attach(enter(&mut app));

    assert!(app.state().modals.top().is_none());
    assert_eq!(attachment.issue_id, FIRST_ID);
    assert_eq!(attachment.identifier, FIRST);
    let Attaching::File { file, .. } = &attachment.what else {
        panic!("{attachment:?}");
    };
    assert_eq!(file.name, "Cargo.toml");
    assert_eq!(file.content_type, "application/toml");
    assert_eq!(file.size, fs::metadata("Cargo.toml").unwrap().len());
    assert_eq!(file.path, Path::new("Cargo.toml"));
}

#[test]
fn an_unknown_extension_goes_up_as_octet_stream() {
    let mut app = app();
    press(&mut app, "FLICENSE-MIT");

    let attachment = only_attach(enter(&mut app));

    let Attaching::File { file, .. } = &attachment.what else {
        panic!("{attachment:?}");
    };
    assert_eq!(file.content_type, "application/octet-stream");
}

// The upload's progress and Linear's answer.

#[test]
fn the_upload_shows_its_progress_in_the_indicator_until_it_is_attached() {
    let mut app = with_detail();
    press(&mut app, "FCargo.toml");
    let attachment = only_attach(enter(&mut app));
    let Attaching::File { upload, file } = &attachment.what else {
        panic!("{attachment:?}");
    };
    assert!(
        shows(&app, "uploading Cargo.toml 0%"),
        "{:#?}",
        screen(&app)
    );

    app.handle(Event::UploadProgress {
        upload: *upload,
        sent: file.size * 42 / 100 + 1,
    })
    .unwrap();

    assert!(
        shows(&app, "uploading Cargo.toml 42%"),
        "{:#?}",
        screen(&app)
    );
    assert_snapshot!(app.backend());

    app.handle(Event::Attached {
        attachment: attachment.clone(),
        result: Ok(stored(
            "https://uploads.linear.app/a/Cargo.toml",
            "Cargo.toml",
        )),
    })
    .unwrap();

    assert!(!shows(&app, "uploading"), "{:#?}", screen(&app));
    assert!(shows(&app, "Attached Cargo.toml to DES-34"));
    assert!(
        attachments(&app)
            .iter()
            .any(|attachment| attachment.title == "Cargo.toml")
    );
}

#[test]
fn a_failed_upload_clears_the_indicator_and_goes_to_the_notice_log_and_capabilities() {
    let mut app = with_detail();
    press(&mut app, "FCargo.toml");
    let attachment = only_attach(enter(&mut app));
    let before = attachments(&app);

    app.handle(Event::Attached {
        attachment,
        result: Err(Failure::new(
            ErrorKind::Forbidden,
            "attach Cargo.toml to DES-34",
            "Linear returned errors: Forbidden (FORBIDDEN)",
        )),
    })
    .unwrap();

    assert!(!shows(&app, "uploading"), "{:#?}", screen(&app));
    assert!(shows(&app, "Could not attach Cargo.toml to DES-34"));
    assert_eq!(app.state().errors.newest_first().count(), 1);
    assert_eq!(attachments(&app), before);
    assert!(
        app.state()
            .write_denied(WriteClass::Attach)
            .is_some_and(|reason| reason.contains("cannot attach to issues"))
    );
}

#[test]
fn attaching_the_same_url_again_replaces_the_attachment() {
    let mut app = with_detail();
    let count = attachments(&app).len();
    press(&mut app, "Whttps://example.com/cave");
    enter(&mut app);
    let attachment = only_attach(enter(&mut app));

    for title in ["Cave", "Cave notes"] {
        app.handle(Event::Attached {
            attachment: attachment.clone(),
            result: Ok(stored("https://example.com/cave", title)),
        })
        .unwrap();
    }

    let now = attachments(&app);
    assert_eq!(now.len(), count + 1);
    assert_eq!(now.last().unwrap().title, "Cave notes");
}

// The link prompts.

#[test]
fn shift_w_opens_the_link_prompt() {
    let mut app = app();

    press(&mut app, "W");

    assert_snapshot!(app.backend());
}

#[test]
fn a_url_that_is_not_http_is_refused() {
    let mut app = app();
    press(&mut app, "Wftp://example.com/cave");

    let effects = enter(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        error(&app).as_deref(),
        Some("Use a URL that starts with http:// or https://")
    );
    assert_snapshot!(app.backend());
}

#[test]
fn a_url_without_a_host_is_refused() {
    let mut app = app();
    press(&mut app, "Whttps://");

    enter(&mut app);

    assert_eq!(
        error(&app).as_deref(),
        Some("Use a URL that starts with http:// or https://")
    );
}

#[test]
fn a_good_url_asks_for_an_optional_title_then_sends_both() {
    let mut app = app();
    press(&mut app, "Whttps://example.com/cave");

    let effects = enter(&mut app);

    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(typed(&app), "");
    assert_snapshot!(app.backend());
    press(&mut app, "Cave notes");
    let attachment = only_attach(enter(&mut app));
    assert_eq!(
        attachment.what,
        Attaching::Link {
            url: "https://example.com/cave".to_owned(),
            title: Some("Cave notes".to_owned()),
        }
    );
    assert!(app.state().modals.top().is_none());
}

#[test]
fn an_empty_title_lets_linear_name_the_link() {
    let mut app = app();
    press(&mut app, "Whttps://example.com/cave");
    enter(&mut app);

    let attachment = only_attach(enter(&mut app));

    assert_eq!(
        attachment.what,
        Attaching::Link {
            url: "https://example.com/cave".to_owned(),
            title: None,
        }
    );
}

#[test]
fn esc_closes_the_prompt_without_attaching() {
    let mut app = app();
    press(&mut app, "Whttps://example.com");

    let effects = app.handle(key(KeyCode::Esc)).unwrap();

    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.state().modals.top().is_none());
}

// The demo.

#[test]
fn the_demo_attaches_a_file_and_a_link_without_network() {
    let upload = linear_rat::linear::FileUpload {
        path: "Cargo.toml".into(),
        name: "Cargo.toml".to_owned(),
        content_type: "application/toml".to_owned(),
        size: 42,
    };
    let heard = Arc::new(Mutex::new(Vec::new()));
    let progress = {
        let heard = Arc::clone(&heard);
        move |sent| heard.lock().unwrap().push(sent)
    };

    let (file, link, detail) = run(async {
        let file = FakeClient
            .attach_file(FIRST_ID, &upload, progress)
            .await
            .unwrap();
        let link = FakeClient
            .attach_link(FIRST_ID, "https://example.com/cave", None)
            .await
            .unwrap();
        let detail = FakeClient
            .issue_detail(FIRST_ID, Urgency::Urgent)
            .await
            .unwrap();
        (file, link, detail)
    });

    assert_eq!(file.title, "Cargo.toml");
    assert_eq!(link.title, "example.com");
    assert_eq!(heard.lock().unwrap().last(), Some(&42));
    assert!(detail.attachments.contains(&file));
    assert!(detail.attachments.contains(&link));
}
