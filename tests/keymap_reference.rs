use std::path::Path;

use linear_rat::keys::{self, ACTIONS, KeySequence};

const REFERENCE: &str = "docs/keymap.md";

#[test]
fn reference_lists_each_action_under_each_of_its_contexts() {
    let markdown = keys::keymap_reference();
    let list = section(&markdown, "## Issue list");
    assert!(list.contains("| `half-page-down` | `<C-d>` | Move down half a page |"));
    assert!(list.contains("| `move-to-top` | `gg` | Move to the top |"));
    assert!(list.contains("| `open` | `<Enter>` | Open the focused issue |"));
    let detail = section(&markdown, "## Issue detail");
    assert!(detail.contains("| `copy-url` | `yu` | Copy the URL |"));
    assert!(!detail.contains("`open`"));
    let global = section(&markdown, "## Everywhere");
    assert!(global.contains("| `show-palette` | `:`, `<C-k>` | Find an action or loaded issue |"));
    let overlay = section(&markdown, "## Overlays");
    assert!(overlay.contains("| `scroll-down` | `j`, `<Down>`, `<C-n>` | Scroll down |"));
}

#[test]
fn every_key_in_the_reference_reads_back_as_config_notation() {
    let markdown = keys::keymap_reference();
    for spec in ACTIONS {
        let row = markdown
            .lines()
            .find(|line| line.starts_with(&format!("| `{}` |", spec.id)))
            .unwrap_or_else(|| panic!("{} is missing from the reference", spec.id));
        let cell = row.split(" | ").nth(1).unwrap();
        let written: Vec<KeySequence> = cell
            .split(", ")
            .map(|key| key.trim_matches('`').parse().unwrap())
            .collect();
        let defaults: Vec<KeySequence> = spec
            .default_keys
            .iter()
            .map(|keys| KeySequence::from(*keys))
            .collect();
        assert_eq!(written, defaults, "{}", spec.id);
    }
}

/// Fails when the registry changed and `docs/keymap.md` was not regenerated.
/// `UPDATE_KEYMAP=1 cargo test --test keymap_reference` rewrites it.
#[test]
fn docs_keymap_matches_the_registry() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(REFERENCE);
    let expected = keys::keymap_reference();
    if std::env::var_os("UPDATE_KEYMAP").is_some() {
        std::fs::write(&path, &expected).unwrap();
        return;
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{REFERENCE} is out of date with the action registry; regenerate it with \
         `UPDATE_KEYMAP=1 cargo test --test keymap_reference` and commit the result"
    );
}

/// The text from `heading` to the next heading of the same level.
fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
    let start = markdown
        .find(heading)
        .unwrap_or_else(|| panic!("no `{heading}` in the reference"));
    let rest = &markdown[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    &rest[..end]
}
