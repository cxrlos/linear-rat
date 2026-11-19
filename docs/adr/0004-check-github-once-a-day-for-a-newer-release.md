# Check GitHub once a day for a newer release

lrat asks GitHub's `releases/latest` at most once a day, in the background after the first frame, and shows a muted release notice when a newer stable version is out. It is the only request lrat makes to anything other than Linear. It is on by default because an opt-in check reaches almost nobody, and the point is fewer bug reports against old versions. The request carries no key and no workspace data, and `[updates] check = false` or `LRAT_NO_UPDATE_CHECK` turns it off.

## Considered options

- **Opt-in**: rejected, for the reach reason above.
- **crates.io as the source**: rejected. A version is on crates.io before its binaries, installer and Homebrew formula exist, while a GitHub release is published only once every asset is attached.

## Consequences

Installs that the package manager keeps up to date and that may lag the release (AUR, Nix) show no notice. Homebrew installs show `brew upgrade linear-rat`. `cargo install`, `cargo binstall` and the shell installer all land in the cargo bin directory with no receipt (dist's updater is off), so their notice points at the README's install section rather than one command. `--demo`, tests and headless subcommands never check; `lrat doctor` checks live.
