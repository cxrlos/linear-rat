//! The completion scripts and man pages under `assets/`, committed so that
//! release archives and distro packages can ship them as plain files.

use std::io;
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use linear_rat::cli::{Cli, CompletionShell};
use linear_rat::completions;

const ASSETS: &str = "assets";

/// Each generated file, by its path under `assets/`.
fn generated() -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = vec![
        (
            "completions/lrat.bash".to_owned(),
            completions::script(CompletionShell::Bash),
        ),
        (
            "completions/_lrat".to_owned(),
            completions::script(CompletionShell::Zsh),
        ),
        (
            "completions/lrat.fish".to_owned(),
            completions::script(CompletionShell::Fish),
        ),
    ];
    // As `clap_mangen::generate_to` does: `lrat help` needs no pages of its own.
    let mut command = Cli::command().disable_help_subcommand(true);
    command.build();
    man_pages(&command, &mut files)?;
    Ok(files)
}

/// The man page for `command` and, recursively, for each visible subcommand,
/// named as `clap_mangen::generate_to` names them (`lrat-auth-login.1`).
///
/// The pages carry no version, so a release that only bumps it leaves them
/// unchanged and the release PR stays green.
fn man_pages(command: &clap::Command, files: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    let man = clap_mangen::Man::new(command.clone()).source("lrat");
    let mut page = Vec::new();
    man.render(&mut page)?;
    let mut page = String::from_utf8(page).map_err(io::Error::other)?;
    // VERSION is the last section, as lrat sets no author.
    if let Some(start) = page.find(".SH VERSION\n") {
        page.truncate(start);
    }
    files.push((format!("man/{}", man.get_filename()), page.into_bytes()));
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        man_pages(sub, files)?;
    }
    Ok(())
}

/// Every file under `dir`, as a path relative to it; none when it is missing.
fn files_under(dir: &Path, prefix: &str, found: &mut Vec<String>) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        entries => entries?,
    };
    for entry in entries {
        let entry = entry?;
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            files_under(&entry.path(), &format!("{name}/"), found)?;
        } else {
            found.push(name);
        }
    }
    Ok(())
}

/// Fails when the CLI changed and `assets/` was not regenerated.
/// `UPDATE_ASSETS=1 cargo test --test generated_assets` rewrites it.
#[test]
fn assets_match_the_cli() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ASSETS);
    let expected = generated().unwrap();
    if std::env::var_os("UPDATE_ASSETS").is_some() {
        let _ = std::fs::remove_dir_all(&root);
        for (path, contents) in &expected {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        return;
    }
    let regenerate = "regenerate them with `UPDATE_ASSETS=1 cargo test --test generated_assets` \
                      and commit the result";
    for (path, contents) in &expected {
        let actual = std::fs::read(root.join(path)).unwrap_or_default();
        assert!(
            actual == *contents,
            "{ASSETS}/{path} is out of date with the CLI; {regenerate}"
        );
    }
    let mut found = Vec::new();
    files_under(&root, "", &mut found).unwrap();
    found.sort();
    let mut names: Vec<String> = expected.into_iter().map(|(path, _)| path).collect();
    names.sort();
    assert_eq!(
        found, names,
        "{ASSETS}/ holds files the CLI does not generate; {regenerate}"
    );
}

#[test]
fn the_man_page_lists_the_commands_and_flags() {
    let page = generated()
        .unwrap()
        .into_iter()
        .find(|(path, _)| path == "man/lrat.1")
        .map(|(_, page)| String::from_utf8(page).unwrap())
        .unwrap();

    assert!(page.starts_with(".ie \\n(.g .ds Aq"), "{page}");
    assert!(page.contains(".TH lrat 1"), "{page}");
    for word in [
        "\\-\\-demo",
        "lrat\\-auth(1)",
        "lrat\\-completions(1)",
        "\\-\\-version",
    ] {
        assert!(page.contains(word), "missing {word}:\n{page}");
    }
}

#[test]
fn no_asset_names_the_version_so_a_release_bump_leaves_them_unchanged() {
    for (path, contents) in generated().unwrap() {
        let text = String::from_utf8(contents).unwrap();
        assert!(!text.contains(env!("CARGO_PKG_VERSION")), "{path}:\n{text}");
    }
}

#[test]
fn every_subcommand_has_a_man_page_and_help_has_none() {
    let pages: Vec<String> = generated()
        .unwrap()
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| path.starts_with("man/"))
        .collect();

    for page in [
        "man/lrat.1",
        "man/lrat-auth.1",
        "man/lrat-auth-login.1",
        "man/lrat-auth-logout.1",
        "man/lrat-auth-status.1",
        "man/lrat-completions.1",
    ] {
        assert!(pages.iter().any(|path| path == page), "{page}: {pages:?}");
    }
    assert!(!pages.iter().any(|path| path.contains("help")), "{pages:?}");
}
