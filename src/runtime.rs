//! The runtime loop: turns terminal input and the passing of time into
//! events, feeds them to the app and carries out the effects it asks for.

use std::cell::Cell;
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

use crate::app::{
    App, Effect, Event, Lifecycle, LifecycleWrite, LoadError, NewComment, NewIssue, SyncFailure,
    TeamPoll, Write,
};
use crate::linear::{AnalyticsWindow, ClientError, LinearClient, LiveEvent, SearchQuery, Urgency};
use crate::snapshot::SnapshotCache;
use crate::terminal::JobControl;
use crate::{browser, clipboard, terminal};

/// How often [`Event::Tick`] goes out.
const TICK: Duration = Duration::from_secs(1);

/// A request in flight, finishing with the event that carries its result.
type Pending<'a> = Pin<Box<dyn Future<Output = Event> + 'a>>;

/// Every request in flight. They run at the same time and finish in any
/// order; each result comes back as an event.
type InFlight<'a> = FuturesUnordered<Pending<'a>>;

/// Live events from Linear's websocket, once the app has gone live.
type Live<'a> = Pin<Box<dyn Stream<Item = LiveEvent> + 'a>>;

/// What effects reach beyond the app: Linear through `client`, which is
/// `None` when there is no usable API key (the app then already shows why),
/// the snapshot cache, which is `None` under `--demo`, and the live events
/// once [`Effect::GoLive`] has opened them.
struct Reach<'a, C> {
    client: Option<&'a C>,
    snapshots: Option<&'a SnapshotCache>,
    live: Cell<Option<Live<'a>>>,
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
    let no_job_control = futures_util::stream::pending();
    run_with_job_control(app, client, snapshots, input, shutdown, no_job_control).await
}

/// [`run`], suspending or resuming lrat on each `job_control` signal. The
/// loop goes on after either.
pub async fn run_with_job_control<B, C, I, S, J>(
    app: &mut App<B>,
    client: Option<&C>,
    snapshots: Option<&SnapshotCache>,
    input: I,
    shutdown: S,
    job_control: J,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
    I: Stream<Item = io::Result<TerminalEvent>> + Unpin,
    S: Future<Output = ()>,
    J: Stream<Item = JobControl>,
{
    let reach = Reach {
        client,
        snapshots,
        live: Cell::new(None),
    };
    let result = run_loop(app, &reach, input, shutdown, job_control).await;
    save_snapshot(app, &reach);
    result
}

async fn run_loop<B, C, I, S, J>(
    app: &mut App<B>,
    reach: &Reach<'_, C>,
    mut input: I,
    shutdown: S,
    job_control: J,
) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    C: LinearClient,
    I: Stream<Item = io::Result<TerminalEvent>> + Unpin,
    S: Future<Output = ()>,
    J: Stream<Item = JobControl>,
{
    tokio::pin!(shutdown);
    tokio::pin!(job_control);
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
        // Out of `reach` while waiting, so effects can open it meanwhile;
        // put back before the next event is fed.
        let mut live = reach.live.take();
        next = tokio::select! {
            () = &mut shutdown => {
                tracing::info!("shutdown signal received");
                return Ok(());
            }
            Some(event) = in_flight.next(), if !in_flight.is_empty() => Some(event),
            Some(event) = next_live(&mut live) => Some(Event::Live(event)),
            Some(signal) = job_control.next() => on_job_control(app, signal, &mut System)?,
            terminal_event = input.next() => match terminal_event {
                Some(event) => from_terminal(event.context("reading terminal input")?),
                None => return Ok(()),
            },
            _ = ticks.tick() => Some(Event::Tick(clock.now())),
        };
        reach.live.set(live);
    }
}

/// The next live event, or never without live events.
async fn next_live(live: &mut Option<Live<'_>>) -> Option<LiveEvent> {
    match live {
        Some(live) => live.next().await,
        None => std::future::pending().await,
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
    /// Runs the viewer's editor on `text` until it exits, and returns what
    /// it saved.
    fn edit(&mut self, text: &str) -> anyhow::Result<String>;
    /// Hands the terminal to the shell and stops lrat until it is
    /// continued, then takes the terminal back.
    fn suspend(&mut self) -> anyhow::Result<()>;
    /// Takes the terminal back after lrat was continued.
    fn resume(&mut self) -> anyhow::Result<()>;
}

/// The terminal's clipboard, the viewer's browser and their editor.
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

    fn edit(&mut self, text: &str) -> anyhow::Result<String> {
        terminal::edit(text)
    }

    fn suspend(&mut self) -> anyhow::Result<()> {
        terminal::suspend().context("could not suspend lrat")
    }

    fn resume(&mut self) -> anyhow::Result<()> {
        terminal::resume().context("could not take the terminal back")
    }
}

