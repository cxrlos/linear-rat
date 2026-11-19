//! Properties of the keymap under any overrides: in any stack of contexts,
//! a key the hints show for an action runs that action (so no key shows
//! for two), and a key bound to `noop` never shows at all.

use crossterm::event::KeyCode;
use linear_rat::keys::{
    ACTIONS, Change, Context, Key, KeySequence, Keymap, Override, Resolution, Target,
};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

/// A few keys, most of them bound by default somewhere, so overrides
/// collide with the defaults and with each other.
fn key() -> impl Strategy<Value = Key> {
    prop::sample::select(vec![
        Key::char('j'),
        Key::char('k'),
        Key::char('g'),
        Key::char('y'),
        Key::char('u'),
        Key::char('f'),
        Key::char('s'),
        Key::char('1'),
        Key::char('x'),
        Key::ctrl('d'),
        Key::ctrl('y'),
        Key::named(KeyCode::Enter),
        Key::named(KeyCode::Esc),
        Key::named(KeyCode::Tab),
    ])
}

/// One key or a chord of two.
fn sequence() -> impl Strategy<Value = KeySequence> {
    prop::collection::vec(key(), 1..=2).prop_map(|keys| KeySequence::from(keys.as_slice()))
}

fn context() -> impl Strategy<Value = Context> {
    prop::sample::select(Context::ALL.to_vec())
}

fn target() -> impl Strategy<Value = Target> {
    prop_oneof![
        1 => Just(Target::Noop),
        4 => prop::sample::select(ACTIONS.iter().map(|spec| spec.action).collect::<Vec<_>>())
            .prop_map(Target::Action),
    ]
}

fn change() -> impl Strategy<Value = Change> {
    prop::sample::select(vec![Change::Replace, Change::Prepend, Change::Append])
}

fn overrides() -> impl Strategy<Value = Vec<Override>> {
    let one = (
        context(),
        target(),
        change(),
        prop::collection::vec(sequence(), 0..=3),
    )
        .prop_map(|(context, target, change, keys)| Override {
            context,
            target,
            change,
            keys,
        });
    prop::collection::vec(one, 0..12)
}

/// A stack of contexts the app builds, most specific first: each screen
/// and pane, and an overlay.
fn contexts() -> impl Strategy<Value = Vec<Context>> {
    use Context::{Analytics, Detail, Global, List, Overlay, TeamIssues};
    prop::sample::select(vec![
        vec![Global],
        vec![List, Global],
        vec![TeamIssues, List, Global],
        vec![Detail, Global],
        vec![Analytics, Global],
        vec![Overlay],
    ])
}

/// Every key sequence the hints would show for some action in `contexts`.
fn shown(keymap: &Keymap, contexts: &[Context]) -> Vec<(KeySequence, Target)> {
    ACTIONS
        .iter()
        .flat_map(|spec| {
            keymap
                .keys_for(spec.action, contexts)
                .into_iter()
                .map(move |keys| (keys, Target::Action(spec.action)))
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/keymap_properties.txt",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn a_shown_key_runs_its_action_so_no_key_shows_for_two(
        overrides in overrides(),
        contexts in contexts(),
    ) {
        let keymap = Keymap::new(&overrides);

        let shown = shown(&keymap, &contexts);

        for (keys, target) in &shown {
            let Target::Action(action) = *target else { unreachable!() };
            // Pressed one at a time, each key before the last only waits
            // for the rest of the chord.
            for len in 1..keys.keys().len() {
                prop_assert_eq!(
                    keymap.resolve(&contexts, &keys.keys()[..len]),
                    Resolution::Prefix,
                    "{} is shown for {:?} but its start runs something else",
                    keys,
                    action
                );
            }
            prop_assert_eq!(
                keymap.resolve(&contexts, keys.keys()),
                Resolution::Action(action),
                "{} is shown for {:?}",
                keys,
                action
            );
        }
        for (n, (keys, target)) in shown.iter().enumerate() {
            let twice = shown[n + 1..].iter().find(|(other, _)| other == keys);
            prop_assert!(
                twice.is_none(),
                "{} is shown for {:?} and {:?}",
                keys,
                target,
                twice.map(|(_, other)| other)
            );
        }
    }

    #[test]
    fn a_noop_key_is_never_shown_in_its_context(
        overrides in overrides(),
        context in context(),
    ) {
        let keymap = Keymap::new(&overrides);
        let noops: Vec<&KeySequence> = overrides
            .iter()
            .filter(|o| o.context == context && o.target == Target::Noop)
            .flat_map(|o| &o.keys)
            .collect();

        for (keys, target) in shown(&keymap, &[context]) {
            for noop in &noops {
                prop_assert!(
                    !keys.keys().starts_with(noop.keys()),
                    "{} is bound to noop in {:?} but {} is shown for {:?}",
                    noop,
                    context,
                    keys,
                    target
                );
            }
        }
        for noop in noops {
            prop_assert_ne!(
                keymap.resolve(&[context], noop.keys()),
                Resolution::Prefix,
                "{} is bound to noop in {:?} but still starts a chord",
                noop,
                context
            );
        }
    }
}
