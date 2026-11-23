//! Checks every GraphQL document under `src/linear/graphql/` against the pinned
//! copy of Linear's schema in `src/linear/schema.graphql`.
//!
//! The pinned schema is `packages/sdk/src/schema.graphql` from
//! <https://github.com/linear/linear> at commit
//! 253713fef2916ab744defdd71c62acccc8e929a2 (2026-11-02). It moves only in a
//! dedicated PR.
//!
//! Each document declares its complexity budget on a `# budget: N` line. The
//! estimate is a simplified version of Linear's own scoring: every field that
//! selects an object (a connection, its `nodes`, any nested object) costs one
//! point times the product of the `first:` values of the connections around
//! it. Scalars are free. A search payload such as `IssueSearchPayload` pages
//! like a connection and is counted as one. A connection without `first:` (a
//! literal, or a variable with a default) is rejected, because Linear would
//! charge it at its default page size of 50 and the document would hide that
//! cost.
//!
//! `LRAT_SCHEMA_PATH` points the document check at another schema file, which
//! the weekly drift workflow uses to try Linear's latest schema:
//!
//! ```sh
//! LRAT_SCHEMA_PATH=/path/to/schema.graphql cargo test --locked --test graphql_schema every_document
//! ```

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use apollo_compiler::executable::{Field, Selection, SelectionSet};
use apollo_compiler::schema::ExtendedType;
use apollo_compiler::validation::Valid;
use apollo_compiler::{ExecutableDocument, Name, Node, Schema};

/// Linear rejects any single query above this many complexity points.
const LINEAR_MAX_COMPLEXITY: u64 = 10_000;

/// Points the document check at another copy of the schema, such as Linear's
/// latest. The rule tests below always use the pinned copy.
const SCHEMA_OVERRIDE: &str = "LRAT_SCHEMA_PATH";

fn load_schema(path: &Path) -> Valid<Schema> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    Schema::parse_and_validate(text, path).unwrap_or_else(|invalid| {
        panic!(
            "{} is not a valid schema:\n{}",
            path.display(),
            invalid.errors
        )
    })
}

fn pinned_schema() -> &'static Valid<Schema> {
    static SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        load_schema(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/linear/schema.graphql"))
    })
}

/// Validates `text` against the pinned schema and returns its estimated
/// complexity, or every problem found.
fn check(text: &str, name: &str) -> Result<u64, Vec<String>> {
    check_with(pinned_schema(), text, name)
}

fn check_with(schema: &Valid<Schema>, text: &str, name: &str) -> Result<u64, Vec<String>> {
    let document = ExecutableDocument::parse_and_validate(schema, text, name)
        .map_err(|invalid| vec![invalid.errors.to_string()])?;
    let mut problems = Vec::new();
    let mut cost = 0;
    for operation in document.operations.iter() {
        let mut estimator = Estimator {
            document: &document,
            variables: &operation.variables,
            problems: &mut problems,
        };
        cost += estimator.selection_set(&operation.selection_set, 1);
    }
    if problems.is_empty() {
        Ok(cost)
    } else {
        Err(problems)
    }
}

struct Estimator<'a> {
    document: &'a ExecutableDocument,
    variables: &'a [Node<apollo_compiler::ast::VariableDefinition>],
    problems: &'a mut Vec<String>,
}

impl Estimator<'_> {
    fn selection_set(&mut self, selection_set: &SelectionSet, multiplier: u64) -> u64 {
        let mut cost = 0;
        for selection in &selection_set.selections {
            cost += match selection {
                Selection::Field(field) => self.field(field, multiplier),
                Selection::FragmentSpread(spread) => {
                    match self.document.fragments.get(&spread.fragment_name) {
                        Some(fragment) => self.selection_set(&fragment.selection_set, multiplier),
                        None => 0,
                    }
                }
                Selection::InlineFragment(inline) => {
                    self.selection_set(&inline.selection_set, multiplier)
                }
            };
        }
        cost
    }

    fn field(&mut self, field: &Field, multiplier: u64) -> u64 {
        let definition = &field.definition;
        if is_internal(definition.description.as_deref()) {
            self.problems
                .push(format!("`{}` is [Internal]", field.name));
        }
        if definition.directives.get("deprecated").is_some() {
            self.problems
                .push(format!("`{}` is deprecated", field.name));
        }
        for argument in &field.arguments {
            let Some(argument_definition) = definition
                .arguments
                .iter()
                .find(|candidate| candidate.name == argument.name)
            else {
                continue;
            };
            if is_internal(argument_definition.description.as_deref()) {
                self.problems.push(format!(
                    "argument `{}` of `{}` is [Internal]",
                    argument.name, field.name
                ));
            }
            if argument_definition.directives.get("deprecated").is_some() {
                self.problems.push(format!(
                    "argument `{}` of `{}` is deprecated",
                    argument.name, field.name
                ));
            }
        }

        if field.selection_set.selections.is_empty() {
            return 0;
        }
        let type_name = field.ty().inner_named_type();
        let inner = if type_name.ends_with("Connection") || type_name.ends_with("SearchPayload") {
            match self.first(field) {
                Some(first) => multiplier * first,
                None => {
                    self.problems.push(format!(
                        "connection `{}` has no `first:` to bound its cost",
                        field.name
                    ));
                    multiplier
                }
            }
        } else {
            multiplier
        };
        multiplier + self.selection_set(&field.selection_set, inner)
    }

    fn first(&self, field: &Field) -> Option<u64> {
        let value = field
            .arguments
            .iter()
            .find(|argument| argument.name == "first")?
            .value
            .clone();
        let value = match value.as_ref() {
            apollo_compiler::ast::Value::Variable(name) => self.default_of(name)?,
            _ => value,
        };
        match value.as_ref() {
            apollo_compiler::ast::Value::Int(int) => int.try_to_i32().ok()?.try_into().ok(),
            _ => None,
        }
    }

    fn default_of(&self, variable: &Name) -> Option<Node<apollo_compiler::ast::Value>> {
        self.variables
            .iter()
            .find(|definition| definition.name == *variable)?
            .default_value
            .clone()
    }
}

