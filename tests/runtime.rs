use std::future::{self, Future};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
use futures_util::stream::{self, Stream, StreamExt};
use jiff::Timestamp;
use linear_rat::app::{App, State};
use linear_rat::auth::KeySource;
use linear_rat::linear::model::{Issue, Member, Viewer, WorkflowState, Workspace};
use linear_rat::linear::{
    AssignedIssues, ClientError, FakeClient, IssueDetail, IssueUpdate, LinearClient, TeamChanges,
    Urgency, demo,
};
use linear_rat::runtime;
use linear_rat::snapshot::SnapshotCache;
use ratatui::backend::TestBackend;

fn press(c: char) -> io::Result<TerminalEvent> {
    Ok(TerminalEvent::Key(KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::NONE,
    )))
}

/// Terminal input that delivers `events` and then stays open, as a real
/// terminal does.
fn input(
    events: Vec<io::Result<TerminalEvent>>,
) -> impl Stream<Item = io::Result<TerminalEvent>> + Unpin {
    stream::iter(events).chain(stream::pending())
}

async fn run(
    events: Vec<io::Result<TerminalEvent>>,
    shutdown: impl Future<Output = ()>,
) -> Option<anyhow::Result<()>> {
    let mut app = App::new(TestBackend::new(80, 24), State::default()).unwrap();
    let running = runtime::run(&mut app, Some(&FakeClient), None, input(events), shutdown);
    tokio::time::timeout(Duration::from_millis(200), running)
        .await
        .ok()
}

/// Runs `app` with `client` and no input until it settles, then hands back
/// what it shows.
async fn settle(app: App<TestBackend>, client: Option<&impl LinearClient>) -> String {
    settle_after(app, client, "").await
}

/// Runs `app` with `client`, types `keys`, and hands back what it shows
/// once it settles.
async fn settle_after(
    mut app: App<TestBackend>,
    client: Option<&impl LinearClient>,
    keys: &str,
) -> String {
    let typed = keys.chars().map(press).collect();
    let running = runtime::run(&mut app, client, None, input(typed), future::pending());
    let stopped = tokio::time::timeout(Duration::from_millis(200), running).await;
    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    let buffer = app.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A client for a key Linear does not accept.
struct RejectingClient;

impl LinearClient for RejectingClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn changed_issues(
        &self,
        _: &str,
        _: Option<Timestamp>,
    ) -> Result<TeamChanges, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn existing_issues(&self, _: &[String]) -> Result<Vec<String>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn workflow_states(&self, _team_id: &str) -> Result<Vec<WorkflowState>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn team_members(&self, _team_id: &str) -> Result<Vec<Member>, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn update_issue(
        &self,
        _issue_id: &str,
        _update: &IssueUpdate,
    ) -> Result<Issue, ClientError> {
        Err(ClientError::InvalidKey)
    }

    async fn issue_detail(&self, _: &str, _: Urgency) -> Result<IssueDetail, ClientError> {
        Err(ClientError::InvalidKey)
    }
}

#[tokio::test]
async fn stops_when_q_is_pressed() {
    let result = run(vec![press('x'), press('q')], future::pending()).await;

    assert!(matches!(result, Some(Ok(()))));
}

#[tokio::test]
async fn keeps_running_until_asked_to_stop() {
    let result = run(vec![press('x')], future::pending()).await;

    assert!(result.is_none());
}

#[tokio::test]
async fn stops_on_a_shutdown_signal() {
    let result = run(Vec::new(), future::ready(())).await;

    assert!(matches!(result, Some(Ok(()))));
}

#[tokio::test]
async fn stops_with_an_error_when_terminal_input_fails() {
    let broken = Err(io::Error::other("tty gone"));

    let result = run(vec![broken], future::pending()).await;

    let error = result.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("tty gone"));
}

#[tokio::test]
async fn loads_my_issues_through_the_client_at_start() {
    let app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();

    let shown = settle(app, Some(&FakeClient)).await;

    let demo = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
    let demo_shown = settle(demo, Some(&FakeClient)).await;
    assert_eq!(shown, demo_shown);
}

#[tokio::test]
async fn a_rejected_key_ends_on_its_error_screen_naming_the_source() {
    let app = App::new(
        TestBackend::new(80, 24),
        State::loading(Some(KeySource::Env)),
    )
    .unwrap();

    let shown = settle(app, Some(&RejectingClient)).await;

    assert!(shown.contains("API key rejected"), "{shown}");
    assert!(shown.contains("from LINEAR_API_KEY"), "{shown}");
}

#[tokio::test]
async fn q_quits_while_the_load_is_still_in_flight() {
    struct SlowClient;
    impl LinearClient for SlowClient {
        async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
            future::pending().await
        }
        async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
            future::pending().await
        }
        async fn changed_issues(
            &self,
            _: &str,
            _: Option<Timestamp>,
        ) -> Result<TeamChanges, ClientError> {
            future::pending().await
        }
        async fn existing_issues(&self, _: &[String]) -> Result<Vec<String>, ClientError> {
            future::pending().await
        }
        async fn workflow_states(&self, _: &str) -> Result<Vec<WorkflowState>, ClientError> {
            future::pending().await
        }
        async fn team_members(&self, _: &str) -> Result<Vec<Member>, ClientError> {
            future::pending().await
        }
        async fn update_issue(&self, _: &str, _: &IssueUpdate) -> Result<Issue, ClientError> {
            future::pending().await
        }
        async fn issue_detail(&self, _: &str, _: Urgency) -> Result<IssueDetail, ClientError> {
            future::pending().await
        }
    }
    let mut app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();

    let running = runtime::run(
        &mut app,
        Some(&SlowClient),
        None,
        input(vec![press('q')]),
        future::pending(),
    );
    let result = tokio::time::timeout(Duration::from_millis(200), running).await;

    assert!(matches!(result, Ok(Ok(()))), "{result:?}");
}

