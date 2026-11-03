# Contributing

Thanks for helping. lrat is a terminal client for Linear; this page covers what you need to get a change merged.

## Scope

lrat supports Linux and macOS, and one Linear workspace at a time. Issues labelled `good first issue` are a good place to start.

## Setup

- Rust 1.88 or newer
- [`cargo-insta`](https://insta.rs) for snapshot tests
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) for dependency checks

`cargo run -- --demo` starts lrat with fake data, so you don't need a Linear account to work on the UI.

## Workflow

1. Open an issue, or comment on an existing one, before starting larger work.
2. Branch off `main` as `<type>/<issue>-<slug>`, for example `feat/42-board-view`.
3. Before opening a PR, run the checks listed under Commands in [AGENTS.md](AGENTS.md). CI runs the same ones.
4. When a snapshot changes, review it with `cargo insta review` and commit the accepted `.snap` files.

## Pull requests

- The PR title becomes the commit on `main`, so it must be a [Conventional Commit](https://www.conventionalcommits.org): `feat: ...`, `fix: ...`, `feat!: ...` for breaking changes. The changelog and version bump are generated from it.
- PRs are squash-merged.
- Fill in the template, including the exact commands you ran.

## Words and decisions

[GLOSSARY.md](GLOSSARY.md) defines the terms lrat uses; please use them in code and issues. Recorded decisions live in [docs/adr](docs/adr).

## AI-assisted contributions

Welcome. Agents read [AGENTS.md](AGENTS.md). You are responsible for what you submit, and you should be able to explain every change.

## Security

Never paste an API key into an issue, PR or log. Report vulnerabilities as described in [SECURITY.md](SECURITY.md).

## License

Unless you say otherwise, any contribution you submit is dual-licensed as MIT OR Apache-2.0, like the rest of the project, with no additional terms.
