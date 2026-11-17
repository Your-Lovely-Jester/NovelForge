//! Conservative chapter XHTML cleanup.
//!
//! lncrawl already extracts the story body, but some adapters leave the
//! chapter title twice and a few sites inject navigation, ad or donation
//! blocks. Cleaning happens on the split XHTML files only; the bundled EPUB is
//! never rewritten.
//!
//! Rules are deliberately narrow:
//!
//! - navigation, ad, donation and sharing elements listed in
//!   [`COMMON_CHROME`] plus the source's [`sources::SourceProfile::chapter_chrome`]
//!   are removed,
//! - an `aria-hidden` or numeric leading marker heading (`#1`, `1`) is removed,
//! - a *duplicate* leading heading is removed (the first visible title stays),
//! - a leading paragraph that only repeats the chapter title is removed,
//! - everything else is preserved in document order with its text intact.
//!
//! If the XHTML cannot be parsed, [`ChapterCleaner::clean`] returns `None` and
//! the caller keeps the original bytes, so a cleaning failure never loses
//! story text.

use crate::sources;
use ego_tree::NodeRef;
use regex::Regex;
use scraper::node::{Element, Node};
use scraper::{Html, Selector};
use std::sync::OnceLock;

/// Navigation/ad/chrome selectors applied to every source.
pub const COMMON_CHROME: &[&str] = &[
    "nav",
    "[role=navigation]",
    ".chapter-nav",
    ".chapter-navigation",
    ".chapter-nav-top",
    ".chapter-nav-bottom",
    ".nav-links",
    ".prev-next",
    ".next-prev",
    ".ad",
    ".ads",
    ".advert",
    ".advertisement",
    ".adsbygoogle",
    ".ad-container",
    ".ad-wrapper",
    ".patreon",
    ".donation",
    ".donate",
    ".ko-fi",
    ".kofi",
    ".buy-me-a-coffee",
    ".sharedaddy",
    ".jp-relatedposts",
    ".post-navigation",
    ".site-navigation",
    "#chapter-nav",
    "#chapter-navigation",
    "#nav-chapters",
    "#ad",
    "#ads",
    "#advertisement",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    Tag(String),
    Class(String),
    Id(String),
    Attr(String, String),
}

/// Reusable cleaner for one export. Build it once, then clean every chapter.
pub struct ChapterCleaner {
    rules: Vec<Rule>,
}

impl ChapterCleaner {
    /// Build a cleaner for a source host or URL. Unknown sources still get the
    /// common chrome rules.
    pub fn for_source(source: Option<&str>) -> Self {
        let profile = sources::for_host(source.unwrap_or_default());
        let mut tokens = COMMON_CHROME.to_vec();
        tokens.extend_from_slice(profile.chapter_chrome);
        Self::with_selectors(&tokens)
    }

    /// Build a cleaner from explicit `tag`, `.class`, `#id` or `[attr=value]`
    /// tokens. Invalid tokens are ignored.
    pub fn with_selectors(tokens: &[&str]) -> Self {
        Self {
            rules: tokens
                .iter()
                .filter_map(|token| parse_rule(token))
                .collect(),
        }
    }

    /// Returns cleaned XHTML, or `None` when the input is not UTF-8 or does not
    /// contain an HTML document. Callers must keep the original bytes on `None`.
    pub fn clean(&self, bytes: &[u8], chapter_title: &str) -> Option<Vec<u8>> {
        let source = std::str::from_utf8(bytes).ok()?;
        let document = Html::parse_document(source);
        let root = document.tree.root();
        if !root.children().any(|child| child.value().is_element()) {
            return None;
        }

        let mut title_keys = Vec::new();
        push_title_key(chapter_title, &mut title_keys);
        if let Ok(selector) = Selector::parse("title")
            && let Some(title) = document.select(&selector).next()
        {
            push_title_key(&title.text().collect::<String>(), &mut title_keys);
        }

        let mut output = String::with_capacity(source.len());
        output.push_str("<?xml version='1.0' encoding='utf-8'?>\n<!DOCTYPE html>\n");
        for child in root.children() {
            if child.value().is_element() {
                self.write_node(child, &mut output, None, &title_keys);
            }
        }
        Some(output.into_bytes())
    }