/// Suspends or resumes as `signal` asks and draws the whole screen, which
/// the shell may have drawn over. A failure comes back as the event that
/// shows it.
fn on_job_control<B>(
    app: &mut App<B>,
    signal: JobControl,
    outside: &mut impl Outside,
) -> anyhow::Result<Option<Event>>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    tracing::info!(?signal, "job control");
    let result = match signal {
        JobControl::Suspend => outside.suspend(),
        JobControl::Continue => outside.resume(),
    };
    app.redraw_all().context("drawing the screen")?;
    Ok(result
        .err()
        .map(|error| Event::EffectFailed(format!("{error:#}"))))
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
        // Like the editor, the stop holds the loop: nothing is read or drawn
        // until lrat is continued and has the terminal back.
        Effect::Suspend => {
            let result = outside.suspend();
            app.redraw_all().context("drawing the screen")?;
            result
        }
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
        Effect::GoLive(team_ids) => {
            let live = reach.live.take();
            match (live, reach.client) {
                (None, Some(client)) => {
                    tracing::info!(teams = team_ids.len(), "opening live updates");
                    reach.live.set(Some(Box::pin(client.live_events(team_ids))));
                }
                (live, _) => reach.live.set(live),
            }
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
        Effect::LoadTeamLabels { team_id } => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let result = match client {
                    Some(client) => client.team_labels(&team_id).await.map_err(message),
                    None => Err(NO_CLIENT.to_owned()),
                };
                Event::TeamLabelsLoaded { team_id, result }
            }));
            Ok(())
        }
        Effect::LoadProjectMilestones { project_id } => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let result = match client {
                    Some(client) => client
                        .project_milestones(&project_id)
                        .await
                        .map_err(message),
                    None => Err(NO_CLIENT.to_owned()),
                };
                Event::ProjectMilestonesLoaded { project_id, result }
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
        Effect::Search(query) => {
            in_flight.push(Box::pin(search(reach.client, query)));
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
        Effect::LoadAnalytics(window) => {
            in_flight.push(Box::pin(load_analytics(reach.client, window)));
            Ok(())
        }
        Effect::LoadAdministrableTeams => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let what = "could not load the teams you administer";
                Event::AdministrableTeamsLoaded(
                    ask(client, what, |c| c.administrable_teams()).await,
                )
            }));
            Ok(())
        }
        Effect::LoadTeamAnalytics { team_id, window } => {
            let client = reach.client;
            in_flight.push(Box::pin(async move {
                let what = "could not load team analytics";
                let result = ask(client, what, |c| c.team_analytics(&team_id, window)).await;
                if let Ok(loaded) = &result {
                    tracing::info!(issues = loaded.issues.len(), "loaded team analytics");
                }
                Event::TeamAnalyticsLoaded {
                    team_id,
                    window,
                    result,
                }
            }));
            Ok(())
        }
        Effect::Copy(text) => outside.copy(&text),
        Effect::OpenUrl(url) => outside.open(&url),
        // The editor owns the terminal until it exits, so the loop waits for
        // it here: no input is read and nothing is drawn meanwhile. Input is
        // only read between events, so nothing else reads the terminal now.
        Effect::OpenEditor { writing, text } => {
            let result = outside.edit(&text).map_err(|error| format!("{error:#}"));
            app.redraw_all().context("drawing the screen")?;
            let closed = Event::EditorClosed { writing, result };
            return feed(app, closed, reach, in_flight, outside);
        }
        Effect::ChangeLifecycle(write) => {
            in_flight.push(Box::pin(change_lifecycle(reach.client, write)));
            Ok(())
        }
        Effect::CreateComment(comment) => {
            in_flight.push(Box::pin(create_comment(reach.client, comment)));
            Ok(())
        }
        Effect::CreateIssue(issue) => {
            in_flight.push(Box::pin(create_issue(reach.client, issue)));
            Ok(())
        }
        Effect::UpdateIssueText { write, seen, text } => {
            in_flight.push(Box::pin(update_issue_text(reach.client, write, seen, text)));
            Ok(())
        }
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
            .update_issue(&write.issue_id, &write.update())
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

