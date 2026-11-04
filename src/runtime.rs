//! The runtime loop: turns terminal input and the passing of time into
//! events, feeds them to the app and carries out the effects it asks for.

use std::future::Future;
use std::io;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context;
use crossterm::event::Event as TerminalEvent;
use futures_util::stream::FuturesUnordered;
use futures_util::{Stream, StreamExt};
use jiff::{SignedDuration, Timestamp};
use ratatui::backend::Backend;
use tokio::time::{Instant, MissedTickBehavior};

use crate::app::{App, Effect, Event, LoadError, SyncFailure, TeamPoll, Write};
use crate::linear::{ClientError, LinearClient, Urgency};
use crate::snapshot::SnapshotCache;
use crate::{browser, clipboard};

/// How often [`Event::Tick`] goes out.
const TICK: Duration = Duration::from_secs(1);

/// A request in flight, finishing with the event that carries its result.
type Pending<'a> = Pin<Box<dyn Future<Output = Event> + 'a>>;

/// Every request in flight. They run at the same time and finish in any
/// order; each result comes back as an event.
type InFlight<'a> = FuturesUnordered<Pending<'a>>;

/// What effects reach beyond the app: Linear through `client`, which is
/// `None` when there is no usable API key (the app then already shows why),
/// and the snapshot cache, which is `None` under `--demo`.
struct Reach<'a, C> {
    client: Option<&'a C>,
    snapshots: Option<&'a SnapshotCache>,
}

/// Runs until the app asks to quit, `shutdown` resolves, or terminal input
/// ends, then saves the snapshot. Waits without drawing while nothing
/// happens, apart from a tick a second that lets polls fall due. Input keeps
/// being handled while requests are in flight.
pub async fn run<B, C, I, S>(
    app: &mut App<B>,
    client: Option<&C>,
    snapshots: Option<&SnapshotCache>,
    input: I,
    shutdown: S,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
    I: Stream<Item = io::Result<TerminalEvent>> + Unpin,
    S: Future<Output = ()>,
{
    let reach = Reach { client, snapshots };
    let result = run_loop(app, &reach, input, shutdown).await;
    save_snapshot(app, &reach);
    result
}

async fn run_loop<B, C, I, S>(
    app: &mut App<B>,
    reach: &Reach<'_, C>,
    mut input: I,
    shutdown: S,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
    I: Stream<Item = io::Result<TerminalEvent>> + Unpin,
    S: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    let clock = Clock::start();
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut in_flight: InFlight<'_> = FuturesUnordered::new();
    let mut next = Some(Event::Start);
    loop {
        if let Some(event) = next.take()
            && feed(app, event, reach, &mut in_flight, &mut System)?.is_break()
        {
            return Ok(());
        }
        next = tokio::select! {
            () = &mut shutdown => {
                tracing::info!("shutdown signal received");
                return Ok(());
            }
            Some(event) = in_flight.next(), if !in_flight.is_empty() => Some(event),
            terminal_event = input.next() => match terminal_event {
                Some(event) => from_terminal(event.context("reading terminal input")?),
                None => return Ok(()),
            },
            _ = ticks.tick() => Some(Event::Tick(clock.now())),
        };
    }
}

/// The wall clock at the start, moved on by tokio's monotonic clock, so a
/// paused test clock moves it too.
struct Clock {
    started: Timestamp,
    instant: Instant,
}

impl Clock {
    fn start() -> Self {
        Self {
            started: Timestamp::now(),
            instant: Instant::now(),
        }
    }

    fn now(&self) -> Timestamp {
        SignedDuration::try_from(self.instant.elapsed())
            .ok()
            .and_then(|elapsed| self.started.checked_add(elapsed).ok())
            .unwrap_or(self.started)
    }
}

/// The world outside lrat that effects reach. Tests swap in a fake.
trait Outside {
    fn copy(&mut self, text: &str) -> anyhow::Result<()>;
    fn open(&mut self, url: &str) -> anyhow::Result<()>;
}

/// The terminal's clipboard and the viewer's browser.
struct System;

impl Outside for System {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        clipboard::copy(&mut io::stdout(), text).context("could not copy to the clipboard")?;
        match clipboard::tmux_problem() {
            Some(problem) => Err(anyhow::anyhow!(problem)),
            None => Ok(()),
        }
    }

    fn open(&mut self, url: &str) -> anyhow::Result<()> {
        browser::open(url)
    }
}

