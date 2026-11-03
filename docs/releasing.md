# Releasing

How I ship linear-rat, and the one-time setup that only I can do. Versions follow SemVer from `0.1.0`; while on `0.x`, a breaking change bumps the minor version and a `feat` bumps the patch version.

## Pipeline

1. Every change lands on `main` as a squash-merged PR whose title is a Conventional Commit.
2. On each push to `main`, [release-plz](https://release-plz.dev) opens or updates a release PR: it bumps `Cargo.toml`, and writes `CHANGELOG.md` from the commits (git-cliff format).
3. Merging the release PR makes release-plz tag `vX.Y.Z` and publish to crates.io through trusted publishing (no stored token). It does not create the GitHub release (`git_release_enable = false`).
4. The `vX.Y.Z` tag triggers [dist](https://github.com/axodotdev/cargo-dist), which builds the binaries, creates the GitHub release with every asset attached, and updates the Homebrew formula.

Releases are immutable in this repo: once published, a release's tag and assets cannot change. That is why exactly one tool, dist, creates the release, and only after all assets are built. If release-plz created it first, dist's asset upload would be refused. (UNVERIFIED against current release-plz and dist docs; confirm in the release workflow ticket.)

Targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`. Linux and macOS only; Windows is not supported.

## One-time setup (manual)

Done: immutable releases, Discussions, wiki, delete branches on merge; Projects tab removed.

| # | What | Why |
|---|---|---|
| 1 | Repo settings: allow squash merge only, default the commit message to the PR title, delete branches on merge | Keeps `main` one Conventional Commit per PR, which release-plz reads |
| 2 | Branch protection (a ruleset) on `main`: require the CI checks, require linear history, block force pushes and deletion | Nothing reaches a release PR without passing CI. Add it after CI exists, so the required check names are known |
| 2a | Security: enable private vulnerability reporting, Dependabot alerts and security updates, secret scanning with push protection | `SECURITY.md` and the issue chooser link to private advisories, which only works with private reporting on; `.github/dependabot.yml` already handles version updates |
| 3 | Create a GitHub App (or fine-grained PAT) with `contents: write` and `pull-requests: write` on this repo, and store it as the secrets `RELEASE_PLZ_APP_ID` and `RELEASE_PLZ_APP_PRIVATE_KEY` | With the default `GITHUB_TOKEN`, CI does not run on the release PR and the tag does not trigger dist |
| 4 | Create the public repo `cxrlos/homebrew-tap` and a fine-grained PAT with `contents: write` on it, stored here as `HOMEBREW_TAP_TOKEN` | dist pushes the formula there |
| 5 | Publish `0.1.0` to crates.io by hand (see below) | crates.io cannot trusted-publish a crate that does not exist yet |
| 6 | On crates.io, add a trusted publisher for `linear-rat`: repo `cxrlos/linear-rat`, workflow `release-plz.yml` | Lets later releases publish with OIDC instead of a token |
| 7 | Revoke the API token used in step 5 | It is no longer needed |

## First release (0.1.0)

The `feat/v0.1.0` branch carries all v0.1.0 work and is squash-merged into `main` as one commit. Then, from a clean `main`:

```bash
cargo publish --dry-run --locked
cargo publish --locked
git tag v0.1.0
git push origin v0.1.0
```

Pushing the tag runs dist and creates the GitHub release with binaries. From `0.1.1` on, release-plz takes over.

## Rolling back

A broken release is never deleted: I yank it with `cargo yank --version X.Y.Z linear-rat` and ship a fix as the next patch version. The version number of a yanked release is burned.