    fn write_node(
        &self,
        node: NodeRef<'_, Node>,
        output: &mut String,
        lead: Option<&mut Lead>,
        title_keys: &[String],
    ) {
        match node.value() {
            Node::Text(text) => {
                if let Some(state) = lead
                    && state.active
                    && !text.text.trim().is_empty()
                {
                    state.active = false;
                }
                output.push_str(&escape_text(&text.text));
            }
            Node::Element(element) => {
                let name = element.name();
                if self.removes(name, element) {
                    return;
                }

                let is_body = name == "body";
                let mut local_lead;
                let mut lead = if is_body {
                    local_lead = Lead::start();
                    Some(&mut local_lead)
                } else {
                    lead
                };

                if let Some(state) = lead.as_deref_mut()
                    && !is_body
                    && state.active
                    && self.drop_leading(node, element, name, state, title_keys)
                {
                    return;
                }

                output.push('<');
                output.push_str(name);
                for (attribute, value) in &element.attrs {
                    output.push(' ');
                    if let Some(prefix) = attribute.prefix.as_ref() {
                        output.push_str(prefix);
                        output.push(':');
                    }
                    output.push_str(&attribute.local);
                    output.push_str("=\"");
                    output.push_str(&escape_attribute(value));
                    output.push('"');
                }
                // Split chapter files keep the `.xhtml` extension, so void
                // elements must be self-closing: `<link …>` made every export
                // unparseable as XML (xmllint: "Opening and ending tag mismatch
                // … link and head"). HTML readers accept `<br/>` unchanged.
                if is_void(name) {
                    output.push_str("/>");
                    return;
                }
                output.push('>');

                if matches!(name, "script" | "style") {
                    // CSS/JS text must not be HTML-escaped.
                    let mut raw = String::new();
                    collect_text(node, &mut raw);
                    output.push_str(&raw);
                } else {
                    let propagate = is_body || is_container(name);
                    for child in node.children() {
                        self.write_node(
                            child,
                            output,
                            if propagate { lead.as_deref_mut() } else { None },
                            title_keys,
                        );
                    }
                }
                output.push_str("</");
                output.push_str(name);
                output.push('>');
            }
            // Doctype, comments and processing instructions are replaced by the
            // fixed XML/doctype header.
            _ => {}
        }
    }

    fn removes(&self, name: &str, element: &Element) -> bool {
        // Never remove the document skeleton or a known content wrapper even if
        // a broad page class happens to match it.
        if matches!(name, "html" | "head" | "body" | "main") {
            return false;
        }
        if matches!(
            element.id(),
            Some("chapter") | Some("content") | Some("chapter-content")
        ) {
            return false;
        }
        self.rules.iter().any(|rule| match rule {
            Rule::Tag(tag) => tag == name,
            Rule::Class(class) => element.classes().any(|value| value == class),
            Rule::Id(id) => element.id() == Some(id.as_str()),
            Rule::Attr(key, value) => element.attr(key) == Some(value.as_str()),
        })
    }

    /// Returns true when a leading title marker/duplicate heading or a
    /// title-only paragraph should be dropped.
    fn drop_leading(
        &self,
        node: NodeRef<'_, Node>,
        element: &Element,
        name: &str,
        state: &mut Lead,
        title_keys: &[String],
    ) -> bool {
        if is_heading(name) {
            let text = text_of(node);
            if element
                .attr("aria-hidden")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                || marker_heading().is_match(text.trim())
            {
                return true;
            }
            let key = title_key(&text);
            if !key.is_empty() && state.heading_keys.iter().any(|seen| seen == &key) {
                return true;
            }
            if !key.is_empty() {
                state.heading_keys.push(key);
            }
            state.kept_heading = true;
            return false;
        }

        if name == "p" {
            let text = text_of(node);
            let normalized = title_key(&text);
            if state.kept_heading
                && is_title_only_paragraph(node, &normalized, text.trim(), title_keys)
            {
                return true;
            }
            state.active = false;
            return false;
        }

        if is_container(name) {
            return false;
        }

        state.active = false;
        false
    }
}

#[derive(Default)]
struct Lead {
    active: bool,
    kept_heading: bool,
    heading_keys: Vec<String>,
}