/// Hands `event` to the app and carries out the effects it returns.
fn feed<'a, B, C>(
    app: &mut App<B>,
    event: Event,
    reach: &Reach<'a, C>,
    in_flight: &mut InFlight<'a>,
    outside: &mut impl Outside,
) -> anyhow::Result<ControlFlow<()>>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
{
    for effect in app.handle(event).context("drawing the screen")? {
        if execute(app, effect, reach, in_flight, outside)?.is_break() {
            return Ok(ControlFlow::Break(()));
        }
    }
    Ok(ControlFlow::Continue(()))
}

/// Carries out `effect`. A request joins `in_flight` and finishes later; a
/// failure to copy or open becomes the notice rather than ending the loop.
fn execute<'a, B, C>(
    app: &mut App<B>,
    effect: Effect,
    reach: &Reach<'a, C>,
    in_flight: &mut InFlight<'a>,
    outside: &mut impl Outside,
) -> anyhow::Result<ControlFlow<()>>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
{
    let outcome = match effect {
        Effect::Quit => return Ok(ControlFlow::Break(())),
        Effect::LoadMyIssues => {
            in_flight.push(Box::pin(load_my_issues(reach.client)));
            Ok(())
        }
        Effect::Poll(teams) => {
            in_flight.push(Box::pin(poll(reach.client, teams)));
            if reach.client.is_some_and(LinearClient::throttled) {
                return feed(app, Event::Throttled, reach, in_flight, outside);
            }
            Ok(())
        }
        Effect::Reconcile(ids) => {
            in_flight.push(Box::pin(reconcile(reach.client, ids)));
            Ok(())
        }
        Effect::SaveSnapshot => {
            save_snapshot_in_background(app, reach);
            Ok(())
        }
        Effect::LoadWorkflowStates { team_id } => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let result = match client {
                    Some(client) => client.workflow_states(&team_id).await.map_err(message),
                    None => Err(NO_CLIENT.to_owned()),
                };
                Event::WorkflowStatesLoaded { team_id, result }
            }));
            Ok(())
        }
        Effect::LoadTeamMembers { team_id } => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let result = match client {
                    Some(client) => client.team_members(&team_id).await.map_err(message),
                    None => Err(NO_CLIENT.to_owned()),
                };
                Event::TeamMembersLoaded { team_id, result }
            }));
            Ok(())
        }
        Effect::UpdateIssue(write) => {
            in_flight.push(Box::pin(update_issue(reach.client, write)));
            Ok(())
        }
        Effect::LoadIssueDetail { issue_id, urgency } => {
            in_flight.push(Box::pin(load_issue_detail(reach.client, issue_id, urgency)));
            Ok(())
        }
        Effect::Search(term) => {
            in_flight.push(Box::pin(search(reach.client, term)));
            Ok(())
        }
        Effect::LoadTeams => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                Event::TeamsLoaded(ask(client, "could not load your teams", |c| c.teams()).await)
            }));
            Ok(())
        }
        Effect::LoadTeamOptions(team_id) => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let what = "could not load the team's filters";
                Event::TeamOptionsLoaded(ask(client, what, |c| c.team_options(&team_id)).await)
            }));
            Ok(())
        }
        Effect::LoadTeamIssues { query, after } => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let what = "could not load the team's issues";
                let result = ask(client, what, |c| c.team_issues(&query, after.as_deref())).await;
                Event::TeamIssuesLoaded {
                    query,
                    after,
                    result,
                }
            }));
            Ok(())
        }
        Effect::CountTeamIssues(query) => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let what = "could not count the team's issues";
                let result = ask(client, what, |c| c.count_team_issues(&query)).await;
                Event::TeamIssuesCounted { query, result }
            }));
            Ok(())
        }
        Effect::Copy(text) => outside.copy(&text),
        Effect::OpenUrl(url) => outside.open(&url),
    };
    if let Err(error) = outcome {
        let failed = Event::EffectFailed(format!("{error:#}"));
        return feed(app, failed, reach, in_flight, outside);
    }
    Ok(ControlFlow::Continue(()))
}

