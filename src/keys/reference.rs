use std::fmt::Write;

use super::{ACTIONS, Context, Key};

/// The keymap reference, `docs/keymap.md`: every action of the registry with
/// its config id, default keys in config notation and description, one table
/// per context. An action bound in several contexts is listed in each.
pub fn keymap_reference() -> String {
    let mut out = String::from(
        "# Keymap\n\
         \n\
         <!-- Generated from the action registry in src/keys/registry.rs. \
         Do not edit by hand: run `UPDATE_KEYMAP=1 cargo test --test keymap_reference`. -->\n\
         \n\
         These are lrat's default keys. Press `?` in lrat to see the keys of the screen \
         you are on, with the ones that cannot run now muted. Keys are written in the \
         vim notation `config.toml` uses, and each table heading names the config table \
         that rebinds its actions; see \
         [Configuration](https://github.com/cxrlos/linear-rat/blob/main/docs/config.md#keys).\n\
         \n\
         The Team Issues list also takes the Issue list keys. While an overlay has a text \
         field, as the pickers, the palette and the search do, a character typed without \
         Ctrl or Alt goes into the field instead, except a picker's number keys.\n",
    );
    for context in Context::ALL {
        let _ = write!(
            out,
            "\n## {} (`[keys.{}]`)\n\n| Action | Keys | Description |\n| --- | --- | --- |\n",
            context.title(),
            context.name()
        );
        for spec in ACTIONS
            .iter()
            .filter(|spec| spec.contexts.contains(&context))
        {
            let keys: Vec<String> = spec
                .default_keys
                .iter()
                .map(|sequence| format!("`{}`", notation(sequence)))
                .collect();
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                spec.id,
                keys.join(", "),
                spec.description
            );
        }
    }
    out
}

/// A key sequence as `config.toml` writes it: plain characters as themselves,
/// anything else inside `<...>`.
fn notation(sequence: &[Key]) -> String {
    sequence
        .iter()
        .map(|key| match key.typed() {
            Some('<') => "<lt>".to_owned(),
            Some(' ') => "<Space>".to_owned(),
            Some(c) => c.to_string(),
            None => format!("<{key}>"),
        })
        .collect()
}
