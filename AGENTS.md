# linear-rat

A terminal client for Linear, written in Rust with ratatui. One crate; the binary is `lrat`.

## Read first

- `GLOSSARY.md`: the words for what lrat shows and acts on. Use them in code, docs and issues; never use a term listed under _Avoid_.
- `docs/adr/`: decisions in force. When a change contradicts one, stop and name the ADR.
- `docs/agents/`: issue tracker, triage labels and domain-doc rules.
- `docs/releasing.md`: how versions, tags and publishing work.

## Status

Active, pre-release. Working toward `0.1.0` on the `feat/v0.1.0` branch, which is squash-merged into `main` as one commit.

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

Filled in once the crate exists.

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

CI runs the same set on every PR.