/// Writes the store to the snapshot cache, if there is one, before
/// returning. The cache only speeds up the next launch, so a failure is
/// logged and nothing more.
fn save_snapshot<B: Backend, C>(app: &App<B>, reach: &Reach<'_, C>) {
    let Some(snapshots) = reach.snapshots else {
        return;
    };
    let state = app.state();
    let saved = snapshots.save_in_order(
        snapshots.next_save(),
        &state.store,
        state.sync_status.last_synced(),
    );
    if let Err(error) = saved {
        tracing::warn!("could not save the snapshot: {error:#}");
    }
}

/// [`save_snapshot`] on a blocking thread, so encoding, writing and syncing
/// the file never hold up input or drawing. Saves still land in the order
/// they were asked for.
fn save_snapshot_in_background<B: Backend, C>(app: &App<B>, reach: &Reach<'_, C>) {
    let Some(snapshots) = reach.snapshots else {
        return;
    };
    let snapshots = snapshots.clone();
    let ticket = snapshots.next_save();
    let state = app.state();
    let store = state.store.clone();
    let synced_at = state.sync_status.last_synced();
    tokio::task::spawn_blocking(move || {
        if let Err(error) = snapshots.save_in_order(ticket, &store, synced_at) {
            tracing::warn!("could not save the snapshot: {error:#}");
        }
    });
}

async fn load_my_issues<C: LinearClient>(client: Option<&C>) -> Event {
    let Some(client) = client else {
        return Event::MyIssuesLoaded(Err(LoadError::NoKey));
    };
    let result = client
        .assigned_issues()
        .await
        .map_err(|error| LoadError::from(&error));
    match &result {
        Ok(loaded) => tracing::info!(issues = loaded.issues.len(), "loaded My Issues"),
        Err(error) => tracing::warn!(?error, "loading My Issues failed"),
    }
    Event::MyIssuesLoaded(result)
}

/// Polls `teams` one after another, so a poll never bursts the budget, and
/// stops at the first that fails.
async fn poll<C: LinearClient>(client: Option<&C>, teams: Vec<TeamPoll>) -> Event {
    let Some(client) = client else {
        let failure = SyncFailure::Failed("there is no API key".to_owned());
        return Event::Polled {
            changes: Vec::new(),
            failure: Some(failure),
        };
    };
    let mut changes = Vec::new();
    for team in teams {
        match client.changed_issues(&team.team_id, team.since).await {
            Ok(changed) => changes.push(changed),
            Err(error) => {
                tracing::warn!(?error, team = team.team_id, "a delta poll failed");
                return Event::Polled {
                    changes,
                    failure: Some(SyncFailure::from(&error)),
                };
            }
        }
    }
    let issues: usize = changes.iter().map(|team| team.issues.len()).sum();
    tracing::debug!(teams = changes.len(), issues, "polled");
    Event::Polled {
        changes,
        failure: None,
    }
}

async fn reconcile<C: LinearClient>(client: Option<&C>, checked: Vec<String>) -> Event {
    let result = match client {
        Some(client) => client.existing_issues(&checked).await.map_err(|error| {
            tracing::warn!(?error, "the reconciliation failed");
            SyncFailure::from(&error)
        }),
        None => Err(SyncFailure::Failed("there is no API key".to_owned())),
    };
    Event::Reconciled { checked, result }
}

