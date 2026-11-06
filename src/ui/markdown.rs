//! Markdown into styled terminal lines: CommonMark plus task lists and
//! strikethrough, wrapped to a width in display columns, so a wide character
//! such as CJK takes two. Images become a placeholder with their URL, and a
//! link shows its URL after its text.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::Theme;
use crate::linear::model::StateType;

// Never wikilinks: pulldown-cmark before 0.13.6 parses them in exponential
// time (RUSTSEC-2026-0414, RUSTSEC-2026-0433).
const OPTIONS: Options = Options::ENABLE_TASKLISTS.union(Options::ENABLE_STRIKETHROUGH);

/// `markdown` as lines no wider than `width` columns.
pub fn render(markdown: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let mut renderer = Renderer::new(usize::from(width), theme);
    for event in Parser::new_ext(markdown, OPTIONS) {
        renderer.event(event);
    }
    renderer.flush();
    renderer.lines
}

/// A web link found in Markdown: where it goes, and the text it shows when
/// that is something other than the URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundLink {
    pub url: String,
    pub text: Option<String>,
}

/// Every `http` and `https` link in `markdown`, in order and with repeats:
/// Markdown links, autolinks and bare URLs in the text. Code, images and
/// other schemes such as `mailto:` are left out.
pub fn links(markdown: &str) -> Vec<FoundLink> {
    let mut found = Vec::new();
    let mut link: Option<(String, String)> = None;
    // Text is scanned for bare URLs in runs, since the parser can split one
    // URL over several text events.
    let mut run = String::new();
    let mut skipping = 0usize;
    for event in Parser::new_ext(markdown, OPTIONS) {
        match event {
            Event::Text(text) | Event::Code(text) if link.is_some() => {
                if let Some((_, shown)) = &mut link {
                    shown.push_str(&text);
                }
                continue;
            }
            Event::Text(text) if skipping == 0 => {
                run.push_str(&text);
                continue;
            }
            _ => {}
        }
        found.extend(bare_urls(&std::mem::take(&mut run)));
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                link = Some((dest_url.into_string(), String::new()));
            }
            Event::End(TagEnd::Link) => {
                if let Some((url, shown)) = link.take()
                    && is_web(&url)
                {
                    let shown = shown.trim();
                    let text = (!shown.is_empty() && shown != url).then(|| shown.to_owned());
                    found.push(FoundLink { url, text });
                }
            }
            Event::Start(Tag::Image { .. } | Tag::CodeBlock(_)) => skipping += 1,
            Event::End(TagEnd::Image | TagEnd::CodeBlock) => {
                skipping = skipping.saturating_sub(1);
            }
            _ => {}
        }
    }
    found.extend(bare_urls(&run));
    found
}

