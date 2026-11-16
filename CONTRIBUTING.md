# Contributing

Thanks for helping. lrat is a terminal client for Linear; this page covers what you need to get a change merged.

## Scope

lrat supports Linux and macOS, and one Linear workspace at a time. Issues labelled `good first issue` are a good place to start.

## Setup

- Rust 1.88 or newer
- [`cargo-insta`](https://insta.rs) for snapshot tests
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) for dependency checks

`cargo run -- --demo` starts lrat with fake data, so you don't need a Linear account to work on the UI. When your change touches how lrat talks to Linear, also try it against a throwaway workspace, as described in [Testing against a free Linear workspace](#testing-against-a-free-linear-workspace).

## Testing against a free Linear workspace

The test suite never calls the Linear API, so a change to the Linear client also needs a run against the real thing. I do that in a free workspace I made for it, never in an employer's workspace: lrat can change issues, and a key for a work workspace reaches real data.

1. Create a workspace. Sign up at [linear.app](https://linear.app), or open the workspace menu if you already have an account, and create a new workspace on the Free plan. On the Free plan every member is an admin, so you can make your own key.
2. Create a personal API key in that workspace, on its account security settings page ([linear.app/settings/account/security](https://linear.app/settings/account/security)). Give it a name you will recognise, such as `lrat-dev`.
3. Hand the key to lrat in one of two ways:
   - For one shell session only, keep it in `LINEAR_API_KEY`. `read -rs` keeps it out of your shell history and off the screen; paste the key and press Enter:

     ```sh
     read -rs LINEAR_API_KEY && export LINEAR_API_KEY
     ```

     Closing the shell forgets it. While it is set it wins over any key in the keyring.
   - Or store it in the OS keyring with `lrat auth login`, which prompts for it without echo. `lrat auth logout` removes it again.
4. Load a handful of sample issues. Five to ten is enough: create them by hand in the web app, in a couple of workflow states, some assigned to you and some with labels. If you have a CSV export from another tool, Linear's [importer](https://linear.app/docs/import-issues) can load it instead.

I run lrat from my working tree with `cargo run --`, so `lrat auth status` below is `cargo run -- auth status`, and `lrat` is `cargo run`.

### Manual checks

Run these in order and compare with what each step expects:

1. Log in. Unset `LINEAR_API_KEY` first (`unset LINEAR_API_KEY`), run `lrat auth login` and paste the key. It reports the workspace and viewer it stored the key for.
2. Check the key. `lrat auth status` names where the key comes from (the keyring), the workspace and the viewer.
3. Load My Issues. `lrat` opens on My Issues, grouped by workflow state, and shows the sample issues assigned to you.
4. Make an edit round trip. On an issue in My Issues press `s`, pick another open workflow state (such as In Progress, so the issue stays in My Issues) and press Enter. The new state shows in lrat and in the web app. Press `s` again and put the issue back in its original state; both show the original state again.
5. Watch live updates. Add `live = true` under `[sync]` in `~/.config/lrat/config.toml` (see [Configuration](docs/config.md#sync)) and start `lrat`. In the web app, change the title or workflow state of an issue assigned to you. lrat shows the change within a second or two, sooner than the next poll (15 seconds by default) and without pressing `r`. Remove the line again afterwards if you don't want it.
6. Log out. `lrat auth logout` says it removed the key from the OS keyring, and `lrat auth status` then fails with `no API key`.

When you are done, revoke the key in the same settings page, or delete the whole workspace.

Keys never go in issues, PRs, logs, screenshots or recordings. Check a terminal screenshot before you post it, and if a key leaks anyway, revoke it at once.

## Workflow

1. Open an issue, or comment on an existing one, before starting larger work. Questions and loose ideas go to [Discussions](https://github.com/cxrlos/linear-rat/discussions).
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