/// Why a request could not go out when there is no usable API key.
const NO_CLIENT: &str = "there is no API key to reach Linear with";

/// `error` with each of its causes, for the notice.
fn message(error: ClientError) -> String {
    format!("{:#}", anyhow::Error::new(error))
}

async fn update_issue<C: LinearClient>(client: Option<&C>, write: Write) -> Event {
    let result = match client {
        Some(client) => client
            .update_issue(&write.issue_id, &write.edit.update())
            .await
            .map(Box::new)
            .map_err(message),
        None => Err(NO_CLIENT.to_owned()),
    };
    match &result {
        Ok(_) => tracing::info!(issue = %write.identifier, "Linear applied an edit"),
        Err(error) => tracing::warn!(issue = %write.identifier, %error, "Linear rejected an edit"),
    }
    Event::IssueUpdated { write, result }
}

async fn load_issue_detail<C: LinearClient>(
    client: Option<&C>,
    issue_id: String,
    urgency: Urgency,
) -> Event {
    let result = match client {
        Some(client) => client
            .issue_detail(&issue_id, urgency)
            .await
            .map_err(|error| format!("{:#}", anyhow::Error::new(error))),
        None => Err("there is no API key to load it with".to_owned()),
    };
    match &result {
        Ok(detail) => tracing::info!(
            comments = detail.comments.len(),
            spans = detail.state_history.len(),
            "loaded an issue's detail"
        ),
        Err(error) => tracing::warn!(%error, "loading an issue's detail failed"),
    }
    Event::IssueDetailLoaded { issue_id, result }
}

async fn search<C: LinearClient>(client: Option<&C>, term: String) -> Event {
    let result = match client {
        Some(client) => client
            .search_issues(&term)
            .await
            .map_err(|error| format!("Search failed: {:#}", anyhow::Error::new(error))),
        None => Err("Search needs an API key; run `lrat auth login`".to_owned()),
    };
    match &result {
        Ok(found) => tracing::info!(issues = found.issues.len(), "searched Linear"),
        Err(error) => tracing::warn!(%error, "searching Linear failed"),
    }
    Event::SearchDone { term, result }
}

/// Sends `request` through `client`. A failure, or no client for lack of an
/// API key, becomes a message that starts with `what`.
async fn ask<'a, C, T, R>(
    client: Option<&'a C>,
    what: &str,
    request: impl FnOnce(&'a C) -> R,
) -> Result<T, String>
where
    C: LinearClient,
    R: Future<Output = Result<T, ClientError>>,
{
    let result = match client {
        Some(client) => request(client)
            .await
            .map_err(|error| format!("{:#}", anyhow::Error::new(error).context(what.to_owned()))),
        None => Err(format!("{what}: there is no API key")),
    };
    if let Err(message) = &result {
        tracing::warn!(%message, "a request failed");
    }
    result
}

