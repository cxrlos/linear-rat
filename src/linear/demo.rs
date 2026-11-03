//! The fictional workspace behind `--demo`, snapshot tests and the README
//! recording: Squeakworks, a small company making Nibble, a pantry and cheese
//! cellar app.
//!
//! Everything is derived from [`NOW`] and a fixed seed, so every call returns
//! the same data on every machine.

use jiff::{SignedDuration, Timestamp};

use super::model::{
    Comment, Cycle, Estimation, Issue, Label, Member, Priority, Project, StateSpan, StateType,
    Team, Viewer, WorkflowState, Workspace,
};

/// The demo's "now": every date in the demo workspace is anchored to it.
pub const NOW: Timestamp = Timestamp::constant(1_793_631_600, 0); // 2026-11-02T15:00:00Z

/// Start of the active cycle, a Monday a week before [`NOW`].
const ACTIVE_CYCLE_START: i64 = 1_792_972_800; // 2026-10-26T00:00:00Z
const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;
const CYCLE_DAYS: i64 = 14;
const PAST_CYCLES: i64 = 6;
const SEED: u64 = 0x5eed_c4ee_5e00;
const URL_KEY: &str = "squeakworks";

/// Everything in the demo workspace, as the model types the app uses.
#[derive(Debug, Clone, PartialEq)]
pub struct DemoWorkspace {
    pub workspace: Workspace,
    pub viewer: Viewer,
    pub members: Vec<Member>,
    pub teams: Vec<Team>,
    pub workflow_states: Vec<WorkflowState>,
    pub labels: Vec<Label>,
    pub projects: Vec<Project>,
    pub cycles: Vec<Cycle>,
    pub issues: Vec<Issue>,
    pub comments: Vec<Comment>,
}

/// Builds the demo workspace. Deterministic: no clock, no OS randomness.
pub fn workspace() -> DemoWorkspace {
    let mut rng = Rng(SEED);
    let members = MEMBERS
        .iter()
        .map(|&(handle, name)| Member {
            id: member_id(handle),
            name: name.to_owned(),
            display_name: handle.to_owned(),
        })
        .collect();
    let mut data = DemoWorkspace {
        workspace: Workspace {
            id: "workspace-squeakworks".to_owned(),
            name: "Squeakworks".to_owned(),
            url_key: URL_KEY.to_owned(),
        },
        viewer: Viewer {
            id: member_id(VIEWER),
            name: "Mira Gouda".to_owned(),
        },
        members,
        teams: Vec::new(),
        workflow_states: Vec::new(),
        labels: WORKSPACE_LABELS
            .iter()
            .map(|name| label(None, name))
            .collect(),
        projects: PROJECTS
            .iter()
            .enumerate()
            .map(|(index, name)| Project {
                id: format!("project-{}", index + 1),
                name: (*name).to_owned(),
            })
            .collect(),
        cycles: Vec::new(),
        issues: Vec::new(),
        comments: Vec::new(),
    };
    for spec in &TEAMS {
        add_team(&mut data, spec, &mut rng);
    }
    data
}

struct TeamSpec {
    key: &'static str,
    name: &'static str,
    estimation: Estimation,
    members: &'static [&'static str],
    labels: &'static [&'static str],
    /// Every `viewer_share`th issue of each kind goes to the viewer.
    viewer_share: usize,
    per_past_cycle: usize,
    in_active_cycle: usize,
    triage: usize,
    backlog: usize,
    unstarted: usize,
    started: usize,
    canceled: usize,
    titles: &'static [&'static str],
    descriptions: &'static [&'static str],
}

/// Where an issue ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Fate {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
}

/// The time window an issue's work happens in: its cycle, or the whole demo
/// history for issues outside a cycle.
#[derive(Clone, Copy)]
struct Window {
    cycle: Option<usize>,
    starts: i64,
    ends: i64,
}

