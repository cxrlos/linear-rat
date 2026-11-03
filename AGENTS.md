# linear-rat

A terminal client for Linear, written in Rust with ratatui. One crate; the binary is `lrat`.

## Read first

- `GLOSSARY.md`: the words for what lrat shows and acts on. Use them in code, docs and issues; never use a term listed under _Avoid_.
- `docs/adr/`: decisions in force. When a change contradicts one, stop and name the ADR.
- `docs/agents/`: issue tracker, triage labels and domain-doc rules.
- `docs/releasing.md`: how versions, tags and publishing work.

## Status

Active, pre-release. Work toward `0.1.0` lands on the `feat/v0.1.0` branch, which is squash-merged into `main`.

## Critical rules

1. Never read, print, log or commit an API key, `.env` file or token. Tests never call the Linear API; responses come from fixtures.
2. The PR title is the commit: a Conventional Commit, with `!` for breaking changes. Squash merge only.
3. release-plz owns `CHANGELOG.md` and the version in `Cargo.toml`. Never edit them by hand.
4. Never accept a changed insta snapshot without showing the `.snap.new` diff first.
5. Commit or push only when the person driving the session asks. Never force-push a shared branch or skip hooks.
6. MSRV is 1.88. A dependency or language feature that raises it needs an ADR.
7. Linux and macOS only. Do not add Windows-specific code paths.

## Workflow

1. Orient: read the issue, `GLOSSARY.md` and the relevant ADRs. Branch `<type>/<issue>-<slug>` off `main` (during 0.1.0, off `feat/v0.1.0`).
2. Agree seams: name the module and public interface you will touch before writing code.
3. Red then green with `/tdd`. After each slice, run that test target and `cargo clippy`.
4. Verify: every command under Commands passes.
5. Review: `/code-review` against the merge-base.
6. Write back: PR body with Summary, Changes, Testing (exact commands and results), Decision needed (default built), After merge; `Fixes #N` or `Part of #N`.

Done when someone reading only the issue and the PR could pick up the next piece.

## Design principles

- Rendering is a pure function of app state; every screen has a `TestBackend` + insta snapshot.
- All Linear API access goes through one module, so tests swap it for fixtures.
- One action registry drives key handling, the keymap config, the `?` cheatsheet, the bottom-bar hints and the palette. Never list a key binding anywhere else.
- No `unwrap`/`expect` outside tests; errors carry context to the status line.
- The terminal is restored on every exit path, panics included.

## Map

- `src/main.rs`: the `lrat` binary. Parses the CLI, runs the `lrat auth` commands, or loads the config, starts logging, enters the terminal guard and runs the runtime.
- `src/app/`: the core. `State` (screens, modal stack, keymap, My Issues), `Event` in, `update(state, event)` returning an `Update` (redraw flag and `Effect`s), `perform(state, action)`, and `App`, the driver that applies events and redraws only when asked. Tests drive `App`.
- `src/keys/`: the action registry (`registry.rs`, the only place a key binding is written), key notation, contexts and the keymap built from the registry plus config overrides.
- `src/config.rs`: loads `config.toml` from the XDG config path (only `[keys]` so far), with errors naming file and line.
- `src/ui.rs`, `src/ui/`: rendering, a pure function of `State`: the layout, My Issues, the bottom bar, cheatsheet and which-key, glyphs (`icons.rs`) and colors (`style.rs`).
- `src/runtime.rs`: the async loop. Reads terminal input and the shutdown signal, feeds events to `App` and carries out effects.
- `src/linear/`: all Linear access. The `LinearClient` trait, the real `HttpClient`, `FakeClient`, the GraphQL documents (`graphql/`), the pinned `schema.graphql`, the domain types (`model.rs`) and the demo workspace (`demo.rs`).
- `src/auth/`: the redacting API key type, key resolution (`LINEAR_API_KEY`, then the OS keyring) and the workspace file that names the keyring entry.
- `src/analytics/`: the pure analytics calculator.
- `src/terminal.rs`: the terminal guard (raw mode, alternate screen, panic hook) and the SIGINT/SIGTERM/SIGHUP shutdown signal.
- `src/logging.rs`: `tracing` to a daily file under `$XDG_STATE_HOME/lrat/`, level from `LRAT_LOG`.
- `src/cli.rs`: clap flags and the `auth` subcommands.
- `tests/`: app-driver tests (`app.rs`, `keys.rs`, `runtime.rs`) and their insta snapshots, the demo data (`demo.rs`), auth and the CLI (`auth.rs`, `cli.rs`), the client against a fake server (`linear_client.rs`), GraphQL validation (`graphql_schema.rs`), analytics (`analytics.rs`) and shutdown signals (`signals.rs`).
- `examples/force_panic.rs`: panics inside the guard, to check the terminal is restored.

## Commands

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo insta pending-snapshots
cargo deny check
cargo +1.88 check --locked
cargo run -- --demo
```

CI runs all of these except `cargo run` on every PR.

## Agent skills

### Issue tracker

GitHub Issues on cxrlos/linear-rat, through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