impl Lead {
    fn start() -> Self {
        Self {
            active: true,
            kept_heading: false,
            heading_keys: Vec::new(),
        }
    }
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn is_container(name: &str) -> bool {
    matches!(
        name,
        "div" | "section" | "article" | "header" | "span" | "center" | "font" | "blockquote"
    )
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn text_of(node: NodeRef<'_, Node>) -> String {
    let mut text = String::new();
    collect_text(node, &mut text);
    text
}

fn collect_text(node: NodeRef<'_, Node>, output: &mut String) {
    match node.value() {
        Node::Text(value) => output.push_str(&value.text),
        Node::Element(_) => {
            for child in node.children() {
                collect_text(child, output);
            }
        }
        _ => {}
    }
}

fn is_title_only_paragraph(
    node: NodeRef<'_, Node>,
    key: &str,
    text: &str,
    title_keys: &[String],
) -> bool {
    if key.len() < 12 || text.chars().count() > 300 {
        return false;
    }
    if title_keys.iter().any(|title| title == key) {
        return true;
    }
    // Some sources keep the title as a bold subtitle line, e.g.
    // `<p><strong>Chapter 1 – The heart…</strong></p>`, while the index title is
    // only "Chapter 1". A leading paragraph whose sole child is bold/italic and
    // that starts with a chapter marker is still a title, not story text.
    has_single_title_element(node) && chapter_prefix().is_match(text)
}

/// True when the element has exactly one child element and that child is
/// `strong`, `em` or `b`, with no other non-whitespace text.
fn has_single_title_element(node: NodeRef<'_, Node>) -> bool {
    let mut saw_element = false;
    for child in node.children() {
        match child.value() {
            Node::Text(value) => {
                if !value.text.trim().is_empty() {
                    return false;
                }
            }
            Node::Element(element) => {
                if saw_element || !matches!(element.name(), "strong" | "em" | "b") {
                    return false;
                }
                saw_element = true;
            }
            _ => {}
        }
    }
    saw_element
}

fn push_title_key(value: &str, keys: &mut Vec<String>) {
    let key = title_key(value);
    if key.len() >= 8 && !keys.contains(&key) {
        keys.push(key);
    }
}

/// Strips a leading `Chapter 1`, `Ch. 12`, `Volume 2` style prefix, then keeps
/// only alphanumeric characters. `Chapter 1 - 1: The Heart` and
/// `Chapter 1 – The Heart` therefore compare equal without erasing real title
/// words.
fn title_key(value: &str) -> String {
    let stripped = chapter_prefix().replace(value, "");
    stripped
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn chapter_prefix() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:chapter|ch\.|part|volume|vol\.)\s*[0-9ivxlcdm]+(?:\s*[-–—:.]\s*[0-9ivxlcdm]+)*\s*[-–—:.]?\s*",
        )
        .expect("chapter prefix regex")
    })
}

fn marker_heading() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^#?\s*\d{1,5}\s*$").expect("marker heading regex"))
}

fn parse_rule(token: &str) -> Option<Rule> {
    let token = token.trim();
    if let Some(class) = token.strip_prefix('.') {
        return (!class.is_empty()).then(|| Rule::Class(class.to_ascii_lowercase()));
    }
    if let Some(id) = token.strip_prefix('#') {
        return (!id.is_empty()).then(|| Rule::Id(id.to_ascii_lowercase()));
    }
    if let Some(inner) = token
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        let (key, value) = inner.split_once('=')?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        return (!key.is_empty() && !value.is_empty()).then_some(Rule::Attr(key, value));
    }
    if token.is_empty() {
        None
    } else {
        Some(Rule::Tag(token.to_ascii_lowercase()))
    }
}

fn escape_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
    output
}