fn is_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// The URLs in plain `text`, each ending at whitespace and without the
/// punctuation that closes a sentence or an unmatched bracket around it.
fn bare_urls(text: &str) -> Vec<FoundLink> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("http") {
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`'))
            .unwrap_or(tail.len());
        let url = trim_url(&tail[..end]);
        if is_web(url) && !url.ends_with("//") {
            found.push(FoundLink {
                url: url.to_owned(),
                text: None,
            });
        }
        rest = &tail[end.max(4)..];
    }
    found
}

fn trim_url(mut url: &str) -> &str {
    loop {
        let unmatched = |open: char, close: char| {
            url.ends_with(close) && url.matches(open).count() < url.matches(close).count()
        };
        if url.ends_with(['.', ',', ';', ':', '!', '?', '\'', '*', '_'])
            || unmatched('(', ')')
            || unmatched('[', ']')
        {
            url = &url[..url.len() - 1];
        } else {
            return url;
        }
    }
}

/// `pieces` word-wrapped to `width` columns. The first line starts with
/// `first` and the rest with `rest`, for a hanging indent.
pub fn wrap(
    pieces: &[(String, Style)],
    width: u16,
    first: &[(String, Style)],
    rest: &[(String, Style)],
) -> Vec<Line<'static>> {
    let segments: Vec<Segment> = pieces
        .iter()
        .map(|(text, style)| Segment::Text(text.clone(), *style))
        .collect();
    layout(&segments, usize::from(width), first.to_vec(), rest)
}

/// Inline content waiting to be wrapped.
enum Segment {
    Text(String, Style),
    Break,
}

/// A block that indents or marks the lines inside it.
enum Container {
    Quote,
    /// The number of the next item, for an ordered list.
    List(Option<u64>),
    /// A list item. Its marker goes on its first line only; later lines are
    /// indented by `indent` columns.
    Item {
        marker: Option<String>,
        indent: usize,
    },
}

struct Link {
    url: String,
    text: String,
}

struct Renderer<'t> {
    width: usize,
    theme: &'t Theme,
    lines: Vec<Line<'static>>,
    inline: Vec<Segment>,
    styles: Vec<Style>,
    containers: Vec<Container>,
    /// A block ended, so the next one starts after a blank line.
    gap: bool,
    link: Option<Link>,
    /// An image's URL and the alt text read so far.
    image: Option<(String, String)>,
    /// The text of the code block being read.
    code: Option<String>,
}

impl<'t> Renderer<'t> {
    fn new(width: usize, theme: &'t Theme) -> Self {
        Self {
            width,
            theme,
            lines: Vec::new(),
            inline: Vec::new(),
            styles: Vec::new(),
            containers: Vec::new(),
            gap: false,
            link: None,
            image: None,
            code: None,
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                if let Some(link) = &mut self.link {
                    link.text.push_str(&code);
                }
                let style = self.style().patch(self.code_style());
                self.inline.push(Segment::Text(code.into_string(), style));
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.inline.push(Segment::Break),
            Event::Rule => {
                self.start_block();
                let prefix = self.prefix(true);
                let used: usize = prefix.iter().map(|(text, _)| text.width()).sum();
                let rule = "─".repeat(self.width.saturating_sub(used).max(1));
                let mut spans = to_spans(prefix);
                spans.push(Span::styled(rule, self.muted()));
                self.lines.push(Line::from(spans));
                self.gap = true;
            }
            Event::TaskListMarker(done) => {
                if let Some(Container::Item { marker, indent }) = self.containers.last_mut() {
                    *marker = Some(if done { "[x] " } else { "[ ] " }.to_owned());
                    *indent = 4;
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
            Event::FootnoteReference(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                self.text(&text);
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => self.start_block(),
            Tag::Heading { level, .. } => {
                self.start_block();
                self.styles.push(self.heading(level));
            }
            Tag::BlockQuote(_) => {
                self.start_block();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(_) => {
                self.start_block();
                self.code = Some(String::new());
            }
            Tag::List(first) => {
                if self.in_item() {
                    self.flush();
                } else {
                    self.start_block();
                }
                self.containers.push(Container::List(first));
            }
            Tag::Item => {
                self.flush();
                let depth = self
                    .containers
                    .iter()
                    .filter(|container| matches!(container, Container::List(_)))
                    .count();
                let marker = match self.containers.last_mut() {
                    Some(Container::List(Some(next))) => {
                        let marker = format!("{next}. ");
                        *next += 1;
                        marker
                    }
                    _ => ["• ", "◦ ", "▪ "][depth.saturating_sub(1) % 3].to_owned(),
                };
                let indent = marker.width();
                self.containers.push(Container::Item {
                    marker: Some(marker),
                    indent,
                });
            }
            Tag::Emphasis => self
                .styles
                .push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.styles
                    .push(Style::new().add_modifier(Modifier::UNDERLINED));
                self.link = Some(Link {
                    url: dest_url.into_string(),
                    text: String::new(),
                });
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some((dest_url.into_string(), String::new()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                self.code_block(&code);
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.containers.pop();
                if !self.in_item() {
                    self.gap = true;
                }
            }
            TagEnd::Item => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some(link) = self.link.take() {
                    let shown = link.url.strip_prefix("mailto:").unwrap_or(&link.url);
                    if !link.url.is_empty() && link.text != shown {
                        let style = self.style().patch(self.muted());
                        self.inline
                            .push(Segment::Text(format!(" ({})", link.url), style));
                    }
                }
            }
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    let label = if alt.trim().is_empty() {
                        "[image]".to_owned()
                    } else {
                        format!("[image: {}]", alt.trim())
                    };
                    let style = self.style().patch(self.muted());
                    self.inline.push(Segment::Text(label, style));
                    if !url.is_empty() {
                        self.inline.push(Segment::Text(format!(" {url}"), style));
                    }
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
        } else if let Some((_, alt)) = &mut self.image {
            alt.push_str(text);
        } else {
            if let Some(link) = &mut self.link {
                link.text.push_str(text);
            }
            self.inline
                .push(Segment::Text(text.to_owned(), self.style()));
        }
    }

    /// Ends the inline content before a block, and leaves a blank line after
    /// the block before it.
    fn start_block(&mut self) {
        self.flush();
        if self.gap && !self.lines.is_empty() {
            let bars: Vec<(String, Style)> = self
                .containers
                .iter()
                .filter(|container| matches!(container, Container::Quote))
                .map(|_| ("│".to_owned(), self.muted()))
                .collect();
            self.lines.push(Line::from(to_spans(bars)));
        }
        self.gap = false;
    }

    /// Wraps the inline content read so far into lines.
    fn flush(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let first = self.prefix(true);
        let rest = self.prefix(false);
        let segments = std::mem::take(&mut self.inline);
        let lines = layout(&segments, self.width, first, &rest);
        self.lines.extend(lines);
    }

    fn code_block(&mut self, code: &str) {
        let style = self.code_style();
        let mut first = self.prefix(true);
        first.push(("  ".to_owned(), Style::new()));
        let mut rest = self.prefix(false);
        rest.push(("  ".to_owned(), Style::new()));
        for (index, line) in code.trim_end_matches('\n').split('\n').enumerate() {
            let prefix = if index == 0 {
                first.clone()
            } else {
                rest.clone()
            };
            if line.is_empty() {
                self.lines.push(Line::from(to_spans(prefix)));
                continue;
            }
            let mut out = Output::new(self.width, prefix, &rest);
            for c in expand_tabs(line).chars() {
                out.push_char(c, style);
            }
            self.lines.extend(out.finish());
        }
    }

    /// What starts a line in the current containers. The first line of an
    /// item carries its marker, once.
    fn prefix(&mut self, first: bool) -> Vec<(String, Style)> {
        let muted = self.muted();
        let mut prefix = Vec::new();
        for container in &mut self.containers {
            match container {
                Container::Quote => prefix.push(("│ ".to_owned(), muted)),
                Container::List(_) => {}
                Container::Item { marker, indent } => {
                    let text = match marker.take() {
                        Some(marker) if first => marker,
                        taken => {
                            *marker = taken;
                            " ".repeat(*indent)
                        }
                    };
                    prefix.push((text, muted));
                }
            }
        }
        prefix
    }

    fn in_item(&self) -> bool {
        self.containers
            .iter()
            .any(|container| matches!(container, Container::Item { .. }))
    }

    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::new(), |style, patch| style.patch(*patch))
    }

    fn heading(&self, level: HeadingLevel) -> Style {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        match level {
            HeadingLevel::H1 => bold
                .fg(self.theme.accent)
                .add_modifier(Modifier::UNDERLINED),
            HeadingLevel::H2 => bold.fg(self.theme.accent),
            _ => bold,
        }
    }

    fn muted(&self) -> Style {
        Style::new().fg(self.theme.muted)
    }

    /// Code takes the Base16 strings color, which themes give the completed
    /// state.
    fn code_style(&self) -> Style {
        Style::new().fg(self.theme.state(StateType::Completed))
    }
}

/// Columns between tab stops in a code block.
const TAB_STOP: usize = 4;

/// `line` with each tab replaced by spaces up to the next tab stop. A tab has
/// no display width and the terminal would drop it, losing the indentation.
fn expand_tabs(line: &str) -> String {
    let mut expanded = String::with_capacity(line.len());
    let mut column = 0;
    for c in line.chars() {
        if c == '\t' {
            let spaces = TAB_STOP - column % TAB_STOP;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(c);
            column += c.width().unwrap_or(0);
        }
    }
    expanded
}

fn to_spans(pieces: Vec<(String, Style)>) -> Vec<Span<'static>> {
    pieces
        .into_iter()
        .map(|(text, style)| Span::styled(text, style))
        .collect()
}

/// A word, a run of spaces or a line break, as the wrapper sees it.
enum Token {
    Word(Vec<(char, Style)>),
    Space(Vec<(char, Style)>),
    Break,
}

/// Splits `segments` into words and spaces. Each wide character is a word of
/// its own, since CJK text breaks between any two characters.
fn tokens(segments: &[Segment]) -> Vec<Token> {
    let mut tokens = Vec::new();
    for segment in segments {
        let (text, style) = match segment {
            Segment::Break => {
                tokens.push(Token::Break);
                continue;
            }
            Segment::Text(text, style) => (text, *style),
        };
        for c in text.chars() {
            if c == '\n' {
                tokens.push(Token::Break);
            } else if c.is_whitespace() {
                match tokens.last_mut() {
                    Some(Token::Space(chars)) => chars.push((' ', style)),
                    _ => tokens.push(Token::Space(vec![(' ', style)])),
                }
            } else if c.width().unwrap_or(0) > 1 {
                tokens.push(Token::Word(vec![(c, style)]));
            } else {
                match tokens.last_mut() {
                    Some(Token::Word(chars)) if !is_wide(chars) => chars.push((c, style)),
                    _ => tokens.push(Token::Word(vec![(c, style)])),
                }
            }
        }
    }
    while matches!(tokens.last(), Some(Token::Break | Token::Space(_))) {
        tokens.pop();
    }
    tokens
}

fn is_wide(chars: &[(char, Style)]) -> bool {
    chars
        .first()
        .is_some_and(|(c, _)| c.width().unwrap_or(0) > 1)
}

fn chars_width(chars: &[(char, Style)]) -> usize {
    chars.iter().map(|(c, _)| c.width().unwrap_or(0)).sum()
}

fn layout(
    segments: &[Segment],
    width: usize,
    first: Vec<(String, Style)>,
    rest: &[(String, Style)],
) -> Vec<Line<'static>> {
    let mut out = Output::new(width, first, rest);
    let mut space: Vec<(char, Style)> = Vec::new();
    for token in tokens(segments) {
        match token {
            Token::Break => {
                out.new_line();
                space.clear();
            }
            Token::Space(chars) => {
                if !out.at_start {
                    space = chars;
                }
            }
            Token::Word(chars) => {
                let word = chars_width(&chars);
                let gap = chars_width(&space);
                if !out.at_start && out.used + gap + word > width {
                    out.new_line();
                } else if !out.at_start {
                    for (c, style) in space.drain(..) {
                        out.push_char(c, style);
                    }
                }
                space.clear();
                for (c, style) in chars {
                    out.push_char(c, style);
                }
            }
        }
    }
    out.finish()
}

/// Lines being filled left to right.
struct Output<'r> {
    width: usize,
    rest: &'r [(String, Style)],
    lines: Vec<Line<'static>>,
    line: Vec<(String, Style)>,
    used: usize,
    /// Nothing but the prefix is on the line yet.
    at_start: bool,
}

impl<'r> Output<'r> {
    fn new(width: usize, first: Vec<(String, Style)>, rest: &'r [(String, Style)]) -> Self {
        let used = first.iter().map(|(text, _)| text.width()).sum();
        Self {
            width,
            rest,
            lines: Vec::new(),
            line: first,
            used,
            at_start: true,
        }
    }

    fn new_line(&mut self) {
        let line = std::mem::replace(&mut self.line, self.rest.to_vec());
        self.lines.push(Line::from(to_spans(line)));
        self.used = self.rest.iter().map(|(text, _)| text.width()).sum();
        self.at_start = true;
    }

    /// Adds `c`, starting a new line first if it does not fit. A line holds
    /// at least one character, however narrow the width.
    fn push_char(&mut self, c: char, style: Style) {
        let width = c.width().unwrap_or(0);
        if !self.at_start && self.used + width > self.width {
            self.new_line();
            if c == ' ' {
                return;
            }
        }
        match self.line.last_mut() {
            Some((text, last)) if *last == style && !self.at_start => text.push(c),
            _ => self.line.push((c.to_string(), style)),
        }
        self.used += width;
        self.at_start = false;
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.at_start {
            let line = std::mem::take(&mut self.line);
            self.lines.push(Line::from(to_spans(line)));
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    fn rendered(markdown: &str, width: u16) -> Vec<String> {
        text(&render(markdown, width, &Theme::default()))
    }

    #[test]
    fn paragraphs_wrap_at_word_boundaries_with_a_blank_line_between() {
        let lines = rendered("The cellar is cold and damp.\n\nAge it longer.", 12);

        assert_eq!(
            lines,
            [
                "The cellar",
                "is cold and",
                "damp.",
                "",
                "Age it",
                "longer."
            ]
        );
    }

    #[test]
    fn a_cjk_line_breaks_between_characters_and_never_overflows() {
        let lines = rendered("チーズの熟成を確認する", 9);

        assert_eq!(lines, ["チーズの", "熟成を確", "認する"]);
        for line in &lines {
            assert!(line.width() <= 9, "{line:?}");
        }
    }

    #[test]
    fn a_word_longer_than_the_width_is_cut() {
        let lines = rendered("see https://example.com/a/very/long/path", 10);

        assert_eq!(
            lines,
            ["see", "https://ex", "ample.com/", "a/very/lon", "g/path"]
        );
    }

    #[test]
    fn list_items_hang_under_their_marker() {
        let lines = rendered("- one two three\n- four\n  1. five\n  2. six", 10);

        assert_eq!(
            lines,
            ["• one two", "  three", "• four", "  1. five", "  2. six"]
        );
    }

    #[test]
    fn task_items_show_a_box_instead_of_a_bullet() {
        let lines = rendered("- [x] Ship it\n- [ ] Tell support", 40);

        assert_eq!(lines, ["[x] Ship it", "[ ] Tell support"]);
    }

    #[test]
    fn a_link_shows_its_url_unless_it_is_the_text() {
        let lines = rendered(
            "[runbook](https://wiki.example/runbook) and <https://example.com>",
            80,
        );

        assert_eq!(
            lines,
            ["runbook (https://wiki.example/runbook) and https://example.com"]
        );
    }

    fn found(markdown: &str) -> Vec<(String, Option<String>)> {
        links(markdown)
            .into_iter()
            .map(|link| (link.url, link.text))
            .collect()
    }

    #[test]
    fn links_are_markdown_links_autolinks_and_bare_urls_in_order() {
        let links = found(
            "See [the runbook](https://wiki.example/runbook), <https://example.com> and \
             https://status.example/page_one.\n\nAlso (https://paren.example/a) and \
             [https://same.example](https://same.example).",
        );

        assert_eq!(
            links,
            [
                (
                    "https://wiki.example/runbook".to_owned(),
                    Some("the runbook".to_owned())
                ),
                ("https://example.com".to_owned(), None),
                ("https://status.example/page_one".to_owned(), None),
                ("https://paren.example/a".to_owned(), None),
                ("https://same.example".to_owned(), None),
            ]
        );
    }

    #[test]
    fn links_skip_code_images_and_what_is_not_the_web() {
        let links = found(
            "`https://code.example` ![photo](https://uploads.example/a.png) \
             [mail](mailto:a@example.com)\n\n```\nhttps://block.example\n```\n\n\
             https://wiki.example/Cheese_(aged)",
        );

        assert_eq!(
            links,
            [("https://wiki.example/Cheese_(aged)".to_owned(), None)]
        );
    }

    #[test]
    fn an_image_becomes_a_placeholder_with_its_url() {
        let lines = rendered("![Rind photo](https://uploads.example/rind.png)", 80);

        assert_eq!(
            lines,
            ["[image: Rind photo] https://uploads.example/rind.png"]
        );
    }

    #[test]
    fn a_code_block_keeps_its_lines_and_spaces() {
        let lines = rendered("```\nfn main() {\n    age();\n}\n```", 40);

        assert_eq!(lines, ["  fn main() {", "      age();", "  }"]);
    }

    #[test]
    fn tabs_in_a_code_block_become_spaces_to_the_next_stop() {
        let lines = rendered("```\n\tif x {\n\t\ty();\n\t}\nab\tc\n```", 40);

        assert_eq!(
            lines,
            ["      if x {", "          y();", "      }", "  ab  c"]
        );
    }

    /// The style of the first span of `lines` whose text is `text`.
    fn style_of(lines: &[Line], text: &str) -> Style {
        lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == text)
            .map(|span| span.style)
            .unwrap_or_else(|| panic!("no span {text:?} in {lines:?}"))
    }

    #[test]
    fn headings_and_emphasis_are_styled() {
        let theme = Theme::default();
        let lines = render(
            "# Big\n\n## Mid\n\n### Small\n\n*it* **bo** ~~st~~ `co` [li](https://x.example)",
            80,
            &theme,
        );

        let bold = Style::new().add_modifier(Modifier::BOLD);
        assert_eq!(
            style_of(&lines, "Big"),
            bold.fg(theme.accent).add_modifier(Modifier::UNDERLINED)
        );
        assert_eq!(style_of(&lines, "Mid"), bold.fg(theme.accent));
        assert_eq!(style_of(&lines, "Small"), bold);
        assert_eq!(
            style_of(&lines, "it"),
            Style::new().add_modifier(Modifier::ITALIC)
        );
        assert_eq!(style_of(&lines, "bo"), bold);
        assert_eq!(
            style_of(&lines, "st"),
            Style::new().add_modifier(Modifier::CROSSED_OUT)
        );
        assert_eq!(
            style_of(&lines, "co"),
            Style::new().fg(theme.state(StateType::Completed))
        );
        assert_eq!(
            style_of(&lines, "li"),
            Style::new().add_modifier(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn a_quote_marks_each_line() {
        let lines = rendered("> one two three\n>\n> four", 10);

        assert_eq!(lines, ["│ one two", "│ three", "│", "│ four"]);
    }
}
