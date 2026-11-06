/// Every link and image target in `markdown`: inline `](target)`, reference
/// definitions `[label]: target`, and HTML `src="..."` / `href="..."`.
fn link_targets(markdown: &str) -> Vec<&str> {
    let inline = markdown
        .split("](")
        .skip(1)
        .filter_map(|rest| rest.split(')').next());
    let reference = markdown.lines().filter_map(|line| {
        let rest = line.trim_start().strip_prefix('[')?;
        let (_, after) = rest.split_once("]:")?;
        after.split_whitespace().next()
    });
    let html = ["src=\"", "href=\""].into_iter().flat_map(|attr| {
        markdown
            .split(attr)
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
    });
    inline.chain(reference).chain(html).collect()
}

#[test]
fn link_targets_cover_inline_reference_and_html_forms() {
    let markdown = "See [auth](docs/auth.md) and ![gif](https://x/demo.gif).\n\
                    [a]: docs/config.md\n  [b]: https://x/b \"title\"\n\
                    <img src=\"docs/demo.gif\" alt=\"demo\"> <a href=\"SECURITY.md\">s</a>\n";
    assert_eq!(
        link_targets(markdown),
        [
            "docs/auth.md",
            "https://x/demo.gif",
            "docs/config.md",
            "https://x/b",
            "docs/demo.gif",
            "SECURITY.md",
        ]
    );
}

/// crates.io renders README.md but cannot resolve relative links or images,
/// so every link target in it is an absolute https URL.
#[test]
fn every_readme_link_is_absolute() {
    let relative: Vec<&str> = link_targets(include_str!("../README.md"))
        .into_iter()
        .filter(|target| !target.starts_with("https://"))
        .collect();
    assert!(
        relative.is_empty(),
        "README.md links these relative targets, which crates.io cannot resolve: {relative:?}"
    );
}