fn escape_attribute(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_CHAPTER: &str = include_str!("../tests/fixtures/tell_tale_heart_chapter_0001.xhtml");

    fn clean(html: &str, title: &str) -> String {
        let cleaner = ChapterCleaner::with_selectors(COMMON_CHROME);
        let bytes = cleaner.clean(html.as_bytes(), title).expect("clean");
        String::from_utf8(bytes).expect("utf8")
    }

    fn page(body: &str) -> String {
        format!(
            r#"<?xml version='1.0' encoding='utf-8'?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" epub:prefix="z3998: http://www.daisy.org/z3998/2012/vocab/structure/#" lang="en">
<head><title>Chapter 1: The Heart Of A Demon</title><link href="style.css" rel="stylesheet" type="text/css"/></head>
<body><div id="chapter">{body}</div></body>
</html>"#
        )
    }

    fn paragraph_texts(html: &str) -> Vec<String> {
        let document = Html::parse_document(html);
        let selector = Selector::parse("p").expect("selector");
        document
            .select(&selector)
            .map(|paragraph| paragraph.text().collect::<String>())
            .collect()
    }

    #[test]
    fn removes_duplicate_marker_and_keeps_the_real_chapter() {
        let output = clean(REAL_CHAPTER, "The Tell-Tale Heart");
        assert!(output.starts_with("<?xml version='1.0' encoding='utf-8'?>"));
        assert!(output.contains("xmlns:epub=\"http://www.idpf.org/2007/ops\""));
        assert!(output.contains("<h1>The Tell-Tale Heart</h1>"));
        assert!(!output.contains("aria-hidden"));
        assert!(!output.contains("<h4"));
        assert!(output.contains("TRUE!—nervous—very, very dreadfully nervous"));

        let before = paragraph_texts(REAL_CHAPTER);
        let after = paragraph_texts(&output);
        assert_eq!(before, after, "every story paragraph must survive");
        assert!(after.len() > 5);
    }

    #[test]
    fn removes_title_paragraph_but_keeps_story_paragraphs() {
        let html = page(
            r#"<h4 style="opacity: 0.75" aria-hidden="true">#1</h4>
<h1>Chapter 1</h1>
<p><strong>Chapter 1 – The Heart Of A Demon</strong></p>
<p>The first real story sentence.</p>
<p>The second real story sentence.</p>"#,
        );
        let output = clean(&html, "Chapter 1 - 1: The Heart Of A Demon");
        assert!(output.contains("<h1>Chapter 1</h1>"));
        assert!(!output.contains("<strong>Chapter 1 – The Heart Of A Demon</strong>"));
        assert!(output.contains("The first real story sentence."));
        assert!(output.contains("The second real story sentence."));
    }

    #[test]
    fn removes_navigation_ads_and_donations_without_touching_story() {
        let html = page(
            r#"<nav><a href="/prev">Previous</a><a href="/next">Next</a></nav>
<div class="adsbygoogle">Buy pills</div>
<div class="patreon">Support me on Patreon</div>
<p>Story line one.</p>
<p>Story line two.</p>"#,
        );
        let output = clean(&html, "Chapter 1: The Heart Of A Demon");
        assert!(!output.contains("Previous"));
        assert!(!output.contains("Buy pills"));
        assert!(!output.contains("Patreon"));
        assert!(output.contains("Story line one."));
        assert!(output.contains("Story line two."));
    }

    #[test]
    fn source_specific_selectors_are_configurable() {
        let html = page(
            r#"<div class="site-chrome">Remove this</div>
<p>Keep this story line.</p>"#,
        );
        let cleaner = ChapterCleaner::with_selectors(&[".site-chrome"]);
        let output = String::from_utf8(
            cleaner
                .clean(html.as_bytes(), "Chapter 1: The Heart Of A Demon")
                .expect("clean"),
        )
        .unwrap();
        assert!(!output.contains("Remove this"));
        assert!(output.contains("Keep this story line."));

        assert!(
            sources::for_host("novelfire.net")
                .chapter_chrome
                .contains(&".adsbygoogle")
        );
        assert!(
            sources::for_host("divinedaolibrary.com")
                .chapter_chrome
                .contains(&".sharedaddy")
        );
        assert!(sources::for_host("example.com").chapter_chrome.is_empty());
    }

    #[test]
    fn title_paragraph_stays_when_no_heading_exists() {
        let html = page(
            r#"<p><strong>Chapter 1 – The only title in this file</strong></p>
<p>Story line that must stay.</p>"#,
        );
        let output = clean(&html, "Chapter 1");
        assert!(output.contains("<strong>Chapter 1 – The only title in this file</strong>"));
        assert!(output.contains("Story line that must stay."));
    }

    #[test]
    fn long_story_paragraphs_that_mention_the_title_are_preserved() {
        let story = format!(
            "Chapter 1: The Heart Of A Demon was the phrase the old woman kept repeating, and nobody in the village could explain why. {}",
            "More story follows. ".repeat(20)
        );
        let html = page(&format!("<p>{story}</p>"));
        let output = clean(&html, "Chapter 1: The Heart Of A Demon");
        assert!(output.contains(&story));
    }

    #[test]
    fn protected_content_wrapper_is_never_removed() {
        let html = page(r#"<div id="chapter" class="ad"><p>Story that must stay.</p></div>"#);
        let output = clean(&html, "Chapter 1: The Heart Of A Demon");
        assert!(output.contains("Story that must stay."));
        assert!(output.contains("id=\"chapter\""));
    }

    #[test]
    fn inline_style_is_not_html_escaped() {
        let html = r#"<html><head><title>Chapter 1</title><style>.x > p { color: red; }</style></head><body><p>Story text.</p></body></html>"#;
        let output = clean(html, "Chapter 1");
        assert!(output.contains(".x > p { color: red; }"));
        assert!(output.contains("Story text."));
    }

    #[test]
    fn invalid_or_empty_input_falls_back_to_the_original() {
        let cleaner = ChapterCleaner::with_selectors(COMMON_CHROME);
        assert!(cleaner.clean(&[0xff, 0xfe, 0x00], "Chapter 1").is_none());
    }

    #[test]
    fn void_elements_are_self_closing_so_the_xhtml_parses() {
        let html = page("<p>Line one.<br>Line two.</p><img src=\"art.jpg\" alt=\"art\">");
        let output = clean(&html, "Chapter 1");
        assert!(output.contains("<link href=\"style.css\" rel=\"stylesheet\" type=\"text/css\"/>"));
        assert!(output.contains("<br/>"));
        assert!(output.contains("<img alt=\"art\" src=\"art.jpg\"/>"));
        assert!(!output.contains("<br>"));
    }
}