fn is_internal(description: Option<&str>) -> bool {
    description.is_some_and(|text| text.contains("[Internal]"))
}

fn budget(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("# budget:"))
        .and_then(|budget| budget.trim().parse().ok())
}

/// Checks every document under `src/linear/graphql/` against `schema`.
/// Fails with a report that names each failing document on a `name.graphql:`
/// line followed by its problems.
fn check_documents(schema: &Valid<Schema>) -> Result<(), String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/linear/graphql");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "graphql")
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no documents found in {}", dir.display());

    let mut failing = Vec::new();
    let mut report = String::new();
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(path).unwrap();
        let problems = document_problems(schema, &text, &name);
        if !problems.is_empty() {
            report.push_str(&format!("\n{name}:\n  {}\n", problems.join("\n  ")));
            failing.push(name);
        }
    }
    if failing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failing documents: {}\n{report}",
            failing.join(", ")
        ))
    }
}

fn document_problems(schema: &Valid<Schema>, text: &str, name: &str) -> Vec<String> {
    let Some(budget) = budget(text) else {
        return vec!["needs a `# budget: N` line".to_owned()];
    };
    if budget > LINEAR_MAX_COMPLEXITY {
        return vec![format!("budget {budget} is above Linear's limit")];
    }
    match check_with(schema, text, name) {
        Ok(cost) if cost > budget => vec![format!("estimated {cost} points, budget {budget}")],
        Ok(_) => Vec::new(),
        Err(problems) => problems,
    }
}

#[test]
fn every_document_is_valid_and_within_its_budget() {
    let result = match std::env::var_os(SCHEMA_OVERRIDE).filter(|path| !path.is_empty()) {
        Some(path) => check_documents(&load_schema(Path::new(&path))),
        None => check_documents(pinned_schema()),
    };

    if let Err(report) = result {
        panic!("{report}");
    }
}

#[test]
fn a_field_unknown_to_the_schema_is_rejected() {
    let problems = check("query { viewer { favouriteCheese } }", "unknown").unwrap_err();

    assert!(problems[0].contains("favouriteCheese"), "{problems:?}");
}

#[test]
fn a_deprecated_field_is_rejected() {
    let problems = check("query { viewer { inviteHash } }", "deprecated").unwrap_err();

    assert_eq!(problems, ["`inviteHash` is deprecated"]);
}

#[test]
fn an_internal_field_is_rejected() {
    let problems = check("query { archivedTeams { id } }", "internal").unwrap_err();

    assert_eq!(problems, ["`archivedTeams` is [Internal]"]);
}

#[test]
fn a_connection_without_first_is_rejected() {
    let problems = check("query { issues { nodes { id } } }", "unbounded").unwrap_err();

    assert_eq!(
        problems,
        ["connection `issues` has no `first:` to bound its cost"]
    );
}

#[test]
fn nested_connections_multiply_the_cost() {
    // teams 1 + nodes 10 + issues 10 + nodes 10 * 5.
    let text = "query { teams(first: 10) { nodes { id issues(first: 5) { nodes { id } } } } }";

    assert_eq!(check(text, "nested"), Ok(71));
}

#[test]
fn first_may_come_from_a_variable_default() {
    let text = "query Page($first: Int = 20) { issues(first: $first) { nodes { id } } }";

    assert_eq!(check(text, "variable"), Ok(21));
}

#[test]
fn a_schema_without_a_field_a_document_uses_names_that_document() {
    let mut schema = pinned_schema().clone().into_inner();
    let Some(ExtendedType::Object(issue)) = schema.types.get_mut("Issue") else {
        panic!("Issue is not an object type");
    };
    issue.make_mut().fields.shift_remove("identifier");
    let schema = schema.validate().unwrap();

    let failures = check_documents(&schema).unwrap_err();

    assert!(failures.contains("assigned_issues.graphql:"), "{failures}");
    assert!(!failures.contains("viewer.graphql:"), "{failures}");
}

/// Every query that reads a whole issue (the ones asking for `branchName`)
/// asks for the fields the store keeps, as often as it asks for an issue. A
/// query that left one out would overwrite the stored value with nothing.
#[test]
fn every_whole_issue_projection_asks_for_every_stored_field() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/linear/graphql");
    let mut missing = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_none_or(|extension| extension != "graphql")
        {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let issues = text.matches("branchName").count();
        if issues == 0 {
            continue;
        }
        for field in [
            "archivedAt",
            "subscribers(",
            "dueDate",
            "projectMilestone {",
        ] {
            let found = text.matches(field).count();
            if found < issues {
                missing.push(format!(
                    "{}: {field} {found} of {issues} times",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}

/// Every document that reads a comment (the ones asking for a comment's
/// `body` beside `createdAt`) asks for the fields the store keeps on it, so
/// a reply, a resolve or a reaction never vanishes when a comment is read
/// again.
#[test]
fn every_comment_projection_asks_for_every_stored_field() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/linear/graphql");
    let mut missing = Vec::new();
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("comment") && name != "issue_comments.graphql" {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        if !text.contains("body") {
            continue;
        }
        checked += 1;
        for field in ["parentId", "resolvedAt", "editedAt", "reactions {"] {
            if !text.contains(field) {
                missing.push(format!("{name}: {field}"));
            }
        }
    }
    assert!(checked >= 3, "only {checked} comment documents found");
    assert!(missing.is_empty(), "{missing:#?}");
}