async fn change_lifecycle<C: LinearClient>(client: Option<&C>, write: LifecycleWrite) -> Event {
    let result = match client {
        Some(client) => {
            let id = write.issue_id.as_str();
            match write.change {
                Lifecycle::Archive => client.archive_issue(id).await,
                Lifecycle::Unarchive => client.unarchive_issue(id).await,
                Lifecycle::Trash => client.trash_issue(id).await,
                Lifecycle::Subscribe => client.subscribe_to_issue(id).await,
                Lifecycle::Unsubscribe => client.unsubscribe_from_issue(id).await,
            }
            .map_err(message)
        }
        None => Err(NO_CLIENT.to_owned()),
    };
    match &result {
        Ok(()) => {
            tracing::info!(issue = %write.identifier, change = ?write.change, "Linear made a lifecycle change")
        }
        Err(error) => {
            tracing::warn!(issue = %write.identifier, change = ?write.change, %error, "Linear rejected a lifecycle change")
        }
    }
    Event::LifecycleChanged { write, result }
}

/// Reads the issue's `updatedAt` again and sends `write` only if it is still
/// `seen`. If it moved, `text` is kept in a file and the error names it.
/// Linear has no compare-and-set, so an edit landing between the read and
/// the write is not caught.
async fn update_issue_text<C: LinearClient>(
    client: Option<&C>,
    write: Write,
    seen: Timestamp,
    text: String,
) -> Event {
    let Some(client) = client else {
        let result = Err(NO_CLIENT.to_owned());
        return Event::IssueUpdated { write, result };
    };
    let result = match client.issue_updated_at(&write.issue_id).await {
        Err(error) => Err(format!(
            "could not check it is unchanged in Linear: {}",
            message(error)
        )),
        Ok(now) if now != seen => {
            tracing::info!(issue = %write.identifier, "refused a save over a newer edit");
            Err(match terminal::keep(&text, &write.identifier) {
                Ok(path) => format!(
                    "it changed in Linear while the editor was open, so nothing was sent; \
                     your text is in {}",
                    path.display()
                ),
                Err(error) => format!(
                    "it changed in Linear while the editor was open, so nothing was sent, \
                     and your text could not be kept: {error:#}"
                ),
            })
        }
        Ok(_) => {
            return update_issue(Some(client), write).await;
        }
    };
    Event::IssueUpdated { write, result }
}

async fn create_comment<C: LinearClient>(client: Option<&C>, comment: NewComment) -> Event {
    let result = match client {
        Some(client) => client
            .create_comment(&comment.issue_id, &comment.body)
            .await
            .map_err(message),
        None => Err(NO_CLIENT.to_owned()),
    };
    match &result {
        Ok(_) => tracing::info!(issue = %comment.identifier, "Linear stored a comment"),
        Err(error) => {
            tracing::warn!(issue = %comment.identifier, %error, "Linear rejected a comment");
        }
    }
    Event::CommentCreated { comment, result }
}

