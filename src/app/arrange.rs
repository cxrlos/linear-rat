//! How an issue list is grouped and sorted, and the pure function that
//! arranges a list's issues that way. Rendering and navigation both read
//! the groups it returns.

use std::cmp::Ordering;

use super::Names;
use crate::linear::TeamOrder;
use crate::linear::model::{Issue, Priority, StateType};

/// What an issue list groups its issues by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping {
    /// By state type: started, unstarted, backlog, triage, completed, then
    /// canceled.
    State,
    Priority,
    Project,
    Cycle,
    Assignee,
    /// One list with no group headers.
    None,
}

impl Grouping {
    /// Every grouping, in the order the picker lists them.
    pub const ALL: [Grouping; 6] = [
        Grouping::State,
        Grouping::Priority,
        Grouping::Project,
        Grouping::Cycle,
        Grouping::Assignee,
        Grouping::None,
    ];

    /// The name used in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Grouping::State => "state",
            Grouping::Priority => "priority",
            Grouping::Project => "project",
            Grouping::Cycle => "cycle",
            Grouping::Assignee => "assignee",
            Grouping::None => "none",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|grouping| grouping.name() == name)
    }

    /// What the picker and the list header call it.
    pub fn label(self) -> &'static str {
        match self {
            Grouping::State => "workflow state",
            Grouping::Priority => "priority",
            Grouping::Project => "project",
            Grouping::Cycle => "cycle",
            Grouping::Assignee => "assignee",
            Grouping::None => "none",
        }
    }
}

/// A field an issue list can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Updated,
    Created,
    Priority,
    DueDate,
    Estimate,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        SortKey::Updated,
        SortKey::Created,
        SortKey::Priority,
        SortKey::DueDate,
        SortKey::Estimate,
    ];

    /// The name used in `config.toml`, before any `-asc` or `-desc`.
    pub fn name(self) -> &'static str {
        match self {
            SortKey::Updated => "updated",
            SortKey::Created => "created",
            SortKey::Priority => "priority",
            SortKey::DueDate => "due-date",
            SortKey::Estimate => "estimate",
        }
    }

    /// The direction a bare name sorts in: the newest, the most urgent,
    /// the soonest due and the largest estimate first.
    pub fn natural_descending(self) -> bool {
        match self {
            SortKey::Updated | SortKey::Created | SortKey::Estimate => true,
            SortKey::Priority | SortKey::DueDate => false,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortKey::Updated => "updated",
            SortKey::Created => "created",
            SortKey::Priority => "priority",
            SortKey::DueDate => "due date",
            SortKey::Estimate => "estimate",
        }
    }
}

/// A sort: a field and its direction. Ascending puts urgent before low,
/// earlier dates before later ones and smaller estimates before larger.
/// Issues missing the field go last either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Sort {
    pub const fn new(key: SortKey, descending: bool) -> Self {
        Self { key, descending }
    }

    /// Every sort, each field in its natural direction first, in the order
    /// the picker lists them.
    pub fn all() -> impl Iterator<Item = Sort> {
        SortKey::ALL.into_iter().flat_map(|key| {
            let natural = key.natural_descending();
            [Sort::new(key, natural), Sort::new(key, !natural)]
        })
    }

    /// The name used in `config.toml`, as `priority-asc`.
    pub fn name(self) -> String {
        let direction = if self.descending { "desc" } else { "asc" };
        format!("{}-{direction}", self.key.name())
    }

    /// Reads a name from `config.toml`: a field alone, which sorts in its
    /// natural direction, or a field with `-asc` or `-desc`.
    pub fn from_name(name: &str) -> Option<Self> {
        SortKey::ALL.into_iter().find_map(|key| {
            let rest = name.strip_prefix(key.name())?;
            match rest {
                "" => Some(Sort::new(key, key.natural_descending())),
                "-asc" => Some(Sort::new(key, false)),
                "-desc" => Some(Sort::new(key, true)),
                _ => None,
            }
        })
    }

    /// What the picker and the list header call it, as `priority, urgent
    /// first`.
    pub fn label(self) -> String {
        let end = match (self.key, self.descending) {
            (SortKey::Updated | SortKey::Created, true) => "newest first",
            (SortKey::Updated | SortKey::Created, false) => "oldest first",
            (SortKey::Priority, false) => "urgent first",
            (SortKey::Priority, true) => "low first",
            (SortKey::DueDate, false) => "soonest first",
            (SortKey::DueDate, true) => "latest first",
            (SortKey::Estimate, true) => "largest first",
            (SortKey::Estimate, false) => "smallest first",
        };
        format!("{}, {end}", self.key.label())
    }

    /// How `a` and `b` compare, with a missing value after any present one.
    fn compare(self, a: &Issue, b: &Issue) -> Ordering {
        let directed = |ordering: Ordering| {
            if self.descending {
                ordering.reverse()
            } else {
                ordering
            }
        };
        match self.key {
            SortKey::Updated => directed(a.updated_at.cmp(&b.updated_at)),
            SortKey::Created => directed(a.created_at.cmp(&b.created_at)),
            SortKey::Priority => missing_last(
                (a.priority != Priority::None).then_some(a.priority),
                (b.priority != Priority::None).then_some(b.priority),
                |a, b| directed(a.cmp(&b)),
            ),
            SortKey::DueDate => missing_last(a.due_date, b.due_date, |a, b| directed(a.cmp(&b))),
            SortKey::Estimate => {
                missing_last(a.estimate, b.estimate, |a, b| directed(a.total_cmp(&b)))
            }
        }
    }
}