/// An issue before it gets its number, which follows creation order.
struct Draft {
    fate: Fate,
    cycle_id: Option<String>,
    created: i64,
    triaged: Option<i64>,
    /// Entered the team's first started state.
    started: Option<i64>,
    /// Moved to review, the team's second started state.
    reviewed: Option<i64>,
    closed: Option<i64>,
    duplicate: bool,
    assignee: Option<String>,
    creator: String,
    priority: Priority,
    estimate: Option<f64>,
    label_ids: Vec<String>,
    project_id: Option<String>,
    description: Option<String>,
    comments: Vec<(i64, String, &'static str)>,
}

fn add_team(data: &mut DemoWorkspace, spec: &TeamSpec, rng: &mut Rng) {
    let team_id = format!("team-{}", spec.key.to_lowercase());
    data.teams.push(Team {
        id: team_id.clone(),
        key: spec.key.to_owned(),
        name: spec.name.to_owned(),
        estimation: spec.estimation,
    });
    data.workflow_states
        .extend(
            STATES
                .iter()
                .map(|&(slug, name, state_type, position)| WorkflowState {
                    id: state_id(&team_id, slug),
                    team_id: team_id.clone(),
                    name: name.to_owned(),
                    state_type,
                    position,
                }),
        );
    let team_labels: Vec<String> = spec
        .labels
        .iter()
        .map(|name| {
            let team_label = label(Some(&team_id), name);
            let id = team_label.id.clone();
            data.labels.push(team_label);
            id
        })
        .collect();
    let labels: Vec<String> = WORKSPACE_LABELS
        .iter()
        .map(|name| label_id(None, name))
        .chain(team_labels)
        .collect();

    let mut windows = Vec::new();
    for number in 1..=PAST_CYCLES + 1 {
        let starts = ACTIVE_CYCLE_START - (PAST_CYCLES + 1 - number) * CYCLE_DAYS * DAY;
        windows.push(Window {
            cycle: Some(data.cycles.len()),
            starts,
            ends: starts + CYCLE_DAYS * DAY,
        });
        data.cycles.push(Cycle {
            id: format!("{team_id}-cycle-{number}"),
            team_id: team_id.clone(),
            number: u32::try_from(number).unwrap_or(u32::MAX),
            name: None,
            starts_at: ts(starts),
            ends_at: ts(starts + CYCLE_DAYS * DAY),
            completed_at: (number <= PAST_CYCLES).then(|| ts(starts + CYCLE_DAYS * DAY)),
            scope_history: Vec::new(),
            completed_scope_history: Vec::new(),
            issue_count_history: Vec::new(),
            completed_issue_count_history: Vec::new(),
        });
    }
    let outside = Window {
        cycle: None,
        starts: windows[0].starts - 14 * DAY,
        ends: unix(NOW),
    };

    let mut plan: Vec<(Fate, Window)> = Vec::new();
    for (index, window) in windows.iter().enumerate() {
        if index < windows.len() - 1 {
            for slot in 0..spec.per_past_cycle {
                let fate = if (index * spec.per_past_cycle + slot) % 9 == 8 {
                    Fate::Canceled
                } else {
                    Fate::Completed
                };
                plan.push((fate, *window));
            }
        } else {
            for slot in 0..spec.in_active_cycle {
                let fate = [Fate::Started, Fate::Unstarted, Fate::Completed][slot % 3];
                plan.push((fate, *window));
            }
        }
    }
    for (fate, count) in [
        (Fate::Triage, spec.triage),
        (Fate::Backlog, spec.backlog),
        (Fate::Unstarted, spec.unstarted),
        (Fate::Started, spec.started),
        (Fate::Canceled, spec.canceled),
    ] {
        plan.extend(std::iter::repeat_n((fate, outside), count));
    }

    let mut seen = std::collections::HashMap::<Fate, usize>::new();
    let mut drafts: Vec<Draft> = plan
        .into_iter()
        .map(|(fate, window)| {
            let nth = seen.entry(fate).or_default();
            let share = if fate.is_open() {
                spec.viewer_share
            } else {
                spec.viewer_share * 2
            };
            let for_viewer = nth.is_multiple_of(share);
            *nth += 1;
            let cycle_id = window.cycle.map(|index| data.cycles[index].id.clone());
            draft(spec, fate, window, cycle_id, for_viewer, &labels, rng)
        })
        .collect();
    drafts.sort_by_key(|draft| draft.created);

    let first_cycle = data.cycles.len() - windows.len();
    let first_issue = data.issues.len();
    let mut comment_count = data.comments.len();
    for (index, draft) in drafts.into_iter().enumerate() {
        let number = index + 1;
        let (issue, comments) = finish(spec, &team_id, number, draft, &mut comment_count);
        data.issues.push(issue);
        data.comments.extend(comments);
    }
    let team_issues = &data.issues[first_issue..];
    for cycle in &mut data.cycles[first_cycle..] {
        fill_history(cycle, team_issues);
    }
}

impl Fate {
    fn is_open(self) -> bool {
        !matches!(self, Fate::Completed | Fate::Canceled)
    }
}

fn draft(
    spec: &TeamSpec,
    fate: Fate,
    window: Window,
    cycle_id: Option<String>,
    for_viewer: bool,
    labels: &[String],
    rng: &mut Rng,
) -> Draft {
    let in_cycle = window.cycle.is_some();
    let last = window.ends.min(unix(NOW));
    let created = match fate {
        Fate::Triage => unix(NOW) - rng.between(2 * HOUR, 10 * DAY),
        _ if in_cycle && rng.chance(25) => window.starts + rng.between(DAY + HOUR, 4 * DAY),
        _ if in_cycle => window.starts - rng.between(HOUR, 20 * DAY),
        Fate::Started => window.starts + rng.between(0, 60 * DAY),
        _ => window.starts + rng.between(0, (last - window.starts) - 3 * DAY),
    };
    let triaged =
        (fate != Fate::Triage && rng.chance(40)).then(|| created + rng.between(HOUR, DAY));
    let ready = triaged.unwrap_or(created);

    let mut started = None;
    let mut reviewed = None;
    let mut closed = None;
    match fate {
        Fate::Started | Fate::Completed => {
            let low = ready.max(window.starts) + HOUR;
            let start = rng.between(low, low + (last - low) / 2);
            started = Some(start);
            if fate == Fate::Completed {
                let done = rng.between(start + 2 * HOUR, last - HOUR);
                if rng.chance(60) {
                    reviewed = Some(rng.between(start + HOUR, done));
                }
                closed = Some(done);
            } else if rng.chance(40) {
                reviewed = Some(rng.between(start + HOUR, last - HOUR));
            }
        }
        Fate::Canceled => {
            let low = ready.max(window.starts) + HOUR;
            closed = Some(rng.between(low, last - HOUR));
        }
        Fate::Triage | Fate::Backlog | Fate::Unstarted => {}
    }

    let members = spec.members;
    let assignee = if for_viewer {
        Some(member_id(VIEWER))
    } else if fate == Fate::Triage || (fate == Fate::Backlog && rng.chance(30)) {
        None
    } else {
        Some(member_id(rng.pick::<&str>(members)))
    };
    let estimate = match spec.estimation {
        Estimation::NotUsed => None,
        _ if matches!(fate, Fate::Triage | Fate::Backlog) && rng.chance(50) => None,
        Estimation::Fibonacci => Some(*rng.pick(&[1.0, 2.0, 3.0, 5.0, 8.0])),
        Estimation::Exponential => Some(*rng.pick(&[1.0, 2.0, 4.0, 8.0, 16.0])),
        Estimation::Linear | Estimation::TShirt => Some(*rng.pick(&[1.0, 2.0, 3.0, 4.0, 5.0])),
    };
    let priority = *rng.pick(&[
        Priority::Urgent,
        Priority::High,
        Priority::High,
        Priority::Medium,
        Priority::Medium,
        Priority::Medium,
        Priority::Low,
        Priority::Low,
        Priority::None,
        Priority::None,
    ]);
    let mut label_ids = Vec::new();
    for _ in 0..rng.between(0, 3) {
        let label = rng.pick(labels).clone();
        if !label_ids.contains(&label) {
            label_ids.push(label);
        }
    }
    let project_id = rng
        .chance(50)
        .then(|| format!("project-{}", rng.between(1, PROJECTS.len() as i64 + 1)));
    let description = rng
        .chance(75)
        .then(|| (*rng.pick(spec.descriptions)).to_owned());

    let until = closed.unwrap_or(unix(NOW));
    let mut comments = Vec::new();
    if rng.chance(45) {
        for _ in 0..rng.between(1, 4) {
            let at = rng.between(created + HOUR, until);
            comments.push((
                at,
                member_id(rng.pick::<&str>(members)),
                *rng.pick(COMMENTS),
            ));
        }
        comments.sort_by_key(|&(at, _, _)| at);
    }

    Draft {
        fate,
        cycle_id,
        created,
        triaged,
        started,
        reviewed,
        closed,
        duplicate: fate == Fate::Canceled && rng.chance(30),
        assignee,
        creator: member_id(rng.pick::<&str>(members)),
        priority,
        estimate,
        label_ids,
        project_id,
        description,
        comments,
    }
}

fn finish(
    spec: &TeamSpec,
    team_id: &str,
    number: usize,
    draft: Draft,
    comment_count: &mut usize,
) -> (Issue, Vec<Comment>) {
    let identifier = format!("{}-{number}", spec.key);
    let issue_id = format!("issue-{}", identifier.to_lowercase());
    let state_of = |slug: &str| -> WorkflowState {
        let (_, name, state_type, position) = STATES
            .iter()
            .copied()
            .find(|&(candidate, ..)| candidate == slug)
            .unwrap_or(STATES[0]);
        WorkflowState {
            id: state_id(team_id, slug),
            team_id: team_id.to_owned(),
            name: name.to_owned(),
            state_type,
            position,
        }
    };

    let mut steps: Vec<(&str, i64)> = Vec::new();
    if draft.fate == Fate::Triage || draft.triaged.is_some() {
        steps.push(("triage", draft.created));
    }
    let waiting = match draft.fate {
        Fate::Backlog | Fate::Canceled => "backlog",
        _ => "todo",
    };
    if draft.fate != Fate::Triage {
        steps.push((waiting, draft.triaged.unwrap_or(draft.created)));
    }
    steps.extend(draft.started.map(|at| ("in-progress", at)));
    steps.extend(draft.reviewed.map(|at| ("in-review", at)));
    let closing = match draft.fate {
        Fate::Canceled if draft.duplicate => "duplicate",
        Fate::Canceled => "canceled",
        _ => "done",
    };
    steps.extend(draft.closed.map(|at| (closing, at)));

    let state_history = steps
        .iter()
        .enumerate()
        .map(|(index, &(slug, at))| {
            let state = state_of(slug);
            StateSpan {
                state_id: state.id,
                state_type: state.state_type,
                started_at: ts(at),
                ended_at: steps.get(index + 1).map(|&(_, next)| ts(next)),
            }
        })
        .collect();
    let current = steps.last().map_or("triage", |&(slug, _)| slug);
    let last_change = steps.last().map_or(draft.created, |&(_, at)| at);
    let last_comment = draft.comments.last().map_or(0, |&(at, ..)| at);

    let comments = draft
        .comments
        .into_iter()
        .map(|(at, author, body)| {
            *comment_count += 1;
            Comment {
                id: format!("comment-{comment_count}"),
                issue_id: issue_id.clone(),
                author_id: Some(author),
                body: body.to_owned(),
                created_at: ts(at),
            }
        })
        .collect();
    let title = spec.titles.get(number - 1).copied().unwrap_or("Untitled");
    let issue = Issue {
        id: issue_id,
        url: format!("https://linear.app/{URL_KEY}/issue/{identifier}"),
        branch_name: branch_name(&identifier, title),
        title: title.to_owned(),
        identifier,
        description: draft.description,
        team_id: team_id.to_owned(),
        state: state_of(current),
        priority: draft.priority,
        estimate: draft.estimate,
        assignee_id: draft.assignee,
        creator_id: Some(draft.creator),
        cycle_id: draft.cycle_id,
        project_id: draft.project_id,
        label_ids: draft.label_ids,
        created_at: ts(draft.created),
        updated_at: ts(last_change.max(last_comment)),
        started_at: draft.started.map(ts),
        completed_at: draft
            .closed
            .filter(|_| draft.fate == Fate::Completed)
            .map(ts),
        canceled_at: draft
            .closed
            .filter(|_| draft.fate == Fate::Canceled)
            .map(ts),
        triaged_at: draft.triaged.map(ts),
        state_history,
    };
    (issue, comments)
}

/// Fills the daily history arrays the way Linear reports them: one entry per
/// day since the cycle started, each measured at the end of that day (or at
/// [`NOW`] for the current day).
fn fill_history(cycle: &mut Cycle, issues: &[Issue]) {
    let starts = unix(cycle.starts_at);
    let last = unix(cycle.ends_at).min(unix(NOW));
    let days = (last - starts + DAY - 1) / DAY;
    let in_cycle: Vec<&Issue> = issues
        .iter()
        .filter(|issue| issue.cycle_id.as_ref() == Some(&cycle.id))
        .collect();
    for day in 0..days {
        let at = ts((starts + (day + 1) * DAY).min(last));
        let scope: Vec<&&Issue> = in_cycle
            .iter()
            .filter(|issue| issue.created_at <= at)
            .filter(|issue| issue.canceled_at.is_none_or(|canceled| canceled > at))
            .collect();
        let done: Vec<&&Issue> = scope
            .iter()
            .copied()
            .filter(|issue| issue.completed_at.is_some_and(|completed| completed <= at))
            .collect();
        let points = |issues: &[&&Issue]| issues.iter().filter_map(|issue| issue.estimate).sum();
        cycle.scope_history.push(points(&scope));
        cycle.completed_scope_history.push(points(&done));
        cycle.issue_count_history.push(count(scope.len()));
        cycle.completed_issue_count_history.push(count(done.len()));
    }
}

fn count(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn ts(seconds: i64) -> Timestamp {
    Timestamp::UNIX_EPOCH + SignedDuration::from_secs(seconds)
}

fn unix(timestamp: Timestamp) -> i64 {
    timestamp.as_second()
}

fn member_id(handle: &str) -> String {
    format!("member-{handle}")
}

fn state_id(team_id: &str, slug: &str) -> String {
    format!("{team_id}-state-{slug}")
}

fn label_id(team_id: Option<&str>, name: &str) -> String {
    let slug = name.replace(' ', "-");
    match team_id {
        Some(team_id) => format!("{team_id}-label-{slug}"),
        None => format!("label-{slug}"),
    }
}

fn label(team_id: Option<&str>, name: &str) -> Label {
    Label {
        id: label_id(team_id, name),
        team_id: team_id.map(str::to_owned),
        name: name.to_owned(),
    }
}

/// SplitMix64: small, fast, and the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `low..high`, or `low` when the range is empty.
    fn between(&mut self, low: i64, high: i64) -> i64 {
        match u64::try_from(high - low) {
            Ok(span) if span > 0 => low + i64::try_from(self.next() % span).unwrap_or(0),
            _ => low,
        }
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let index = usize::try_from(self.between(0, items.len() as i64)).unwrap_or(0);
        &items[index]
    }
}

const VIEWER: &str = "mira";

const MEMBERS: [(&str, &str); 9] = [
    ("mira", "Mira Gouda"),
    ("theo", "Theo Whisker"),
    ("ada", "Ada Roquefort"),
    ("jun", "Jun Emmental"),
    ("omar", "Omar Stilton"),
    ("iris", "Iris Comté"),
    ("nell", "Nell Havarti"),
    ("basil", "Basil Burrow"),
    ("pia", "Pia Brie"),
];

const STATES: [(&str, &str, StateType, f64); 8] = [
    ("triage", "Triage", StateType::Triage, 0.0),
    ("backlog", "Backlog", StateType::Backlog, 0.0),
    ("todo", "Todo", StateType::Unstarted, 0.0),
    ("in-progress", "In Progress", StateType::Started, 0.0),
    ("in-review", "In Review", StateType::Started, 1.0),
    ("done", "Done", StateType::Completed, 0.0),
    ("canceled", "Canceled", StateType::Canceled, 0.0),
    ("duplicate", "Duplicate", StateType::Canceled, 1.0),
];

const WORKSPACE_LABELS: [&str; 3] = ["bug", "feature", "improvement"];

const PROJECTS: [&str; 4] = [
    "Nibble 3.0",
    "Shared pantries",
    "Cellar sensors",
    "Cheese cave move",
];

const COMMENTS: &[&str] = &[
    "I can reproduce this on the staging pantry.",
    "Picking this up after standup.",
    "Pushed a fix, could someone take a look?",
    "Blocked on the sensor firmware update.",
    "Moved to the next cycle, we ran out of time.",
    "Looks good to me.",
    "Can we split this? The second half is bigger than it looks.",
    "Added screenshots from the cellar map.",
    "Customer reported this again today.",
    "Checked with design, the spacing is intentional.",
];

const TEAMS: [TeamSpec; 3] = [
    TeamSpec {
        key: "ENG",
        name: "Engineering",
        estimation: Estimation::Fibonacci,
        members: &["mira", "theo", "ada", "jun", "omar"],
        labels: &["backend", "mobile", "performance"],
        viewer_share: 2,
        per_past_cycle: 7,
        in_active_cycle: 12,
        triage: 4,
        backlog: 7,
        unstarted: 2,
        started: 1,
        canceled: 2,
        titles: ENG_TITLES,
        descriptions: ENG_DESCRIPTIONS,
    },
    TeamSpec {
        key: "DES",
        name: "Design",
        estimation: Estimation::Linear,
        members: &["iris", "nell", "pia", "mira"],
        labels: &["research", "brand"],
        viewer_share: 3,
        per_past_cycle: 4,
        in_active_cycle: 7,
        triage: 2,
        backlog: 3,
        unstarted: 2,
        started: 1,
        canceled: 1,
        titles: DES_TITLES,
        descriptions: DES_DESCRIPTIONS,
    },
    TeamSpec {
        key: "OPS",
        name: "Operations",
        estimation: Estimation::NotUsed,
        members: &["basil", "omar", "pia", "mira"],
        labels: &["infra", "on-call"],
        viewer_share: 3,
        per_past_cycle: 4,
        in_active_cycle: 7,
        triage: 2,
        backlog: 3,
        unstarted: 2,
        started: 1,
        canceled: 1,
        titles: OPS_TITLES,
        descriptions: OPS_DESCRIPTIONS,
    },
];

const ENG_DESCRIPTIONS: &[&str] = &[
    "Seen in production since the last release. Steps:\n\n1. Open a shared pantry\n2. Scan a wheel\n3. Watch the count jump by two\n\nExpected: the count goes up by one.",
    "The cellar service spends most of this request in one query. We should add an index or cache the result per household.",
    "Part of the Nibble 3.0 push. Keep the old behaviour behind a flag until the mobile apps catch up.",
    "Support asked for this twice this month. A small change on the API side, then a setting in the app.",
    "Flaky in CI roughly one run in ten. It fails on the timezone fixture, so start there.",
];

const DES_DESCRIPTIONS: &[&str] = &[
    "Start from the whisker components we already have. Share early sketches in the design channel before polishing.",
    "Households told us this screen is confusing. Keep the copy short and make the main action obvious.",
    "Needs light and dark variants, and must pass contrast checks.",
    "Bring two options to the critique on Thursday.",
];

const OPS_DESCRIPTIONS: &[&str] = &[
    "Follow the runbook and note anything that is out of date.",
    "Schedule this outside the evening sync peak and post in the ops channel before starting.",
    "Due before the end of the quarter. Finance needs the numbers for planning.",
    "Came out of last week's incident review.",
];

const ENG_TITLES: &[&str] = &[
    "Set up the crumb event pipeline staging cluster",
    "Add retry with backoff to pantry sync",
    "Cheese wheel barcode scanner drops the last digit",
    "Paginate the cellar inventory endpoint",
    "Cache aging schedules on the device",
    "Migrate burrow storage to the new bucket layout",
    "Fix timezone drift in aging reminders",
    "Export cellar reports to CSV",
    "Rate-limit the public recipes API",
    "Crash when a pantry has no shelves",
    "Speed up search across large pantries",
    "Support shared pantries between households",
    "Remove the legacy v1 sync endpoint",
    "Log slow queries in the cellar service",
    "Rind photos upload twice on flaky networks",
    "Add a health check to the crumb consumer",
    "Handle duplicate wheels from the importer",
    "Show expiry warnings on the home screen",
    "Make aging temperature units configurable",
    "Fix flaky integration test for pantry invites",
    "Batch push notifications for ripening alerts",
    "Store the last sync watermark per device",
    "Dark mode text is unreadable on the cellar map",
    "Add an audit log for shelf edits",
    "Upgrade the database driver",
    "Pantry invite links expire too early",
    "Deduplicate cheese varieties in the catalog",
    "Add OpenTelemetry traces to burrow",
    "Recipe import fails on fractional quantities",
    "Background sync drains the battery on Android",
    "Add feature flags for the tasting notes beta",
    "Validate humidity readings from sensors",
    "Index tasting notes for full-text search",
    "Wheel weight rounds to the wrong unit",
    "Split the auth module out of the monolith",
    "Move several shelves at once",
    "Offline edits are lost after a restart",
    "Expose aging stages in the public API",
    "Reduce cold start time of the mobile app",
    "Allow archiving an empty cellar",
    "Fix memory leak in the crumb producer",
    "Add a webhook for ripening milestones",
    "Sort the pantry list by expiry date",
    "Sensor pairing hangs on the second device",
    "Encrypt household notes at rest",
    "Add pagination cursors to the activity feed",
    "Clean up unused GraphQL resolvers",
    "Shared pantry shows stale counts after leaving",
    "Support metric and imperial wheel sizes",
    "Retry failed rind photo uploads in the background",
    "Add a dry-run mode to the importer",
    "Cellar map renders blank on tablets",
    "Bump the minimum supported iOS version",
    "Track aging time per shelf",
    "Add a read replica for reporting queries",
    "Notification settings reset after update",
    "Throttle sensor writes during bursts",
    "Migrate the tasting note schema",
    "Show who last moved a wheel",
    "Improve error messages on sync conflicts",
    "Add contract tests for the recipes API",
    "Burrow returns 500 on empty uploads",
    "Precompute nightly cellar reports",
    "Support multiple cellars per household",
    "Rotate refresh tokens on every use",
    "Add keyboard shortcuts to the web cellar view",
    "Expiry dates parse wrong in some locales",
    "Profile the slow pantry dashboard query",
    "Make the importer resumable",
    "Gouda batch labels print off-center",
];

const DES_TITLES: &[&str] = &[
    "Audit whisker colors for contrast",
    "Redesign the empty pantry screen",
    "Icon set for aging stages",
    "Onboarding flow for shared pantries",
    "Spacing tokens for whisker",
    "Cellar map visual refresh",
    "Illustrate the cheese wheel lifecycle",
    "Design the ripening alert notification",
    "Typography scale for tablets",
    "Usability test the shelf editor",
    "Dark mode palette for the cellar map",
    "Motion guidelines for list reordering",
    "Rework the settings information architecture",
    "Tasting notes card layout",
    "App store screenshots for the autumn release",
    "Accessible focus rings across whisker",
    "Error screens for sync conflicts",
    "Design the household invite sheet",
    "Recipe import review screen",
    "Empty activity feed illustration",
    "Expiry badge variants",
    "Interview five households about storage habits",
    "Prototype the bulk move gesture",
    "Refresh the Nibble wordmark",
    "Tablet layout for the cellar report",
    "Sensor pairing walkthrough",
    "Iconography for humidity levels",
    "Simplify the add-wheel form",
    "Marketing site hero illustration",
    "Data table pattern for whisker",
    "Review copy for permission prompts",
    "Chart styles for aging reports",
    "Toast and banner guidelines",
    "Design QA for the autumn release",
    "Explore a compact density mode",
    "Map legend for shelf temperatures",
    "Avatar and household badge styles",
    "Feedback widget placement",
    "Holiday cheese board campaign assets",
    "Color-blind safe workflow markers",
];

const OPS_TITLES: &[&str] = &[
    "Rotate the staging database credentials",
    "Set up an on-call rotation for the crumb pipeline",
    "Alert on cellar sensor gateway latency",
    "Document the incident review process",
    "Renew TLS certificates for the recipes API",
    "Move nightly backups to cold storage",
    "Load test the pantry sync service",
    "Patch the base images for October",
    "Review cloud spend against the budget",
    "Write runbooks for burrow outages",
    "Clean up orphaned volumes in staging",
    "Vendor review for the label printer fleet",
    "Public outage page for customers",
    "Restore drill for the cellar database",
    "Tag cloud resources by team",
    "Shorten deploy pipeline time",
    "Quarterly access review",
    "Upgrade the Kubernetes control plane",
    "Set log retention to 30 days",
    "Replace the warehouse label printers",
    "Pager noise from the humidity alerts",
    "Add disk usage alerts to the crumb brokers",
    "Onboarding checklist for new hires",
    "Inventory of third-party services",
    "Fix the broken nightly report email",
    "Archive the old marketing site",
    "Capacity plan for the holiday peak",
    "Set up a sandbox for the payments provider",
    "Order replacement temperature sensors",
    "Write the data retention policy",
    "Move DNS to infrastructure as code",
    "Track SLOs for pantry sync",
    "Review the disaster recovery plan",
    "Audit admin access to the cellar dashboard",
    "Reduce container image sizes",
    "Plan the office cheese cave move",
    "Clean up unused API keys",
    "Add synthetic checks for login",
    "Expense policy for conference travel",
    "Schedule the winter maintenance window",
];

/// Linear's suggested branch name: the lowercased identifier, then the title
/// as a slug, cut to a whole word within 60 characters.
fn branch_name(identifier: &str, title: &str) -> String {
    let mut name = identifier.to_lowercase();
    for word in title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if name.len() + 1 + word.len() > 60 {
            break;
        }
        name.push('-');
        name.push_str(&word.to_lowercase());
    }
    name
}
