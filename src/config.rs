//! The config file, `~/.config/lrat/config.toml` (the XDG config path, on
//! macOS too). Only the `[keys]` section is read so far.
//!
//! `[keys]` holds one table per context (`global`, `list`, `detail`,
//! `overlay`). In each, an action's id maps to its keys in vim notation:
//!
//! ```toml
//! [keys.global]
//! quit = "Q"                             # replace: Q is now quit's only key
//! show-cheatsheet = { append = ["<F1>"] }
//!
//! [keys.list]
//! move-down = { prepend = ["n"] }        # n first, j still works
//! noop = ["gg", "<C-d>"]                 # these keys do nothing, and leave every hint
//! ```
//!
//! A problem in one context's table is reported with its file and line, and
//! that context keeps its default keys; the other tables still apply.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use etcetera::BaseStrategy;
use etcetera::base_strategy::Xdg;
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::keys::{self, Change, Context, KeySequence, Override, Target};

/// Everything read from the config file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Config {
    /// The viewer's changes to the default keymap, in file order.
    pub keys: Vec<Override>,
}

/// A config and the problems found while reading it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub errors: Vec<ConfigError>,
}

/// A problem in the config file, located by line where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = self.path.display();
        match self.line {
            Some(line) => write!(f, "{path}:{line}: {}", self.message),
            None => write!(f, "{path}: {}", self.message),
        }
    }
}

/// `$XDG_CONFIG_HOME/lrat/config.toml`, by default `~/.config/lrat/config.toml`.
pub fn path() -> anyhow::Result<PathBuf> {
    let xdg = Xdg::new().context("finding the home directory")?;
    Ok(xdg.config_dir().join("lrat").join("config.toml"))
}

/// Reads the config at `path`. A missing file is the default config.
pub fn load(path: &Path) -> Loaded {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Loaded::default(),
        Err(error) => Loaded {
            config: Config::default(),
            errors: vec![ConfigError {
                path: path.to_owned(),
                line: None,
                message: format!("could not read it ({error}); using the default config"),
            }],
        },
    }
}

/// Reads config `text` as if it came from `path`, which errors name.
pub fn parse(text: &str, path: &Path) -> Loaded {
    let mut reader = Reader {
        text,
        path,
        errors: Vec::new(),
    };
    let keys = match DeTable::parse(text) {
        Ok(document) => match document.get_ref().get("keys") {
            Some(keys) => reader.keys(keys),
            None => Vec::new(),
        },
        Err(error) => {
            reader.error(
                error.span().map(|span| span.start),
                format!("{}; using the default config", error.message().trim_end()),
            );
            Vec::new()
        }
    };
    let mut errors = reader.errors;
    errors.sort_by_key(|error| error.line);
    Loaded {
        config: Config { keys },
        errors,
    }
}

struct Reader<'a> {
    text: &'a str,
    path: &'a Path,
    errors: Vec<ConfigError>,
}

/// An error already recorded in [`Reader::errors`].
struct Reported;

impl Reader<'_> {
    fn keys(&mut self, keys: &Spanned<DeValue>) -> Vec<Override> {
        let DeValue::Table(contexts) = keys.get_ref() else {
            self.error_at(
                keys,
                "`keys` must be a table, as in `[keys.list]`".to_owned(),
            );
            return Vec::new();
        };
        let mut overrides = Vec::new();
        for (name, table) in contexts {
            let Some(context) = Context::from_name(name.get_ref()) else {
                self.error_at(
                    name,
                    format!(
                        "unknown context `[keys.{name}]`; expected global, list, detail or overlay"
                    ),
                );
                continue;
            };
            let before = self.errors.len();
            let found = self.context(context, table);
            if self.errors.len() == before {
                overrides.extend(found);
            } else {
                for error in &mut self.errors[before..] {
                    error.message.push_str(&format!(
                        "; [keys.{}] uses the default keys",
                        context.name()
                    ));
                }
            }
        }
        overrides
    }

    fn context(&mut self, context: Context, table: &Spanned<DeValue>) -> Vec<Override> {
        let DeValue::Table(entries) = table.get_ref() else {
            self.error_at(table, format!("`keys.{}` must be a table", context.name()));
            return Vec::new();
        };
        let mut overrides = Vec::new();
        for (id, value) in entries {
            let id_str: &str = id.get_ref();
            let target = if id_str == "noop" {
                Target::Noop
            } else {
                match keys::by_id(id_str) {
                    Some(spec) if spec.contexts.contains(&context) => Target::Action(spec.action),
                    Some(_) => {
                        self.error_at(
                            id,
                            format!("`{id_str}` does not apply in [keys.{}]", context.name()),
                        );
                        continue;
                    }
                    None => {
                        self.error_at(id, format!("unknown action `{id_str}`"));
                        continue;
                    }
                }
            };
            let mut add = |change, keys| {
                overrides.push(Override {
                    context,
                    target,
                    change,
                    keys,
                });
            };
            match value.get_ref() {
                DeValue::Table(changes) if target != Target::Noop => {
                    for (change, keys) in changes {
                        let change = match change.get_ref().as_ref() {
                            "prepend" => Change::Prepend,
                            "append" => Change::Append,
                            other => {
                                self.error_at(
                                    change,
                                    format!("`{other}` is not `prepend` or `append`"),
                                );
                                continue;
                            }
                        };
                        if let Ok(keys) = self.sequences(keys) {
                            add(change, keys);
                        }
                    }
                }
                _ => {
                    if let Ok(keys) = self.sequences(value) {
                        add(Change::Replace, keys);
                    }
                }
            }
        }
        overrides
    }

    fn sequences(&mut self, value: &Spanned<DeValue>) -> Result<Vec<KeySequence>, Reported> {
        match value.get_ref() {
            DeValue::String(_) => Ok(vec![self.sequence(value)?]),
            DeValue::Array(items) => items.iter().map(|item| self.sequence(item)).collect(),
            _ => Err(self.error_at(value, "expected a key or a list of keys".to_owned())),
        }
    }

    fn sequence(&mut self, value: &Spanned<DeValue>) -> Result<KeySequence, Reported> {
        let Some(notation) = value.get_ref().as_str() else {
            return Err(self.error_at(
                value,
                "expected a key such as \"j\" or \"<C-d>\"".to_owned(),
            ));
        };
        notation
            .parse()
            .map_err(|error| self.error_at(value, format!("{error}")))
    }

    fn error_at<T>(&mut self, spanned: &Spanned<T>, message: String) -> Reported {
        self.error(Some(spanned.span().start), message)
    }

    fn error(&mut self, offset: Option<usize>, message: String) -> Reported {
        let line = offset.map(|offset| {
            let before = self.text.get(..offset).unwrap_or(self.text);
            before.matches('\n').count() + 1
        });
        self.errors.push(ConfigError {
            path: self.path.to_owned(),
            line,
            message,
        });
        Reported
    }
}