async fn create_issue<C: LinearClient>(client: Option<&C>, issue: NewIssue) -> Event {
    let result = match client {
        Some(client) => client
            .create_issue(&issue.input)
            .await
            .map(Box::new)
            .map_err(message),
        None => Err(NO_CLIENT.to_owned()),
    };
    match &result {
        Ok(created) => tracing::info!(issue = %created.identifier, "Linear created an issue"),
        Err(error) => tracing::warn!(%error, "Linear did not create an issue"),
    }
    Event::IssueCreated { issue, result }
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

async fn search<C: LinearClient>(client: Option<&C>, query: SearchQuery) -> Event {
    let result = match client {
        Some(client) => client
            .search_issues(&query)
            .await
            .map_err(|error| format!("Search failed: {:#}", anyhow::Error::new(error))),
        None => Err("Search needs an API key; run `lrat auth login`".to_owned()),
    };
    match &result {
        Ok(found) => tracing::info!(issues = found.issues.len(), "searched Linear"),
        Err(error) => tracing::warn!(%error, "searching Linear failed"),
    }
    Event::SearchDone { query, result }
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

async fn load_analytics<C: LinearClient>(client: Option<&C>, window: AnalyticsWindow) -> Event {
    let result = match client {
        Some(client) => client
            .analytics_issues(window)
            .await
            .map_err(|error| format!("{:#}", anyhow::Error::new(error))),
        None => Err("there is no API key to load analytics with".to_owned()),
    };
    match &result {
        Ok(loaded) => tracing::info!(issues = loaded.issues.len(), "loaded analytics"),
        Err(error) => tracing::warn!(%error, "loading analytics failed"),
    }
    Event::AnalyticsLoaded { window, result }
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
        /// The text each editor opened on.
        edited: Vec<String>,
        /// What the editor saves.
        saves: String,
        failure: Option<&'static str>,
        suspended: usize,
        resumed: usize,
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

        fn edit(&mut self, text: &str) -> anyhow::Result<String> {
            self.edited.push(text.to_owned());
            match self.failure {
                Some(failure) => Err(anyhow::anyhow!(failure)),
                None => Ok(self.saves.clone()),
            }
        }

        fn suspend(&mut self) -> anyhow::Result<()> {
            self.suspended += 1;
            self.failure
                .map_or(Ok(()), |failure| Err(anyhow::anyhow!(failure)))
        }

        fn resume(&mut self) -> anyhow::Result<()> {
            self.resumed += 1;
            Ok(())
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
            live: Cell::new(None),
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
                live: Cell::new(None),
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
            live: Cell::new(None),
        };
        let flow = execute(
            &mut app,
            Effect::Search(SearchQuery::new("cheese cave")),
            &reach,
            &mut in_flight,
            &mut Fake::default(),
        )
        .unwrap();
        let event = in_flight.next().await.unwrap();

        assert_eq!(flow, ControlFlow::Continue(()));
        let Event::SearchDone { query, result } = event else {
            panic!("{event:?}");
        };
        assert_eq!(query.term, "cheese cave");
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
        let event = search(None::<&FakeClient>, SearchQuery::new("cheese")).await;

        let Event::SearchDone { result, .. } = event else {
            panic!("{event:?}");
        };
        assert!(result.unwrap_err().contains("lrat auth login"));
    }

    fn comment_on_first_issue(app: &App<TestBackend>) -> Effect {
        let issue = app.state().focused_issue().unwrap();
        Effect::OpenEditor {
            writing: crate::app::Writing::Comment {
                issue_id: issue.id.clone(),
                identifier: issue.identifier.clone(),
            },
            text: "draft".to_owned(),
        }
    }

    #[test]
    fn the_editor_gets_the_text_and_its_buffer_becomes_a_comment_in_flight() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let effect = comment_on_first_issue(&app);
        let mut outside = Fake {
            saves: "Looks good\n".to_owned(),
            ..Fake::default()
        };
        let mut in_flight = FuturesUnordered::new();
        let reach = Reach {
            client: Some(&FakeClient),
            snapshots: None,
            live: Cell::new(None),
        };
        let frames = app.frames_drawn();

        let flow = execute(&mut app, effect, &reach, &mut in_flight, &mut outside).unwrap();

        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(outside.edited, ["draft"]);
        assert!(
            app.frames_drawn() > frames,
            "the screen was not drawn again"
        );
        assert_eq!(in_flight.len(), 1);
    }

    #[test]
    fn an_editor_that_fails_says_why_and_sends_nothing() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let effect = comment_on_first_issue(&app);
        let mut outside = Fake {
            failure: Some("`nvim` exited with exit status: 1"),
            ..Fake::default()
        };
        let mut in_flight = FuturesUnordered::new();
        let reach = Reach {
            client: Some(&FakeClient),
            snapshots: None,
            live: Cell::new(None),
        };

        let flow = execute(&mut app, effect, &reach, &mut in_flight, &mut outside).unwrap();

        assert_eq!(flow, ControlFlow::Continue(()));
        assert!(in_flight.is_empty());
        assert!(
            screen(&app).contains("`nvim` exited with"),
            "{}",
            screen(&app)
        );
    }

    /// A save of a new title for the demo's DES-34, opened when its
    /// `updatedAt` was the demo's.
    fn text_write() -> (Write, Timestamp) {
        let issue = crate::linear::demo::workspace()
            .issues
            .into_iter()
            .find(|issue| issue.identifier == "DES-34")
            .unwrap();
        let write = Write {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            edit: crate::app::Edit::Text {
                title: "Autumn release, take two".to_owned(),
                description: "Body".to_owned(),
            },
            previous: crate::app::Editable::Text.current(&issue),
        };
        (write, issue.updated_at)
    }

    #[tokio::test]
    async fn a_text_save_over_a_newer_edit_sends_nothing_and_keeps_the_text_in_a_file() {
        use std::os::unix::fs::PermissionsExt;

        let (write, seen) = text_write();
        let teammate = crate::linear::IssueUpdate {
            priority: Some(crate::linear::model::Priority::Urgent),
            ..crate::linear::IssueUpdate::default()
        };
        FakeClient
            .update_issue(&write.issue_id, &teammate)
            .await
            .unwrap();
        let text = "Autumn release, take two\n\nBody\n".to_owned();

        let event = update_issue_text(Some(&FakeClient), write.clone(), seen, text.clone()).await;

        let Event::IssueUpdated { result, .. } = event else {
            panic!("{event:?}");
        };
        let error = result.unwrap_err();
        let path = error.rsplit("your text is in ").next().unwrap();
        let kept = std::fs::read_to_string(path).unwrap();
        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        std::fs::remove_file(path).unwrap();
        assert!(error.starts_with("it changed in Linear while the editor was open"));
        assert_eq!(kept, text);
        assert_eq!(mode, 0o600);
        let checked = FakeClient.issue_updated_at(&write.issue_id).await.unwrap();
        assert_eq!(
            checked,
            seen + SignedDuration::from_secs(1),
            "a second write went out"
        );
    }

    #[tokio::test]
    async fn a_text_save_over_an_unchanged_issue_goes_to_linear() {
        let (write, seen) = text_write();

        let event = update_issue_text(Some(&FakeClient), write, seen, String::new()).await;

        let Event::IssueUpdated { result, .. } = event else {
            panic!("{event:?}");
        };
        let stored = result.unwrap();
        assert_eq!(stored.title, "Autumn release, take two");
        assert_eq!(stored.description.as_deref(), Some("Body"));
    }

    #[test]
    fn quit_ends_the_loop_without_going_outside() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();

        let flow = run_effect(&mut app, Effect::Quit, &mut outside);

        assert_eq!(flow, ControlFlow::Break(()));
        assert!(outside.copied.is_empty() && outside.opened.is_empty());
    }

    #[test]
    fn suspend_stops_lrat_then_draws_the_whole_screen_and_the_loop_goes_on() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();
        let frames = app.frames_drawn();

        let flow = run_effect(&mut app, Effect::Suspend, &mut outside);

        assert_eq!(flow, ControlFlow::Continue(()));
        assert_eq!(outside.suspended, 1);
        assert!(
            app.frames_drawn() > frames,
            "the screen was not drawn again"
        );
    }

    #[test]
    fn a_suspend_that_fails_says_why() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake {
            failure: Some("could not suspend lrat: Operation not permitted"),
            ..Fake::default()
        };

        let flow = run_effect(&mut app, Effect::Suspend, &mut outside);

        assert_eq!(flow, ControlFlow::Continue(()));
        assert!(
            screen(&app).contains("could not suspend lrat"),
            "{}",
            screen(&app)
        );
    }

    #[test]
    fn sigtstp_from_outside_suspends_as_the_key_does() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();
        let frames = app.frames_drawn();

        let next = on_job_control(&mut app, JobControl::Suspend, &mut outside).unwrap();

        assert_eq!(next, None);
        assert_eq!((outside.suspended, outside.resumed), (1, 0));
        assert!(
            app.frames_drawn() > frames,
            "the screen was not drawn again"
        );
    }

    #[test]
    fn sigcont_takes_the_terminal_back_and_draws_the_whole_screen() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake::default();
        let frames = app.frames_drawn();

        let next = on_job_control(&mut app, JobControl::Continue, &mut outside).unwrap();

        assert_eq!(next, None);
        assert_eq!((outside.suspended, outside.resumed), (0, 1));
        assert!(
            app.frames_drawn() > frames,
            "the screen was not drawn again"
        );
    }

    #[test]
    fn a_failed_sigtstp_suspend_comes_back_as_the_notice() {
        let mut app = App::new(TestBackend::new(80, 24), State::demo()).unwrap();
        let mut outside = Fake {
            failure: Some("could not suspend lrat"),
            ..Fake::default()
        };

        let next = on_job_control(&mut app, JobControl::Suspend, &mut outside).unwrap();

        assert_eq!(
            next,
            Some(Event::EffectFailed("could not suspend lrat".to_owned()))
        );
    }
}