fn from_terminal(event: TerminalEvent) -> Option<Event> {
    match event {
        TerminalEvent::Key(key) => Some(Event::Key(key)),
        TerminalEvent::Resize(columns, rows) => Some(Event::Resize { columns, rows }),
        TerminalEvent::FocusGained => Some(Event::FocusGained),
        TerminalEvent::FocusLost => Some(Event::FocusLost),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::State;
    use crate::linear::FakeClient;

    /// Records what lrat asked for, and fails the requests it is told to.
    #[derive(Default)]
    struct Fake {
        copied: Vec<String>,
        opened: Vec<String>,
        failure: Option<&'static str>,
    }

    impl Outside for Fake {
        fn copy(&mut self, text: &str) -> anyhow::Result<()> {
            self.copied.push(text.to_owned());
            self.failure
                .map_or(Ok(()), |failure| Err(anyhow::anyhow!(failure)))
        }

        fn open(&mut self, url: &str) -> anyhow::Result<()> {
            self.opened.push(url.to_owned());
            self.failure
                .map_or(Ok(()), |failure| Err(anyhow::anyhow!(failure)))
        }
    }

    fn screen(app: &App<TestBackend>) -> String {
        let buffer = app.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Carries out `effect` with no client and no load in flight.
    fn run_effect(
        app: &mut App<TestBackend>,
        effect: Effect,
        outside: &mut Fake,
    ) -> ControlFlow<()> {
        let mut in_flight = FuturesUnordered::new();
        let reach = Reach {
            client: None::<&FakeClient>,
            snapshots: None,
        };
        execute(app, effect, &reach, &mut in_flight, outside).unwrap()
    }

    #[test]
    fn a_copy_goes_to_the_clipboard_and_the_loop_goes_on() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let flow = run_effect(&mut app, Effect::Copy("DES-34".to_owned()), &mut outside);

        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(outside.copied, ["DES-34"]);
    }

    #[test]
    fn an_open_url_goes_to_the_browser() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let url = "https://linear.app/squeakworks/issue/DES-34";
        let flow = run_effect(&mut app, Effect::OpenUrl(url.to_owned()), &mut outside);

        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(outside.opened, [url]);
    }

    #[test]
    fn a_failed_effect_becomes_the_notice_and_the_loop_goes_on() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake {
            failure: Some("could not open the browser with xdg-open"),
            ..Fake::default()
        };

        let flow = run_effect(
            &mut app,
            Effect::OpenUrl("https://x".to_owned()),
            &mut outside,
        );

        assert_eq!(flow, ControlFlow::Continue(()));
        assert!(
            screen(&app).contains("could not open the browser with xdg-open"),
            "{}",
            screen(&app)
        );
    }

    #[test]
    fn requests_stay_in_flight_together() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut in_flight = FuturesUnordered::new();

        for _ in 0..2 {
            let reach = Reach {
                client: Some(&FakeClient),
                snapshots: None,
            };
            for effect in [Effect::LoadMyIssues, Effect::Poll(Vec::new())] {
                let flow = execute(
                    &mut app,
                    effect,
                    &reach,
                    &mut in_flight,
                    &mut Fake::default(),
                )
                .unwrap();
                assert_eq!(flow, ControlFlow::Continue(()));
            }
        }

        assert_eq!(in_flight.len(), 4);
    }

    #[tokio::test]
    async fn a_search_joins_the_requests_in_flight_and_comes_back_as_an_event() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut in_flight = FuturesUnordered::new();

        let reach = Reach {
            client: Some(&FakeClient),
            snapshots: None,
        };
        let flow = execute(
            &mut app,
            Effect::Search("cheese cave".to_owned()),
            &reach,
            &mut in_flight,
            &mut Fake::default(),
        )
        .unwrap();
        let event = in_flight.next().await.unwrap();

        assert_eq!(flow, ControlFlow::Continue(()));
        let Event::SearchDone { term, result } = event else {
            panic!("{event:?}");
        };
        assert_eq!(term, "cheese cave");
        let found: Vec<String> = result
            .unwrap()
            .issues
            .into_iter()
            .map(|issue| issue.identifier)
            .collect();
        assert_eq!(found, ["OPS-36"]);
    }

    #[tokio::test]
    async fn a_search_without_a_key_says_how_to_add_one() {
        let event = search(None::<&FakeClient>, "cheese".to_owned()).await;

        let Event::SearchDone { result, .. } = event else {
            panic!("{event:?}");
        };
        assert!(result.unwrap_err().contains("lrat auth login"));
    }

    #[test]
    fn quit_ends_the_loop_without_going_outside() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let flow = run_effect(&mut app, Effect::Quit, &mut outside);

        assert_eq!(flow, ControlFlow::Break(()));
        assert!(outside.copied.is_empty() && outside.opened.is_empty());
    }
}