/// The order Linear can send pages of Team Issues in that `sort` matches,
/// if there is one: Linear orders only by updated or created, newest first.
pub fn linear_order(sort: Sort) -> Option<TeamOrder> {
    match (sort.key, sort.descending) {
        (SortKey::Updated, true) => Some(TeamOrder::Updated),
        (SortKey::Created, true) => Some(TeamOrder::Created),
        _ => None,
    }
}

fn missing_last<T>(a: Option<T>, b: Option<T>, compare: impl Fn(T, T) -> Ordering) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => compare(a, b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// How a list groups and sorts its issues. A `sort` of `None` keeps the
/// list's own order: the order Linear sent, or for My Issues active cycle
/// first, then priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrangement {
    pub group: Grouping,
    pub sort: Option<Sort>,
}

impl Arrangement {
    /// My Issues as it always was: by workflow state, active cycle first.
    pub const MY_ISSUES: Arrangement = Arrangement {
        group: Grouping::State,
        sort: None,
    };

    /// Team Issues as Linear sends them: one list, most recently updated
    /// first.
    pub const TEAM_ISSUES: Arrangement = Arrangement {
        group: Grouping::None,
        sort: Some(Sort::new(SortKey::Updated, true)),
    };

    /// Search results and the issues of a cycle or project: one list in
    /// the order Linear sent.
    pub const LINEAR_ORDER: Arrangement = Arrangement {
        group: Grouping::None,
        sort: None,
    };
}

/// What one group of an arranged list holds in common.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    State(StateType),
    Priority(Priority),
    /// A project id, `None` for issues in no project.
    Project(Option<String>),
    Cycle(Option<String>),
    /// A member id, `None` for unassigned issues.
    Assignee(Option<String>),
    /// The whole list, drawn with no header.
    All,
}

impl GroupKey {
    /// Whether the group is drawn under a header line.
    pub fn has_header(&self) -> bool {
        *self != GroupKey::All
    }
}

/// One group of an arranged list: what its issues share, and the issues in
/// order.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: GroupKey,
    pub issues: Vec<Issue>,
}

/// The state types in the order their groups appear.
const STATE_ORDER: [StateType; 6] = [
    StateType::Started,
    StateType::Unstarted,
    StateType::Backlog,
    StateType::Triage,
    StateType::Completed,
    StateType::Canceled,
];

/// `issues`, given in the list's own order, grouped and sorted as
/// `arrangement` says. Empty groups are left out. Groups of named things
/// come in name order (cycles newest first), with the group of issues that
/// have none last. Issues that tie on the sort keep their own order.
pub fn arrange(mut issues: Vec<Issue>, arrangement: Arrangement, names: &Names) -> Vec<Group> {
    if let Some(sort) = arrangement.sort {
        issues.sort_by(|a, b| sort.compare(a, b));
    }
    let key_of = |issue: &Issue| match arrangement.group {
        Grouping::State => GroupKey::State(issue.state.state_type),
        Grouping::Priority => GroupKey::Priority(issue.priority),
        Grouping::Project => GroupKey::Project(issue.project_id.clone()),
        Grouping::Cycle => GroupKey::Cycle(issue.cycle_id.clone()),
        Grouping::Assignee => GroupKey::Assignee(issue.assignee_id.clone()),
        Grouping::None => GroupKey::All,
    };
    let mut groups: Vec<Group> = Vec::new();
    for issue in issues {
        let key = key_of(&issue);
        match groups.iter_mut().find(|group| group.key == key) {
            Some(group) => group.issues.push(issue),
            None => groups.push(Group {
                key,
                issues: vec![issue],
            }),
        }
    }
    groups.sort_by(|a, b| group_order(&a.key, &b.key, names));
    groups
}

fn group_order(a: &GroupKey, b: &GroupKey, names: &Names) -> Ordering {
    let named = |a: &Option<String>, b: &Option<String>, name: &dyn Fn(&str) -> String| {
        missing_last(a.as_deref(), b.as_deref(), |a, b| {
            name(a).cmp(&name(b)).then_with(|| a.cmp(b))
        })
    };
    match (a, b) {
        (GroupKey::State(a), GroupKey::State(b)) => {
            let place = |state_type| STATE_ORDER.iter().position(|t| *t == state_type);
            place(*a).cmp(&place(*b))
        }
        (GroupKey::Priority(a), GroupKey::Priority(b)) => a.cmp(b),
        (GroupKey::Project(a), GroupKey::Project(b)) => named(a, b, &|id| {
            names.project_name(id).unwrap_or(id).to_lowercase()
        }),
        (GroupKey::Assignee(a), GroupKey::Assignee(b)) => named(a, b, &|id| {
            names.member_name(id).unwrap_or(id).to_lowercase()
        }),
        (GroupKey::Cycle(a), GroupKey::Cycle(b)) => {
            missing_last(a.as_deref(), b.as_deref(), |a, b| {
                names
                    .cycle_start(b)
                    .cmp(&names.cycle_start(a))
                    .then_with(|| a.cmp(b))
            })
        }
        _ => Ordering::Equal,
    }
}
