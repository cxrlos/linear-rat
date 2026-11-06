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

// clippy's allow-unwrap-in-tests does not cover helpers outside #[test] functions.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use apollo_compiler::executable::{Field, Selection, SelectionSet};
use apollo_compiler::validation::Valid;
use apollo_compiler::{ExecutableDocument, Name, Node, Schema};

/// Linear rejects any single query above this many complexity points.
const LINEAR_MAX_COMPLEXITY: u64 = 10_000;

fn schema() -> &'static Valid<Schema> {
    static SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/linear/schema.graphql");
        let text = fs::read_to_string(&path).unwrap();
        Schema::parse_and_validate(text, &path).unwrap()
    })
}

/// Validates `text` against the schema and returns its estimated complexity,
/// or every problem found.
fn check(text: &str, name: &str) -> Result<u64, Vec<String>> {
    let document = ExecutableDocument::parse_and_validate(schema(), text, name)
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

#[test]
fn every_document_is_valid_and_within_its_budget() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/linear/graphql");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_none_or(|extension| extension != "graphql")
        {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let budget = budget(&text).unwrap_or_else(|| panic!("{name} needs a `# budget: N` line"));
        assert!(
            budget <= LINEAR_MAX_COMPLEXITY,
            "{name}: budget {budget} is above Linear's limit"
        );

        let cost = check(&text, &name).unwrap_or_else(|problems| {
            panic!("{name}:\n{}", problems.join("\n"));
        });

        assert!(
            cost <= budget,
            "{name}: estimated {cost} points, budget {budget}"
        );
        checked += 1;
    }
    assert!(checked > 0, "no documents found in {}", dir.display());
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