/// The demo workspace, counting the delta polls asked of it.
#[derive(Default)]
struct CountingClient {
    deltas: AtomicUsize,
}

impl LinearClient for CountingClient {
    async fn viewer(&self) -> Result<(Viewer, Workspace), ClientError> {
        FakeClient.viewer().await
    }
    async fn assigned_issues(&self) -> Result<AssignedIssues, ClientError> {
        FakeClient.assigned_issues().await
    }
    async fn changed_issues(
        &self,
        team_id: &str,
        since: Option<Timestamp>,
    ) -> Result<TeamChanges, ClientError> {
        self.deltas.fetch_add(1, Ordering::SeqCst);
        FakeClient.changed_issues(team_id, since).await
    }
    async fn existing_issues(&self, ids: &[String]) -> Result<Vec<String>, ClientError> {
        FakeClient.existing_issues(ids).await
    }
    async fn workflow_states(&self, team_id: &str) -> Result<Vec<WorkflowState>, ClientError> {
        FakeClient.workflow_states(team_id).await
    }
    async fn team_members(&self, team_id: &str) -> Result<Vec<Member>, ClientError> {
        FakeClient.team_members(team_id).await
    }
    async fn update_issue(
        &self,
        issue_id: &str,
        update: &IssueUpdate,
    ) -> Result<Issue, ClientError> {
        FakeClient.update_issue(issue_id, update).await
    }
    async fn issue_detail(
        &self,
        issue_id: &str,
        urgency: Urgency,
    ) -> Result<IssueDetail, ClientError> {
        FakeClient.issue_detail(issue_id, urgency).await
    }
}

#[tokio::test(start_paused = true)]
async fn polls_every_team_after_the_load_and_then_every_active_interval() {
    let client = CountingClient::default();
    let mut app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();
    let teams = demo::workspace().teams.len();

    let running = runtime::run(
        &mut app,
        Some(&client),
        None,
        input(Vec::new()),
        future::pending(),
    );
    let stopped = tokio::time::timeout(Duration::from_millis(31_500), running).await;

    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    let deltas = client.deltas.load(Ordering::SeqCst);
    assert_eq!(deltas, 3 * teams, "polls at 0s, 15s and 30s");
}

#[tokio::test(start_paused = true)]
async fn quitting_saves_the_snapshot_the_next_launch_paints_from() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("runtime-snapshot-{}", std::process::id()));
    let cache = SnapshotCache::new(dir);
    let mut app = App::new(TestBackend::new(80, 24), State::loading(None)).unwrap();
    // q comes a minute in, once the load and the first polls are done.
    let late_q = stream::once(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        press('q')
    });

    let running = runtime::run(
        &mut app,
        Some(&FakeClient),
        Some(&cache),
        Box::pin(late_q.chain(stream::pending())),
        future::pending(),
    );
    let quit = tokio::time::timeout(Duration::from_secs(61), running).await;

    assert!(matches!(quit, Ok(Ok(()))), "{quit:?}");
    let saved = cache.load().unwrap();
    assert_eq!(saved.store, app.state().store);
    assert!(saved.store.issues().count() > 0);
    assert!(saved.synced_at.is_some());
}

/// The row of the issue with `identifier` on `shown`.
fn row<'a>(shown: &'a str, identifier: &str) -> &'a str {
    shown
        .lines()
        .find(|line| line.contains(&format!("{identifier} ")))
        .unwrap_or_default()
}

#[tokio::test(start_paused = true)]
async fn an_edit_goes_through_the_client_and_its_answer_stays() {
    let app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();

    let shown = settle_after(app, Some(&FakeClient), "p1").await;

    assert!(row(&shown, "DES-34").contains("!!! Urgent"), "{shown}");
    assert!(!row(&shown, "DES-34").contains('✘'), "{shown}");
}

#[tokio::test(start_paused = true)]
async fn an_edit_linear_rejects_is_rolled_back_with_the_reason() {
    let app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();

    let shown = settle_after(app, Some(&RejectingClient), "p1").await;

    assert!(row(&shown, "DES-34").contains("▮▮▮ High"), "{shown}");
    assert!(row(&shown, "DES-34").contains('✘'), "{shown}");
    assert!(
        shown.contains("Could not change the priority of DES-34: Linear rejected the API key"),
        "{shown}"
    );
}

#[tokio::test]
async fn opening_the_detail_loads_its_comments_and_history_through_the_client() {
    let mut app = App::new(TestBackend::new(80, 60), State::demo()).unwrap();

    let running = runtime::run(
        &mut app,
        Some(&FakeClient),
        None,
        input(vec![Ok(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))]),
        future::pending(),
    );
    let stopped = tokio::time::timeout(Duration::from_millis(200), running).await;

    assert!(stopped.is_err(), "the runtime stopped: {stopped:?}");
    let buffer = app.backend().buffer();
    let shown: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(shown.contains("Comments ("), "{shown}");
    assert!(shown.contains("so far"), "{shown}");
    assert!(!shown.contains("Loading"), "{shown}");
}
