//! Fast, real metadata for search rows and the chapter picker.
//!
//! lncrawl has to walk every TOC page before it can print a novel title. That
//! is far too slow for a search result list, so the app reads the source detail
//! page directly for title/author/cover/synopsis/tags, then fetches the TOC
//! pages concurrently only when the user asks for the chapter index.

use crate::cache;
use crate::error::{AppError, AppResult};
use crate::model::{NovelChapter, NovelChapterIndex, NovelMetadata};
use crate::settings::Settings;
use crate::sources::{self, ParserKind};
use crate::transport::{self, RequestKind};
use base64::Engine as _;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const PAGE_TIMEOUT: Duration = Duration::from_secs(18);
const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOC_PAGES: u32 = 250;
/// A donor must carry a real title for this share of the target's numbered
/// chapters before any name is adopted; an offset or partial list is rejected.
const TITLE_ENRICHMENT_MIN_COVERAGE: f64 = 0.95;
/// Bound the donor lookup. The source's own index still returns on timeout; it
/// simply keeps its numeric titles instead of delaying the picker forever.
const TITLE_ENRICHMENT_BUDGET_SECS: u64 = 180;
/// NovelPhoenix publishes names for most of a table of contents but leaves an
/// early run numbered (Reverend Insanity: chapters 1–347, 379 of 2,334). One
/// stray numbered row is not worth a donor search; this share is.
const TITLE_ENRICHMENT_MIN_BARE_SHARE: f64 = 0.01;
/// ...and this many rows, so a two-chapter sample never triggers a search.
const TITLE_ENRICHMENT_MIN_BARE: usize = 2;
/// A chapter page is only read to recover a name when few rows need it. A
/// fully numbered 2,000-chapter source must not turn into a page crawl.
const TITLE_RECOVERY_MAX_PAGES: usize = 80;
/// Upper bound for the recovery pass; whatever it reads is kept even if the
/// budget expires mid-book.
const TITLE_RECOVERY_MAX_SECS: u64 = 120;
// Transport handles HTTP-level retries (including 429/503 and Retry-After).
// The outer loop only retries a page whose response was readable but yielded
// no chapter links.
const TOC_ATTEMPTS: usize = 2;
/// A complete chapter index is stable for hours; reuse it across the picker and
/// download jobs instead of re-walking every TOC page.
const INDEX_CACHE_TTL_SECONDS: u64 = 6 * 60 * 60;

pub fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(transport::USER_AGENT)
        .timeout(PAGE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .map_err(|error| AppError::msg(format!("Could not build HTTP client: {error}")))
}

fn validate_url(value: &str) -> AppResult<Url> {
    let url =
        Url::parse(value.trim()).map_err(|_| AppError::msg("That is not a valid source URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::msg("Only http(s) source URLs are supported"));
    }
    Ok(url)
}

fn source_label(url: &Url) -> String {
    url.host_str()
        .unwrap_or("source")
        .trim_start_matches("www.")
        .to_string()
}

pub async fn fetch_novel_metadata(raw_url: &str) -> AppResult<NovelMetadata> {
    let (client, url, html, parser, mut metadata) = fetch_novel_page(raw_url).await?;

    let (cover, chapter_count) = tokio::join!(
        fetch_cover(&client, metadata_cover_url(&url, &html, parser)),
        async {
            if parser == ParserKind::NovelPhoenix && metadata.chapter_count.is_none() {
                novelphoenix_chapter_count(&client, &url, &html).await
            } else {
                None
            }
        }
    );
    if metadata.cover.is_none() {
        metadata.cover = cover;
    }
    if metadata.chapter_count.is_none() {
        metadata.chapter_count = chapter_count;
    }

    Ok(metadata)
}

/// Title/author/alias summary without the cover download. The resolver uses
/// this to verify authors and synonyms cheaply; cover bytes are unnecessary
/// there.
pub async fn fetch_novel_summary(raw_url: &str) -> AppResult<NovelMetadata> {
    let (_, _, _, _, metadata) = fetch_novel_page(raw_url).await?;
    Ok(metadata)
}

/// One novel row from a source author page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorPageNovel {
    pub title: String,
    pub url: String,
}

/// A source author page: every novel the site credits to that author.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorPage {
    /// Author name exactly as the page states it, when it states one.
    pub author: Option<String>,
    /// Canonical URL declared by the page. Sites answer an unknown author slug
    /// with their home page, so the canonical path proves where we landed.
    pub canonical: Option<String>,
    pub novels: Vec<AuthorPageNovel>,
}

/// Reads a source author page. Sites list novels under an author much more
/// completely than their title search does, so the resolver uses this instead
/// of guessing authors from search hits.
pub async fn fetch_author_page(raw_url: &str) -> AppResult<AuthorPage> {
    let url = validate_url(raw_url)?;
    let client = http_client()?;
    let html = fetch_html_kind(&client, url.as_str(), RequestKind::Metadata).await?;
    Ok(parse_author_page(&url, &html))
}

fn parse_author_page(page: &Url, html: &str) -> AuthorPage {
    let document = Html::parse_document(html);
    let parser = sources::for_host(page.host_str().unwrap_or_default()).parser;
    let mut novels = author_page_jsonld(html, page);
    if novels.is_empty() {
        novels = author_page_list(&document, page, parser);
    }
    let mut seen = HashSet::new();
    novels.retain(|novel| seen.insert(novel.url.clone()));
    novels.truncate(80);
    AuthorPage {
        author: author_page_name(html, &document),
        canonical: attr_value(&document, r#"link[rel="canonical"]"#, "href"),
        novels,
    }
}

/// Machine-readable list first: `ItemList` is the one page structure that
/// cannot be confused with the sidebar recommendations.
fn author_page_jsonld(html: &str, page: &Url) -> Vec<AuthorPageNovel> {
    let mut novels = Vec::new();
    for value in json_ld_values(html) {
        collect_item_list(&value, page, &mut novels);
    }
    novels
}

fn collect_item_list(value: &Value, page: &Url, out: &mut Vec<AuthorPageNovel>) {
    match value {
        Value::Object(map) => {
            let is_list = match map.get("@type") {
                Some(Value::String(kind)) => kind == "ItemList",
                Some(Value::Array(kinds)) => {
                    kinds.iter().any(|kind| kind.as_str() == Some("ItemList"))
                }
                _ => false,
            };
            if is_list && let Some(Value::Array(items)) = map.get("itemListElement") {
                for item in items {
                    let name = item.get("name").and_then(Value::as_str).or_else(|| {
                        item.get("item")
                            .and_then(|entry| entry.get("name"))
                            .and_then(Value::as_str)
                    });
                    let href = item
                        .get("url")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            item.get("item")
                                .and_then(|entry| entry.get("url"))
                                .and_then(Value::as_str)
                        })
                        .or_else(|| item.get("item").and_then(Value::as_str));
                    let (Some(name), Some(href)) = (name, href) else {
                        continue;
                    };
                    let name = clean_text(name);
                    if name.is_empty() {
                        continue;
                    }
                    let Ok(url) = page.join(href) else {
                        continue;
                    };
                    if url.host_str() != page.host_str() || url.path().trim_matches('/').is_empty()
                    {
                        continue;
                    }
                    out.push(AuthorPageNovel {
                        title: name,
                        url: url.to_string(),
                    });
                }
            }
            for nested in map.values() {
                collect_item_list(nested, page, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_item_list(item, page, out);
            }
        }
        _ => {}
    }
}

fn author_page_list(document: &Html, page: &Url, parser: ParserKind) -> Vec<AuthorPageNovel> {
    let mut novels = Vec::new();
    match parser {
        ParserKind::NovelFire => {
            // The first `ul.novel-list` is the author's own shelf; later lists
            // on the same page are "You may also Like" recommendations.
            if let Ok(selector) = Selector::parse("ul.novel-list")
                && let Some(list) = document.select(&selector).next()
            {
                collect_author_anchors(list, page, &mut novels);
            }
        }
        ParserKind::NovelPhoenix => {
            if let Ok(selector) = Selector::parse("h3.tit a[href], .novel-list a[href]") {
                for element in document.select(&selector) {
                    push_author_anchor(element, page, &mut novels);
                }
            }
        }
        _ => {
            if let Ok(selector) = Selector::parse("main a[href][title]") {
                for element in document.select(&selector) {
                    push_author_anchor(element, page, &mut novels);
                }
            }
        }
    }
    novels
}

fn collect_author_anchors(root: ElementRef<'_>, page: &Url, out: &mut Vec<AuthorPageNovel>) {
    if let Ok(selector) = Selector::parse("a[href]") {
        for element in root.select(&selector) {
            push_author_anchor(element, page, out);
        }
    }
}

fn push_author_anchor(element: ElementRef<'_>, page: &Url, out: &mut Vec<AuthorPageNovel>) {
    let Some(href) = element.value().attr("href") else {
        return;
    };
    let Ok(url) = page.join(href) else {
        return;
    };
    if url.host_str() != page.host_str() || url.path() == page.path() {
        return;
    }
    let path = url.path().to_ascii_lowercase();
    if [
        "/author",
        "/search",
        "/genre",
        "/tag",
        "/page",
        "/login",
        "/register",
        "/bookmark",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
    {
        return;
    }
    let title = element
        .value()
        .attr("title")
        .map(clean_text)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let heading = Selector::parse("h1, h2, h3, h4").ok()?;
            element
                .select(&heading)
                .next()
                .map(|heading| clean_text(&heading.text().collect::<String>()))
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            let text = clean_text(&element.text().collect::<String>());
            (!text.is_empty() && text.chars().count() <= 160).then_some(text)
        });
    let Some(title) = title else {
        return;
    };
    out.push(AuthorPageNovel {
        title,
        url: url.to_string(),
    });
}

/// Reads the author name from the breadcrumb, the page heading or the
/// collection metadata. Used to reject requests that landed on the wrong page.
fn author_page_name(html: &str, document: &Html) -> Option<String> {
    if let Ok(selector) = Selector::parse(r#"a[href*="/author/"]"#)
        && let Some(label) = document
            .select(&selector)
            .find_map(|element| {
                element
                    .value()
                    .attr("title")
                    .map(clean_text)
                    .or_else(|| Some(clean_text(&element.text().collect::<String>())))
            })
            .map(|value| strip_author_prefix(&value))
            .filter(|value| !value.is_empty() && value.chars().count() <= 120)
    {
        return Some(label);
    }
    if let Ok(selector) = Selector::parse("h1")
        && let Some(label) = document
            .select(&selector)
            .next()
            .map(|element| clean_text(&element.text().collect::<String>()))
            .map(|value| strip_author_suffix(&value))
            .filter(|value| !value.is_empty() && value.chars().count() <= 120)
    {
        return Some(label);
    }
    for value in json_ld_values(html) {
        if let Some(label) = collection_page_name(&value) {
            return Some(label);
        }
    }
    None
}

fn strip_author_prefix(value: &str) -> String {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("author:") || lower.starts_with("author：") {
        trimmed[7..].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn strip_author_suffix(value: &str) -> String {
    let mut text = value.trim().to_string();
    for suffix in [
        " author novels",
        " novels",
        " | novelphoenix",
        " | novelfire",
    ] {
        if text.to_ascii_lowercase().ends_with(suffix) {
            text.truncate(text.len() - suffix.len());
            break;
        }
    }
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("novels by ") {
        text["novels by ".len()..].trim().to_string()
    } else {
        text.to_string()
    }
}

fn collection_page_name(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            let is_collection = match map.get("@type") {
                Some(Value::String(kind)) => kind == "CollectionPage",
                Some(Value::Array(kinds)) => kinds
                    .iter()
                    .any(|kind| kind.as_str() == Some("CollectionPage")),
                _ => false,
            };
            if is_collection && let Some(name) = map.get("name").and_then(Value::as_str) {
                let label = strip_author_suffix(&clean_text(name));
                if !label.is_empty() && label.chars().count() <= 120 {
                    return Some(label);
                }
            }
            map.values().find_map(collection_page_name)
        }
        Value::Array(items) => items.iter().find_map(collection_page_name),
        _ => None,
    }
}

async fn fetch_novel_page(
    raw_url: &str,
) -> AppResult<(reqwest::Client, Url, String, ParserKind, NovelMetadata)> {
    let url = validate_url(raw_url)?;
    let client = http_client()?;
    let html = fetch_html_kind(&client, url.as_str(), RequestKind::Metadata).await?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let parser = sources::for_host(&host).parser;

    let mut metadata = match parser {
        ParserKind::NovelFire => parse_novelfire_metadata(&url, &html),
        ParserKind::NovelPhoenix => parse_novelphoenix_metadata(&url, &html),
        ParserKind::NovelFull => parse_novelfull_metadata(&url, &html),
        ParserKind::Generic => parse_generic_metadata(&url, &html),
    };

    if metadata.title.trim().is_empty() {
        metadata.title = crate::resolver::title_from_source(&url);
    }
    if metadata.language.is_none() {
        metadata.language = Some("en".to_string());
    }
    metadata.aliases = parse_aliases(&html, &Html::parse_document(&html), &metadata.title);

    Ok((client, url, html, parser, metadata))
}

async fn fetch_html(client: &reqwest::Client, url: &str) -> AppResult<String> {
    fetch_html_kind(client, url, RequestKind::Toc).await
}

async fn fetch_html_kind(
    client: &reqwest::Client,
    url: &str,
    kind: RequestKind,
) -> AppResult<String> {
    let response = transport::fetch(client, url, kind).await?;
    Ok(response.text())
}

fn meta_content(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .find_map(|element| element.value().attr("content"))
        .map(clean_text)
        .filter(|value| !value.is_empty())
}

fn first_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| clean_text(&element.text().collect::<String>()))
        .filter(|value| !value.is_empty())
}

fn attr_value(document: &Html, selector: &str, attr: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .find_map(|element| element.value().attr(attr))
        .map(clean_text)
        .filter(|value| !value.is_empty())
}

fn parse_novelfire_metadata(url: &Url, html: &str) -> NovelMetadata {
    let document = Html::parse_document(html);
    let mut metadata = NovelMetadata {
        title: first_text(&document, "h1.novel-title")
            .or_else(|| meta_content(&document, r#"meta[property="og:title"]"#))
            .unwrap_or_default(),
        source_url: url.to_string(),
        source: source_label(url),
        language: Some("en".to_string()),
        ..Default::default()
    };

    let author_selector = Selector::parse(r#"span[itemprop="author"]"#).expect("valid selector");
    metadata.authors = unique_values(
        document
            .select(&author_selector)
            .map(|element| clean_text(&element.text().collect::<String>())),
    );
    metadata.author = metadata.authors.first().cloned();

    let tag_selector = Selector::parse(".categories li a").expect("valid selector");
    metadata.tags = unique_values(
        document
            .select(&tag_selector)
            .map(|element| clean_text(&element.text().collect::<String>())),
    );
    metadata.tags.truncate(24);

    metadata.synopsis = meta_content(&document, r#"meta[itemprop="description"]"#)
        .or_else(|| meta_content(&document, r#"meta[name="description"]"#))
        .or_else(|| first_text(&document, ".description"))
        .filter(|value| value.len() > 20);

    let stats_selector = Selector::parse(".header-stats span").expect("valid selector");
    for span in document.select(&stats_selector) {
        let small = first_child_text(&span, "small");
        let strong = first_child_text(&span, "strong");
        match small.as_deref() {
            Some("Chapters") => {
                metadata.chapter_count = strong.as_deref().and_then(parse_number);
            }
            Some("Status") => {
                if let Some(value) = strong.filter(|value| !value.is_empty()) {
                    metadata.status = Some(value);
                }
            }
            _ => {}
        }
    }

    if metadata.chapter_count.is_none() {
        // Some mirrors move the stat block; fall back to a "2334 Chapters" text scan.
        let fallback = Regex::new(r"(?i)([\d,]+)\s*chapters").expect("valid regex");
        if let Some(captures) = fallback.captures(html) {
            metadata.chapter_count = captures
                .get(1)
                .and_then(|value| parse_number(value.as_str()));
        }
    }

    metadata
}

fn parse_novelphoenix_metadata(url: &Url, html: &str) -> NovelMetadata {
    let document = Html::parse_document(html);
    let json = json_ld(html);
    let mut metadata = NovelMetadata {
        title: first_text(&document, "h1.tit")
            .or_else(|| {
                json.as_ref()
                    .and_then(|value| json_value_string(value, "name"))
            })
            .or_else(|| meta_content(&document, r#"meta[property="og:title"]"#))
            .unwrap_or_default(),
        source_url: url.to_string(),
        source: source_label(url),
        ..Default::default()
    };

    if let Some(value) = &json {
        metadata.authors = json_authors(value);
        metadata.tags = json_strings(value, "genre");
        metadata.status = json_value_string(value, "creativeWorkStatus");
        metadata.language = json_value_string(value, "inLanguage");
        metadata.synopsis = json_value_string(value, "description");
    }
    metadata.author = metadata.authors.first().cloned();

    if metadata.authors.is_empty() {
        let author_selector = Selector::parse(".m-imgtxt .item a.a1").expect("valid selector");
        metadata.authors = unique_values(
            document
                .select(&author_selector)
                .map(|element| clean_text(&element.text().collect::<String>()))
                .filter(|value| value.len() < 80),
        )
        .into_iter()
        .take(3)
        .collect();
        metadata.author = metadata.authors.first().cloned();
    }

    if metadata.tags.is_empty() {
        let tag_selector = Selector::parse(r#"a[href^="/genre/"]"#).expect("valid selector");
        metadata.tags = unique_values(
            document
                .select(&tag_selector)
                .map(|element| clean_text(&element.text().collect::<String>())),
        )
        .into_iter()
        .take(24)
        .collect();
    }

    if metadata.synopsis.is_none() {
        metadata.synopsis = first_text(&document, "#novel-summary-inner")
            .or_else(|| meta_content(&document, r#"meta[itemprop="description"]"#));
    }
    metadata.synopsis = metadata.synopsis.filter(|value| value.len() > 20);

    // The JSON-LD block does not expose a chapter count. The latest chapter
    // number is visible on the page and is a useful fallback before the TOC
    // pagination helper runs.
    if metadata.chapter_count.is_none() {
        metadata.chapter_count = latest_chapter_number(&document);
    }

    metadata
}

fn parse_novelfull_metadata(url: &Url, html: &str) -> NovelMetadata {
    let document = Html::parse_document(html);
    let mut metadata = NovelMetadata {
        // NovelFull's og:title carries a `| NovelFull` suffix while `h1.tit`
        // is clean, so prefer the heading and only sanitise the meta fallback.
        title: first_text(&document, "h1.tit")
            .or_else(|| {
                meta_content(&document, r#"meta[property="og:title"]"#)
                    .map(|value| strip_site_suffix(&value, "novelfull"))
            })
            .unwrap_or_default(),
        source_url: url.to_string(),
        source: source_label(url),
        language: Some("en".to_string()),
        ..Default::default()
    };

    // The author block links every credited name (native and romanised), so
    // collect the links rather than relying on author meta tags.
    let author_selector = Selector::parse(r#"a[href^="/author/"]"#).expect("valid selector");
    metadata.authors = unique_values(
        document
            .select(&author_selector)
            .map(|element| clean_text(&element.text().collect::<String>()))
            .filter(|value| value.len() < 80),
    )
    .into_iter()
    .take(8)
    .collect();
    metadata.author = metadata.authors.first().cloned();

    let tag_selector = Selector::parse(r#"a[href^="/genre/"]"#).expect("valid selector");
    metadata.tags = unique_values(
        document
            .select(&tag_selector)
            .map(|element| clean_text(&element.text().collect::<String>()))
            .filter(|value| !value.is_empty() && value.len() < 40),
    )
    .into_iter()
    .take(24)
    .collect();

    metadata.status = first_text(&document, r#"a[href^="/latest-release-novel"]"#);

    metadata.synopsis = first_text(&document, ".m-desc > div.txt")
        .or_else(|| meta_content(&document, r#"meta[name="description"]"#))
        .or_else(|| meta_content(&document, r#"meta[property="og:description"]"#))
        .filter(|value| value.len() > 20);

    metadata.chapter_count = attr_value(
        &document,
        "#list-chapter[data-total-chapters]",
        "data-total-chapters",
    )
    .and_then(|value| parse_number(&value))
    .or_else(|| latest_chapter_number(&document));

    metadata
}

fn parse_generic_metadata(url: &Url, html: &str) -> NovelMetadata {
    let document = Html::parse_document(html);
    let mut metadata = NovelMetadata {
        title: meta_content(&document, r#"meta[property="og:title"]"#)
            .or_else(|| first_text(&document, "h1"))
            .or_else(|| first_text(&document, "title"))
            .unwrap_or_default(),
        source_url: url.to_string(),
        source: source_label(url),
        ..Default::default()
    };

    metadata.synopsis = meta_content(&document, r#"meta[property="og:description"]"#)
        .or_else(|| meta_content(&document, r#"meta[name="description"]"#));
    metadata.author = meta_content(&document, r#"meta[name="author"]"#);
    metadata.authors = metadata.author.iter().cloned().collect();
    if metadata.authors.is_empty() {
        // JSON-LD carries one entry per author, while `og:novel:author` often
        // packs every name into one comma-separated string.
        metadata.authors = json_ld_authors(html);
    }
    if metadata.authors.is_empty() {
        metadata.authors = meta_content(&document, r#"meta[property="og:author"]"#)
            .or_else(|| meta_content(&document, r#"meta[property="og:novel:author"]"#))
            .or_else(|| attr_value(&document, r#"[itemprop="author"]"#, "content"))
            .or_else(|| first_text(&document, r#"[itemprop="author"]"#))
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect();
    }
    metadata.author = metadata.authors.first().cloned();

    let tag_selector =
        Selector::parse(r#"[rel="tag"], .tags a, a[href*="/genre/"]"#).expect("valid selector");
    metadata.tags = unique_values(
        document
            .select(&tag_selector)
            .map(|element| clean_text(&element.text().collect::<String>()))
            .filter(|value| !value.is_empty() && value.len() < 40),
    )
    .into_iter()
    .take(16)
    .collect();

    metadata
}

fn metadata_cover_url(url: &Url, html: &str, parser: ParserKind) -> Option<String> {
    let document = Html::parse_document(html);
    let raw = match parser {
        ParserKind::NovelFire => attr_value(&document, ".cover img", "src")
            .or_else(|| meta_content(&document, r#"meta[property="og:image"]"#)),
        ParserKind::NovelPhoenix => json_ld(html)
            .and_then(|json| json_value_string(&json, "image"))
            .or_else(|| attr_value(&document, ".m-imgtxt .pic img", "src"))
            .or_else(|| meta_content(&document, r#"meta[property="og:image"]"#)),
        ParserKind::NovelFull => meta_content(&document, r#"meta[property="og:image"]"#)
            .or_else(|| meta_content(&document, r#"meta[name="twitter:image"]"#)),
        ParserKind::Generic => meta_content(&document, r#"meta[property="og:image"]"#)
            .or_else(|| meta_content(&document, r#"meta[name="twitter:image"]"#)),
    }?;
    url.join(&raw)
        .ok()
        .map(|value| value.to_string())
        .or(Some(raw))
}

async fn fetch_cover(client: &reqwest::Client, raw_url: Option<String>) -> Option<String> {
    let raw_url = raw_url?;
    let url = Url::parse(&raw_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    let cache = cover_cache_path(&raw_url);
    if let Ok(bytes) = std::fs::read(&cache)
        && bytes.len() > 512
    {
        return data_url_for(&bytes);
    }

    let response = transport::fetch(client, url.as_str(), RequestKind::Cover)
        .await
        .ok()?;
    let bytes = response.body;
    if bytes.len() < 512 || bytes.len() > MAX_COVER_BYTES {
        return None;
    }
    let _ = std::fs::create_dir_all(cache.parent()?);
    let _ = std::fs::write(&cache, &bytes);
    data_url_for(&bytes)
}

fn cover_cache_path(url: &str) -> PathBuf {
    let extension = if url.to_ascii_lowercase().contains(".png") {
        "png"
    } else if url.to_ascii_lowercase().contains(".webp") {
        "webp"
    } else if url.to_ascii_lowercase().contains(".gif") {
        "gif"
    } else {
        "jpg"
    };
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in url.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    cover_dir().join(format!("{hash:016x}.{extension}"))
}

fn cover_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("novelforge")
        .join("covers")
}

pub(crate) fn data_url_for(bytes: &[u8]) -> Option<String> {
    let mime = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
        "image/png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        return None;
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

async fn novelphoenix_chapter_count(
    client: &reqwest::Client,
    url: &Url,
    html: &str,
) -> Option<u32> {
    let (last_page, first_count) = {
        let document = Html::parse_document(html);
        let last_page = last_page_number(&document).unwrap_or(1);
        let count = document
            .select(&Selector::parse("#idData li a").ok()?)
            .count();
        (last_page, count)
    };
    if last_page <= 1 {
        return u32::try_from(first_count).ok().filter(|value| *value > 0);
    }
    let page_url = page_url(url, last_page);
    let page_html = fetch_html(client, &page_url).await.ok()?;
    let page = Html::parse_document(&page_html);
    let last_count = page.select(&Selector::parse("#idData li a").ok()?).count() as u32;
    Some((last_page - 1) * 40 + last_count).filter(|value| *value > 0)
}

fn latest_chapter_number(document: &Html) -> Option<u32> {
    const SELECTORS: &[&str] = &[
        "#latest-chapter a",
        "a[rel=\"latest\"]",
        "[data-latest-chapter] a",
        ".l-chapter .item-value a",
        ".l-chapter a.chapter-title",
        ".latest-chapter a",
        ".chapter-latest a",
    ];
    for selector in SELECTORS {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        let number = document.select(&selector).find_map(|element| {
            let title = element
                .value()
                .attr("title")
                .map(clean_text)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| clean_text(&element.text().collect::<String>()));
            chapter_number_from_text(&title).or_else(|| {
                element
                    .value()
                    .attr("href")
                    .and_then(chapter_number_from_url)
            })
        });
        if number.is_some() {
            return number;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Chapter index
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexCacheEntry {
    key: String,
    fetched_at: u64,
    index: NovelChapterIndex,
}

fn index_cache_ttl_seconds() -> u64 {
    std::env::var("NOVELFORGE_INDEX_TTL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(INDEX_CACHE_TTL_SECONDS)
}

fn index_cache_dir() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("NOVELFORGE_INDEX_CACHE_DIR") {
        return Some(PathBuf::from(configured));
    }
    dirs::data_dir().map(|base| base.join("novelforge").join("index-cache"))
}

/// Serialises table-of-contents walks for one cache key. Held only for the
/// walk itself, never while title enrichment borrows another source's index.
fn index_walk_lock(key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<
        std::sync::Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(key.to_string())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default()
}

fn index_cache_path(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("{:016x}.json", cache::key_hash(key)))
}

fn read_cached_index_at(
    directory: &Path,
    key: &str,
    now: u64,
    ttl_seconds: u64,
) -> Option<NovelChapterIndex> {
    let bytes = std::fs::read(index_cache_path(directory, key)).ok()?;
    let entry: IndexCacheEntry = serde_json::from_slice(&bytes).ok()?;
    // The stored key guards against a hash collision returning the wrong book.
    if entry.key != key || !entry.index.complete {
        return None;
    }
    let age = now.saturating_sub(entry.fetched_at);
    if age > ttl_seconds {
        return None;
    }
    let mut index = entry.index;
    index.cached = true;
    index.cache_age_seconds = Some(age);
    // Transport metrics describe the request that produced this response, not
    // an older fetch stored in the cache.
    index.transport = None;
    Some(index)
}

fn write_cached_index_at(directory: &Path, key: &str, index: &NovelChapterIndex, now: u64) {
    // A truncated walk is not evidence and must never suppress a retry.
    if !index.complete {
        return;
    }
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    let entry = IndexCacheEntry {
        key: key.to_string(),
        fetched_at: now,
        index: index.clone(),
    };
    let Ok(json) = serde_json::to_vec(&entry) else {
        return;
    };
    let path = index_cache_path(directory, key);
    if let Ok(json) = String::from_utf8(json) {
        let _ = crate::fsutil::write_text_atomic(&path, &json);
    }
}

fn cached_index(key: &str) -> Option<NovelChapterIndex> {
    let directory = index_cache_dir()?;
    read_cached_index_at(
        &directory,
        key,
        now_unix_seconds(),
        index_cache_ttl_seconds(),
    )
}

fn store_cached_index(key: &str, index: &NovelChapterIndex) {
    if let Some(directory) = index_cache_dir() {
        write_cached_index_at(&directory, key, index, now_unix_seconds());
    }
}

fn strip_fragment(url: &Url) -> String {
    let mut clean = url.clone();
    clean.set_fragment(None);
    clean.to_string()
}

/// Cache identity is the source host plus the canonical table-of-contents URL,
/// never the title. Pagination and fragments are stripped so the chapter picker
/// and the download job land on the same entry.
fn index_cache_key(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let canonical = match sources::for_host(&host).parser {
        ParserKind::NovelFire => novelfire_toc_url(url)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| strip_fragment(url)),
        ParserKind::NovelPhoenix | ParserKind::NovelFull | ParserKind::Generic => {
            generic_toc_url(url).to_string()
        }
    };
    format!("{host}|{canonical}")
}

/// Reports `(page, pages)` while a paginated table of contents is walked.
///
/// The safe request gap makes a 3,000-chapter book take about a minute, so the
/// picker needs the page counter to prove the walk is moving rather than hung.
pub type IndexProgress<'a> = &'a (dyn Fn(u32, u32) + Send + Sync);

/// Index fetch with the app settings, so a source whose table of contents only
/// numbers its chapters can borrow real chapter names from a verified other
/// source.
pub async fn fetch_chapter_index_for(
    settings: &Settings,
    raw_url: &str,
    refresh: bool,
) -> AppResult<NovelChapterIndex> {
    fetch_chapter_index_with(settings, raw_url, refresh, true, None).await
}

/// Same as [`fetch_chapter_index_for`] but reports walk progress to the caller.
pub async fn fetch_chapter_index_reporting(
    settings: &Settings,
    raw_url: &str,
    refresh: bool,
    progress: IndexProgress<'_>,
) -> AppResult<NovelChapterIndex> {
    fetch_chapter_index_with(settings, raw_url, refresh, true, Some(progress)).await
}

/// NF-044: the catalogue rows without the title passes. A partial download
/// only needs the row order to translate the numbers the source shows into
/// lncrawl's 1-based serials. Borrowing names or opening chapter pages first
/// would delay a two-chapter crawl for no benefit; the export's own index fetch
/// enriches this cached list afterwards.
pub async fn fetch_chapter_numbers_for(
    settings: &Settings,
    raw_url: &str,
) -> AppResult<NovelChapterIndex> {
    fetch_chapter_index_with(settings, raw_url, false, false, None).await
}

async fn fetch_chapter_index_with(
    settings: &Settings,
    raw_url: &str,
    refresh: bool,
    allow_enrichment: bool,
    progress: Option<IndexProgress<'_>>,
) -> AppResult<NovelChapterIndex> {
    let url = validate_url(raw_url)?;
    let key = index_cache_key(&url);
    if !refresh && let Some(mut index) = cached_index(&key) {
        // A cache entry written before title lookup existed (or a source
        // whose TOC leaves rows numbered) is repaired in place. The TOC
        // itself is not re-walked; only the title passes run.
        if allow_enrichment && title_pass_needed(&index) {
            enrich_index_titles(settings, &mut index).await;
            let cached = index.cached;
            let age = index.cache_age_seconds;
            index.cached = false;
            index.cache_age_seconds = None;
            index.transport = None;
            store_cached_index(&key, &index);
            index.cached = cached;
            index.cache_age_seconds = age;
        }
        return Ok(index);
    }

    let client = http_client()?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();

    // A profile may declare that its index is unproven; refusing early is
    // clearer than returning 20 links and calling it complete.
    let profile = sources::for_host(&host);
    if !profile.capabilities.index {
        return Err(AppError::msg(format!(
            "{} chapter indexes are not supported yet; the source list could not be proven complete",
            profile.display_name
        )));
    }

    // One walk per book at a time. The transport gate spaces requests per
    // host, so a duplicate walk (picker still loading when the download
    // starts, or a second click on Refresh) does not run in parallel: it
    // doubles the wall clock and re-fetches every page. Waiters re-check the
    // cache below and return the finished index instead of walking again.
    let walk_lock = index_walk_lock(&key);
    let mut index = {
        let _walking = walk_lock.lock().await;
        // A waiter that arrived during the walk finds the finished cache here
        // and never starts a second walk of the same pages.
        match if refresh { None } else { cached_index(&key) } {
            Some(index) => index,
            None => match profile.parser {
                ParserKind::NovelFire => fetch_novelfire_index(&client, &url, progress).await,
                ParserKind::NovelPhoenix => fetch_novelphoenix_index(&client, &url, progress).await,
                ParserKind::NovelFull => fetch_generic_index(&client, &url, progress).await,
                ParserKind::Generic => fetch_generic_index(&client, &url, progress).await,
            }?,
        }
    };

    if allow_enrichment && title_pass_needed(&index) {
        enrich_index_titles(settings, &mut index).await;
    }

    if index.complete {
        store_cached_index(&key, &index);
    }
    index.transport = Some(transport::summary_for_url(url.as_str()));
    Ok(index)
}

async fn fetch_novelfire_index(
    client: &reqwest::Client,
    url: &Url,
    progress: Option<IndexProgress<'_>>,
) -> AppResult<NovelChapterIndex> {
    let toc_url = novelfire_toc_url(url)?;
    let first_html = fetch_html(client, toc_url.as_str()).await?;
    let (first_page, last_page) = {
        let first_document = Html::parse_document(&first_html);
        let first_page = parse_novelfire_chapters(&first_document, &toc_url, 1);
        let last_page = last_page_number(&first_document).unwrap_or(1);
        if last_page > MAX_TOC_PAGES {
            return Err(AppError::msg(format!(
                "This chapter list has {last_page} pages, which exceeds the {MAX_TOC_PAGES}-page safety limit"
            )));
        }
        (first_page, last_page)
    };

    if let Some(report) = progress {
        report(1, last_page.max(1));
    }
    let mut pages: Vec<(u32, Vec<NovelChapter>)> = vec![(1, first_page)];
    if last_page > 1 {
        let page_numbers: Vec<u32> = (2..=last_page).collect();
        let base = toc_url.clone();
        let (results, failed) = fetch_toc_pages(
            client,
            base,
            page_numbers,
            parse_novelfire_chapters,
            progress,
            last_page,
        )
        .await;
        if !failed.is_empty() {
            return Err(AppError::msg(format!(
                "Chapter index incomplete: {} of {} table-of-contents page(s) could not be fetched ({}). Retry the source.",
                failed.len(),
                last_page,
                preview_pages(&failed)
            )));
        }
        pages.extend(results);
    }
    pages.sort_by_key(|(page, _)| *page);

    let chapters = dedupe_chapters(pages.into_iter().flat_map(|(_, list)| list).collect());
    if chapters.is_empty() {
        return Err(AppError::msg(
            "No chapter links were found on the table-of-contents page. Retry the source.",
        ));
    }
    Ok(NovelChapterIndex {
        source_url: url.to_string(),
        source: source_label(url),
        title: None,
        total: chapters.len() as u32,
        complete: true,
        detail: Some(format!(
            "{} chapters from {} table-of-contents page(s); last page reached.",
            chapters.len(),
            last_page
        )),
        chapters,
        cached: false,
        cache_age_seconds: None,
        transport: None,
        title_source: None,
        title_lookup_attempted: false,
        title_recovery_attempted: false,
    })
}

fn novelfire_toc_url(url: &Url) -> AppResult<Url> {
    let mut clean = url.clone();
    clean.set_query(None);
    clean.set_fragment(None);
    let path = clean.path().to_string();
    let book_path = if let Some(index) = path.find("/chapter-") {
        path[..index].to_string()
    } else {
        path.trim_end_matches('/').to_string()
    };
    if book_path.is_empty() {
        return Err(AppError::msg("This NovelFire URL has no book path"));
    }
    let mut toc = clean;
    toc.set_path(&format!("{book_path}/chapters"));
    Ok(toc)
}

async fn fetch_novelphoenix_index(
    client: &reqwest::Client,
    url: &Url,
    progress: Option<IndexProgress<'_>>,
) -> AppResult<NovelChapterIndex> {
    let mut clean = url.clone();
    clean.set_fragment(None);
    let first_html = fetch_html(client, clean.as_str()).await?;
    let (first_page, last_page) = {
        let first_document = Html::parse_document(&first_html);
        let first_page = parse_novelphoenix_chapters(&first_document, &clean, 1);
        let last_page = last_page_number(&first_document).unwrap_or(1);
        if last_page > MAX_TOC_PAGES {
            return Err(AppError::msg(format!(
                "This chapter list has {last_page} pages, which exceeds the {MAX_TOC_PAGES}-page safety limit"
            )));
        }
        (first_page, last_page)
    };

    if let Some(report) = progress {
        report(1, last_page.max(1));
    }
    let mut pages: Vec<(u32, Vec<NovelChapter>)> = vec![(1, first_page)];
    if last_page > 1 {
        let page_numbers: Vec<u32> = (2..=last_page).collect();
        let base = clean.clone();
        let (results, failed) = fetch_toc_pages(
            client,
            base,
            page_numbers,
            parse_novelphoenix_chapters,
            progress,
            last_page,
        )
        .await;
        if !failed.is_empty() {
            return Err(AppError::msg(format!(
                "Chapter index incomplete: {} of {} table-of-contents page(s) could not be fetched ({}). Retry the source.",
                failed.len(),
                last_page,
                preview_pages(&failed)
            )));
        }
        pages.extend(results);
    }
    pages.sort_by_key(|(page, _)| *page);

    let chapters = dedupe_chapters(pages.into_iter().flat_map(|(_, list)| list).collect());
    if chapters.is_empty() {
        return Err(AppError::msg(
            "No chapter links were found on the table-of-contents page. Retry the source.",
        ));
    }
    Ok(NovelChapterIndex {
        source_url: url.to_string(),
        source: source_label(url),
        title: None,
        total: chapters.len() as u32,
        complete: true,
        detail: Some(format!(
            "{} chapters from {} table-of-contents page(s); last page reached.",
            chapters.len(),
            last_page
        )),
        chapters,
        cached: false,
        cache_age_seconds: None,
        transport: None,
        title_source: None,
        title_lookup_attempted: false,
        title_recovery_attempted: false,
    })
}

/// What the source itself says about the size and shape of its chapter list.
#[derive(Debug, Default)]
struct GenericTocSignals {
    advertised_total: Option<u32>,
    latest_chapter: Option<u32>,
    json_ld_total: Option<u32>,
    json_ld_chapters: Vec<NovelChapter>,
    pagination: PaginationSignals,
    ajax: Option<AjaxArchive>,
}

#[derive(Debug, Default, Clone)]
struct PaginationSignals {
    total_pages: Option<u32>,
    page_template: Option<String>,
    next_url: Option<String>,
    detected: bool,
}

#[derive(Debug, Clone)]
struct AjaxArchive {
    endpoint: String,
    novel_id: String,
    total_pages: u32,
    paged: bool,
}

#[derive(Debug, Default)]
struct PaginationOutcome {
    expected_pages: Option<u32>,
    walked_pages: u32,
    reached_terminal: bool,
    limit_exceeded: bool,
}

struct FetchedTocPage {
    html: String,
    url: Url,
    chapters: Vec<NovelChapter>,
}

async fn fetch_generic_index(
    client: &reqwest::Client,
    url: &Url,
    progress: Option<IndexProgress<'_>>,
) -> AppResult<NovelChapterIndex> {
    let toc_url = generic_toc_url(url);
    let html = fetch_html(client, toc_url.as_str()).await?;
    let (signals, mut chapters) = {
        let document = Html::parse_document(&expand_templates(&html));
        let signals = generic_toc_signals(&document, &html, &toc_url);
        let chapters = parse_generic_chapters(&document, &toc_url, 1);
        // `scraper::Html` is not `Send`; keep the parsed DOM inside this block
        // so the returned future stays spawnable across the TOC awaits.
        (signals, chapters)
    };
    let mut pagination = PaginationOutcome {
        expected_pages: signals.pagination.total_pages,
        walked_pages: 1,
        ..Default::default()
    };
    let mut failed_pages = Vec::new();

    if let Some(expected) = signals.pagination.total_pages {
        if expected > MAX_TOC_PAGES {
            pagination.limit_exceeded = true;
        } else {
            let template = signals
                .pagination
                .page_template
                .clone()
                .unwrap_or_else(|| default_page_template(&toc_url));
            if let Some(report) = progress {
                report(1, expected);
            }
            for page in 2..=expected {
                let page_url = page_template_url(&template, page);
                match fetch_generic_page(client, &page_url, &toc_url, page).await {
                    Some(fetched) => {
                        chapters.extend(fetched.chapters);
                        pagination.walked_pages += 1;
                        if let Some(report) = progress {
                            report(page, expected);
                        }
                    }
                    None => failed_pages.push(page),
                }
            }
            if failed_pages.is_empty() {
                pagination.reached_terminal = true;
            }
        }
    } else if signals.pagination.next_url.is_some() {
        let mut cursor = signals.pagination.next_url.clone();
        let mut visited = HashSet::new();
        while let Some(next) = cursor {
            if pagination.walked_pages >= MAX_TOC_PAGES {
                pagination.limit_exceeded = true;
                break;
            }
            if !visited.insert(next.clone()) {
                pagination.reached_terminal = true;
                break;
            }
            let page = pagination.walked_pages + 1;
            match fetch_generic_page(client, &next, &toc_url, page).await {
                Some(fetched) => {
                    chapters.extend(fetched.chapters);
                    pagination.walked_pages += 1;
                    cursor = {
                        let document = Html::parse_document(&expand_templates(&fetched.html));
                        next_page_url(&document, &fetched.url)
                    };
                    if cursor.is_none() {
                        pagination.reached_terminal = true;
                    }
                }
                None => {
                    failed_pages.push(page);
                    break;
                }
            }
        }
    } else if signals.pagination.detected {
        pagination.reached_terminal = signals.pagination.total_pages == Some(1);
    }

    let expected_total = signals.advertised_total.or(signals.json_ld_total);
    let html_listed = chapters.len() as u32;
    let html_shortfall = expected_total.is_some_and(|expected| html_listed < expected);
    let latest_shortfall = signals.latest_chapter.is_some_and(|latest| {
        chapters
            .iter()
            .map(|chapter| chapter.number)
            .max()
            .unwrap_or(0)
            < latest
    });
    let needs_ajax = signals.ajax.is_some()
        && (chapters.is_empty()
            || html_shortfall
            || latest_shortfall
            || !failed_pages.is_empty()
            || (expected_total.is_none()
                && signals.latest_chapter.is_none()
                && !signals.pagination.detected));

    let mut ajax_failed_pages = Vec::new();
    let mut ajax_error: Option<String> = None;
    let mut ajax_used = false;
    if needs_ajax && let Some(archive) = signals.ajax.as_ref() {
        match fetch_ajax_chapters(client, &toc_url, archive).await {
            Ok((archive_chapters, failed, fetched_pages)) => {
                ajax_used = fetched_pages > 0;
                chapters.extend(archive_chapters);
                ajax_failed_pages = failed;
                if ajax_failed_pages.is_empty() {
                    // The archive is the source's canonical list, so a
                    // complete archive fetch supersedes failed HTML TOC
                    // pages from the same book.
                    failed_pages.clear();
                    if archive.paged {
                        if archive.total_pages > MAX_TOC_PAGES {
                            pagination.limit_exceeded = true;
                        } else if fetched_pages >= archive.total_pages {
                            pagination.reached_terminal = true;
                            pagination.expected_pages = Some(archive.total_pages);
                            pagination.walked_pages = pagination.walked_pages.max(fetched_pages);
                        }
                    }
                }
            }
            Err(error) => ajax_error = Some(error),
        }
    }

    chapters.extend(signals.json_ld_chapters.clone());
    let mut chapters = dedupe_chapters(chapters);
    chapters.sort_by_key(|chapter| chapter.number);
    if chapters.is_empty() {
        return Err(AppError::msg(
            "No chapter links were found on this page. Paste the novel's table-of-contents URL.",
        ));
    }

    let numbers: HashSet<u32> = chapters.iter().map(|chapter| chapter.number).collect();
    let max_number = numbers.iter().copied().max();
    let contiguous_from_one = numbers.contains(&1)
        && max_number.is_some_and(|max| (1..=max).all(|number| numbers.contains(&number)));

    let listed = chapters.len() as u32;
    let (complete, evidence) = assess_generic_index(
        listed,
        html_listed,
        max_number,
        contiguous_from_one,
        signals.advertised_total,
        signals.json_ld_total,
        signals.latest_chapter,
        &pagination,
        signals.pagination.detected,
        &failed_pages,
        &ajax_failed_pages,
        ajax_error.as_deref(),
        ajax_used,
    );

    let mut detail = format!("{listed} chapter(s)");
    if pagination.walked_pages > 1 {
        detail.push_str(&format!(
            " from {} table-of-contents page(s)",
            pagination.walked_pages
        ));
    }
    if ajax_used {
        detail.push_str("; source chapter archive queried");
    }
    detail.push_str(&format!("; {}.", evidence.trim_end_matches('.')));

    Ok(NovelChapterIndex {
        source_url: url.to_string(),
        source: source_label(url),
        title: None,
        total: listed,
        complete,
        detail: Some(detail),
        chapters,
        cached: false,
        cache_age_seconds: None,
        transport: None,
        title_source: None,
        title_lookup_attempted: false,
        title_recovery_attempted: false,
    })
}

fn generic_toc_url(url: &Url) -> Url {
    let mut clean = url.clone();
    clean.set_fragment(None);
    let pagination_keys = ["page", "paged"];
    let kept = {
        clean
            .query_pairs()
            .filter(|(key, _)| {
                !pagination_keys
                    .iter()
                    .any(|name| key.eq_ignore_ascii_case(name))
            })
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>()
    };
    if clean.query_pairs().any(|(key, _)| {
        pagination_keys
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
    }) {
        clean.set_query(None);
        if !kept.is_empty() {
            let mut pairs = clean.query_pairs_mut();
            for (key, value) in kept {
                pairs.append_pair(&key, &value);
            }
        }
    }
    clean
}

fn expand_templates(html: &str) -> String {
    if !html.contains("<template") {
        return html.to_string();
    }
    static TEMPLATE: OnceLock<Regex> = OnceLock::new();
    let regex = TEMPLATE.get_or_init(|| {
        Regex::new(r"(?is)<template\b[^>]*>(.*?)</template>").expect("valid template regex")
    });
    regex.replace_all(html, "$1").into_owned()
}

fn generic_toc_signals(document: &Html, html: &str, base: &Url) -> GenericTocSignals {
    let (json_ld_chapters, json_ld_total) = json_ld_chapter_list(html, base);
    GenericTocSignals {
        advertised_total: advertised_chapter_total(document, json_ld_total),
        latest_chapter: latest_chapter_number(document),
        json_ld_total,
        json_ld_chapters,
        pagination: pagination_signals(document, base),
        ajax: discover_ajax_archive(document, html, base),
    }
}

fn parse_generic_chapters(document: &Html, base: &Url, page: u32) -> Vec<NovelChapter> {
    const CONTAINERS: &[&str] = &[
        "#idData a[href]",
        "#chapter-archive a[href]",
        "[data-chapter-archive-grid] a[href]",
        "[data-chapter-item] a[href]",
        ".chapter-group__list a[href]",
        "ul.chapter-list a[href]",
        ".chapter-list a[href]",
        "#list-chapter a[href]",
        "#chapter-list a[href]",
        ".chapters a[href]",
        ".chapter a[href]",
        "[data-chapter-list] a[href]",
        "table#chapters a[href]",
        "table.chapters a[href]",
        ".eplister a[href]",
    ];

    let mut best = Vec::new();
    for selector in CONTAINERS {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        let parsed = collect_chapter_anchors(document.select(&selector), base, page);
        if parsed.len() > best.len() {
            best = parsed;
        }
    }

    if best.is_empty() {
        let Ok(selector) = Selector::parse("a[href]") else {
            return Vec::new();
        };
        best = collect_chapter_anchors(
            document.select(&selector).filter(|anchor| {
                anchor
                    .value()
                    .attr("href")
                    .is_some_and(|href| href.to_ascii_lowercase().contains("chapter"))
            }),
            base,
            page,
        );
    }

    best
}

fn collect_chapter_anchors<'a>(
    anchors: impl Iterator<Item = ElementRef<'a>>,
    base: &Url,
    page: u32,
) -> Vec<NovelChapter> {
    let mut chapters = Vec::new();
    let mut seen = HashSet::new();
    for (index, anchor) in anchors.enumerate() {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        if !is_chapter_href(href) {
            continue;
        }
        let Ok(mut absolute) = base.join(href) else {
            continue;
        };
        absolute.set_fragment(None);
        let url = absolute.to_string();
        if !seen.insert(chapter_key(&url)) {
            continue;
        }
        let title = anchor
            .value()
            .attr("title")
            .map(clean_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| clean_text(&anchor.text().collect::<String>()));
        if title.is_empty() {
            continue;
        }
        let number = chapter_number_from_url(&url)
            .or_else(|| chapter_number_from_text(&title))
            .unwrap_or_else(|| page.saturating_sub(1) * 100 + index as u32 + 1);
        chapters.push(NovelChapter {
            number,
            title,
            url,
            volume: Some(page),
            downloaded: false,
            file: None,
        });
    }
    chapters
}

fn is_chapter_href(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    if lower.is_empty()
        || lower.starts_with('#')
        || lower.starts_with("javascript:")
        || lower.starts_with("mailto:")
        || lower.contains("report")
        || lower.contains("comment")
        || lower.contains("login")
        || lower.contains("register")
    {
        return false;
    }
    lower.contains("chapter")
        || lower.contains("/read/")
        || lower.contains("/episode/")
        || lower.contains("/c/")
}

fn chapter_key(url: &str) -> String {
    url.split('#')
        .next()
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_string()
}

fn advertised_chapter_total(document: &Html, json_ld_total: Option<u32>) -> Option<u32> {
    for (selector, attribute) in [
        ("[data-total-chapters]", "data-total-chapters"),
        ("[data-chapter-count]", "data-chapter-count"),
        ("[data-total-chapter]", "data-total-chapter"),
        (r#"meta[itemprop="numberOfEpisodes"]"#, "content"),
    ] {
        if let Some(value) =
            attr_value(document, selector, attribute).and_then(|value| parse_number(&value))
            && value > 0
        {
            return Some(value);
        }
    }

    for selector in [
        r#"[data-fictioneer-story-target="tab"][data-target="chapters"]"#,
        r#"[data-fictioneer-story-tab-name-param="chapters"]"#,
        r#".story__tabs .tabs__item"#,
        r#"[data-target="chapters"]"#,
        r#".chapter-count"#,
        r#"#chapter-count"#,
        r#"#total-chapters"#,
        r#".total-chapters"#,
        r#".chapter-total"#,
    ] {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        for element in document.select(&selector) {
            let text = clean_text(&element.text().collect::<String>());
            if let Some(value) = parse_chapter_count_text(&text) {
                return Some(value);
            }
        }
    }

    json_ld_total
}

fn parse_chapter_count_text(text: &str) -> Option<u32> {
    static LEADING: OnceLock<Regex> = OnceLock::new();
    static TRAILING: OnceLock<Regex> = OnceLock::new();
    let leading = LEADING.get_or_init(|| {
        Regex::new(r"(?i)^\s*([\d,]+)\s*(?:chapters?|episodes?)\s*$").expect("valid count regex")
    });
    let trailing = TRAILING.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:chapters|episodes)\s*(?:[:(]\s*|\(\s*|\s+)([\d,]+)\s*\)?\s*$")
            .expect("valid count regex")
    });
    leading
        .captures(text)
        .or_else(|| trailing.captures(text))
        .and_then(|captures| captures.get(1))
        .and_then(|value| parse_number(value.as_str()))
}

fn pagination_signals(document: &Html, base: &Url) -> PaginationSignals {
    let explicit_total = attr_value(document, "[data-total-page]", "data-total-page")
        .and_then(|value| parse_number(&value))
        .or_else(|| {
            attr_value(document, "input#total-page", "value").and_then(|value| parse_number(&value))
        })
        .or_else(|| {
            attr_value(document, "[data-total-pages]", "data-total-pages")
                .and_then(|value| parse_number(&value))
        });
    let mut total_pages = explicit_total;

    let query_regex = Regex::new(r"(?i)(?:page|paged)=(\d+)").expect("valid query page regex");
    let path_regex = Regex::new(r"(?i)/page/(\d+)").expect("valid path page regex");
    let mut query_slot: Option<(u32, usize, String)> = None;
    let mut path_slot: Option<(u32, usize, String)> = None;

    if let Ok(anchors) = Selector::parse("a[href]") {
        for anchor in document.select(&anchors) {
            let Some(href) = anchor.value().attr("href") else {
                continue;
            };
            let Ok(mut absolute) = base.join(href) else {
                continue;
            };
            absolute.set_fragment(None);
            if !same_toc_path(base, &absolute) {
                continue;
            }
            let value = absolute.to_string();
            for (regex, slot) in [
                (&query_regex, &mut query_slot),
                (&path_regex, &mut path_slot),
            ] {
                let Some(captures) = regex.captures(&value) else {
                    continue;
                };
                let Some(max) = captures.get(1) else {
                    continue;
                };
                let Ok(number) = max.as_str().parse::<u32>() else {
                    continue;
                };
                if number <= 1 {
                    continue;
                }
                let template = format!("{}{{page}}{}", &value[..max.start()], &value[max.end()..]);
                total_pages = Some(total_pages.unwrap_or(0).max(number));
                note_page_slot(slot, number, template);
            }
        }
    }

    let query_best = query_slot.filter(|(max, _, _)| *max > 1);
    let path_best = path_slot.filter(|(max, _, _)| *max > 1);
    let page_template = match (query_best, path_best) {
        (Some(query), Some(path)) => Some(if query.0 >= path.0 { query.2 } else { path.2 }),
        (Some(query), None) => Some(query.2),
        (None, Some(path)) => Some(path.2),
        (None, None) => None,
    };
    let next_url = next_page_url(document, base);
    let detected = explicit_total.is_some() || page_template.is_some() || next_url.is_some();

    PaginationSignals {
        total_pages,
        page_template,
        next_url,
        detected,
    }
}

fn note_page_slot(slot: &mut Option<(u32, usize, String)>, number: u32, template: String) {
    match slot {
        Some((best, count, best_template)) => {
            *count += 1;
            if number > *best {
                *best = number;
                *best_template = template;
            }
        }
        None => *slot = Some((number, 1, template)),
    }
}

fn same_toc_path(base: &Url, candidate: &Url) -> bool {
    let base_path = base.path().trim_end_matches('/');
    let candidate_path = candidate.path().trim_end_matches('/');
    if base_path.is_empty() || candidate_path.is_empty() {
        return true;
    }
    candidate_path == base_path || candidate_path.starts_with(&format!("{base_path}/"))
}

fn next_page_url(document: &Html, base: &Url) -> Option<String> {
    for selector in [
        r#"link[rel="next"]"#,
        r#"a[rel="next"]"#,
        r#"a.next"#,
        r#".pagination .next a"#,
        r#".pager .next a"#,
    ] {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        for element in document.select(&selector) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            if href.starts_with('#') || href.to_ascii_lowercase().contains("javascript:") {
                continue;
            }
            if let Ok(mut url) = base.join(href) {
                url.set_fragment(None);
                return Some(url.to_string());
            }
        }
    }
    None
}

fn default_page_template(base: &Url) -> String {
    let mut clean = base.clone();
    clean.set_fragment(None);
    let separator = if clean.query().is_some() { '&' } else { '?' };
    format!("{clean}{separator}page={{page}}")
}

fn page_template_url(template: &str, page: u32) -> String {
    template.replace("{page}", &page.to_string())
}

async fn fetch_generic_page(
    client: &reqwest::Client,
    page_url: &str,
    base: &Url,
    page: u32,
) -> Option<FetchedTocPage> {
    let trace = std::env::var_os("NOVELFORGE_QA_TRACE").is_some();
    for attempt in 1..=TOC_ATTEMPTS {
        let started = std::time::Instant::now();
        match fetch_html(client, page_url).await {
            Ok(html) => {
                let document = Html::parse_document(&expand_templates(&html));
                let chapters = parse_generic_chapters(&document, base, page);
                if trace {
                    eprintln!(
                        "[toc] generic page {page} attempt {attempt} bytes={} chapters={} elapsed={:.1}s",
                        html.len(),
                        chapters.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
                if !chapters.is_empty() {
                    let url = Url::parse(page_url).unwrap_or_else(|_| base.clone());
                    return Some(FetchedTocPage {
                        html,
                        url,
                        chapters,
                    });
                }
            }
            Err(error) => {
                if trace {
                    eprintln!(
                        "[toc] generic page {page} attempt {attempt} error elapsed={:.1}s: {error}",
                        started.elapsed().as_secs_f64()
                    );
                }
                // Transport owns HTTP retries and backoff.
                return None;
            }
        }
        if attempt < TOC_ATTEMPTS {
            // The page was readable but yielded no chapter links.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    None
}

fn discover_ajax_archive(document: &Html, html: &str, base: &Url) -> Option<AjaxArchive> {
    let novel_id = attr_value(document, "[data-novel-id]", "data-novel-id")
        .or_else(|| attr_value(document, "#truyen-id", "value"))
        .or_else(|| js_string_variable(html, "novelId"))
        .or_else(|| url_slug(base))?;
    let total_pages = attr_value(document, "[data-total-page]", "data-total-page")
        .and_then(|value| parse_number(&value))
        .unwrap_or(1)
        .max(1);

    if let Some(raw) = attr_value(document, "[data-list-url]", "data-list-url") {
        let endpoint = resolve_endpoint(base, &raw)?;
        return Some(AjaxArchive {
            endpoint,
            novel_id,
            total_pages,
            paged: total_pages > 1,
        });
    }

    let ajax_url = js_string_variable(html, "ajaxUrl");
    let mut endpoint = None;
    for name in [
        "ajaxChapterArchiveUrl",
        "ajaxChapterListUrl",
        "chapterArchiveUrl",
    ] {
        if let Some(value) = js_url_expression(html, name, ajax_url.as_deref()) {
            endpoint = Some(value);
            break;
        }
    }
    if endpoint.is_none() {
        let has_archive_grid = ["#chapter-archive", "[data-chapter-archive-grid]"]
            .iter()
            .any(|selector| {
                Selector::parse(selector)
                    .ok()
                    .is_some_and(|selector| document.select(&selector).next().is_some())
            });
        if has_archive_grid && let Some(ajax_url) = ajax_url.as_deref() {
            endpoint = Some(concat_url(ajax_url, "/chapter-archive"));
        }
    }

    let endpoint = resolve_endpoint(base, &endpoint?)?;
    Some(AjaxArchive {
        endpoint,
        novel_id,
        total_pages,
        paged: total_pages > 1,
    })
}

fn resolve_endpoint(base: &Url, raw: &str) -> Option<String> {
    let url = if raw.starts_with("http://") || raw.starts_with("https://") {
        Url::parse(raw).ok()?
    } else {
        base.join(raw).ok()?
    };
    if url.scheme() != base.scheme() || url.host_str()? != base.host_str()? {
        return None;
    }
    Some(url.to_string())
}

fn js_string_variable(html: &str, name: &str) -> Option<String> {
    let regex = Regex::new(&format!(
        r#"(?i)\b{}\s*[:=]\s*["']([^"']+)["']"#,
        regex::escape(name)
    ))
    .ok()?;
    regex
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
}

fn js_url_expression(html: &str, name: &str, ajax_url: Option<&str>) -> Option<String> {
    let regex = Regex::new(&format!(r#"(?i)\b{}\s*=\s*([^;\n]+)"#, regex::escape(name))).ok()?;
    let expression = regex
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().trim())?;
    if let Some(rest) = expression.strip_prefix("ajaxUrl") {
        let suffix = rest
            .trim()
            .trim_start_matches('+')
            .trim()
            .trim_matches(|character| character == '"' || character == '\'');
        return Some(concat_url(ajax_url?, suffix));
    }
    if expression.starts_with('"') || expression.starts_with('\'') {
        return Some(
            expression
                .trim_matches(|character| character == '"' || character == '\'')
                .to_string(),
        );
    }
    None
}

fn concat_url(base: &str, suffix: &str) -> String {
    if suffix.starts_with("http://") || suffix.starts_with("https://") {
        return suffix.to_string();
    }
    let mut joined = base.trim_end_matches('/').to_string();
    if !suffix.starts_with('/') {
        joined.push('/');
    }
    joined.push_str(suffix);
    joined
}

fn url_slug(base: &Url) -> Option<String> {
    base.path_segments()?
        .rfind(|segment| !segment.is_empty())
        .map(|segment| {
            segment
                .split_once('.')
                .map(|(stem, _)| stem)
                .unwrap_or(segment)
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

async fn fetch_ajax_chapters(
    client: &reqwest::Client,
    base: &Url,
    archive: &AjaxArchive,
) -> Result<(Vec<NovelChapter>, Vec<u32>, u32), String> {
    let pages = if archive.paged {
        (1..=archive.total_pages.min(MAX_TOC_PAGES)).collect::<Vec<_>>()
    } else {
        vec![1]
    };
    let mut chapters = Vec::new();
    let mut failed = Vec::new();
    let mut fetched = 0;
    let mut last_error = String::new();

    for page in &pages {
        let mut page_chapters = None;
        for attempt in 1..=TOC_ATTEMPTS {
            let mut url = Url::parse(&archive.endpoint)
                .map_err(|error| format!("invalid chapter archive URL: {error}"))?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("novelId", &archive.novel_id);
                if archive.paged {
                    query.append_pair("page", &page.to_string());
                }
            }
            match fetch_html(client, url.as_str()).await {
                Ok(body) => match archive_fragment(&body) {
                    Ok(fragment) => {
                        let document = Html::parse_fragment(&expand_templates(&fragment));
                        let parsed = parse_generic_chapters(&document, base, *page);
                        if !parsed.is_empty() {
                            page_chapters = Some(parsed);
                            break;
                        }
                    }
                    Err(error) => last_error = error,
                },
                Err(error) => {
                    last_error = error.to_string();
                    // Transport owns HTTP retries and backoff; only retry a
                    // readable archive response that failed to parse.
                    break;
                }
            }
            if attempt < TOC_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        match page_chapters {
            Some(parsed) => {
                chapters.extend(parsed);
                fetched += 1;
            }
            None => failed.push(*page),
        }
    }

    if chapters.is_empty() {
        return Err(if last_error.is_empty() {
            "the archive did not return chapter links".to_string()
        } else {
            last_error
        });
    }
    Ok((chapters, failed, fetched))
}

fn archive_fragment(body: &str) -> Result<String, String> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return Ok(body.to_string());
    }
    let value: Value =
        serde_json::from_str(trimmed).map_err(|error| format!("invalid JSON: {error}"))?;
    if let Some(code) = value.get("code").and_then(Value::as_i64)
        && code != 200
    {
        return Err(format!("archive returned code {code}"));
    }
    Ok(value
        .get("html")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

#[allow(clippy::too_many_arguments)]
fn assess_generic_index(
    listed: u32,
    html_listed: u32,
    max_number: Option<u32>,
    contiguous_from_one: bool,
    advertised_total: Option<u32>,
    json_ld_total: Option<u32>,
    latest_chapter: Option<u32>,
    pagination: &PaginationOutcome,
    pagination_detected: bool,
    failed_pages: &[u32],
    ajax_failed_pages: &[u32],
    ajax_error: Option<&str>,
    ajax_used: bool,
) -> (bool, String) {
    if !failed_pages.is_empty() {
        return (
            false,
            format!(
                "{} table-of-contents page(s) could not be fetched ({})",
                failed_pages.len(),
                preview_pages(failed_pages)
            ),
        );
    }
    if !ajax_failed_pages.is_empty() {
        return (
            false,
            format!(
                "the source chapter archive failed for {} page(s) ({})",
                ajax_failed_pages.len(),
                preview_pages(ajax_failed_pages)
            ),
        );
    }
    if pagination.limit_exceeded {
        return (
            false,
            format!("the table of contents exceeds the {MAX_TOC_PAGES}-page safety limit"),
        );
    }
    if let Some(expected) = advertised_total {
        if listed >= expected {
            return (
                true,
                format!("the source advertises {expected} chapter(s) and all are listed"),
            );
        }
        return (
            false,
            format!("{listed} of {expected} advertised chapter(s) are listed"),
        );
    }
    if let Some(expected) = json_ld_total
        && expected >= html_listed
    {
        if listed >= expected {
            return (
                true,
                format!("the JSON-LD ItemList contains all {expected} chapter(s)"),
            );
        }
        return (
            false,
            format!("{listed} of {expected} chapter(s) from the JSON-LD ItemList are listed"),
        );
    }
    // A smaller ItemList is a sample or a related list, not proof that the
    // larger HTML index is complete. Fall through to the other signals.
    if contiguous_from_one && let Some(expected) = max_number {
        return (true, format!("chapters 1–{expected} are all present"));
    }
    if let (Some(latest), Some(highest)) = (latest_chapter, max_number)
        && highest < latest
    {
        return (
            false,
            format!(
                "the source advertises latest chapter {latest}, but the list stops at {highest}"
            ),
        );
    }
    if pagination.reached_terminal {
        if let Some(expected) = pagination.expected_pages {
            return (
                true,
                format!(
                    "walked {}/{} table-of-contents pages to the terminal page",
                    pagination.walked_pages, expected
                ),
            );
        }
        return (
            true,
            format!(
                "walked {} table-of-contents pages to the terminal page",
                pagination.walked_pages
            ),
        );
    }
    if pagination_detected {
        return (
            false,
            "pagination was detected but the terminal page could not be reached".to_string(),
        );
    }
    if let Some(error) = ajax_error {
        return (
            false,
            format!("the source chapter archive could not be read ({error})"),
        );
    }
    if ajax_used {
        return (
            false,
            format!(
                "the source chapter archive returned {listed} chapter(s), but no advertised total, JSON-LD ItemList, contiguous numbering, or terminal TOC page proves the list is complete"
            ),
        );
    }
    (
        false,
        format!(
            "no advertised chapter total, JSON-LD ItemList, contiguous chapter numbers, or terminal table-of-contents page was found, so {listed} chapter(s) cannot be proven complete"
        ),
    )
}

/// Fetches table-of-contents pages one at a time. Spacing and HTTP retries are
/// enforced by the shared transport gate, so a 429 pauses every request to the
/// host instead of only this loop.
async fn fetch_toc_pages<F>(
    client: &reqwest::Client,
    base: Url,
    pages: Vec<u32>,
    parse: F,
    progress: Option<IndexProgress<'_>>,
    total_pages: u32,
) -> (Vec<(u32, Vec<NovelChapter>)>, Vec<u32>)
where
    F: Fn(&Html, &Url, u32) -> Vec<NovelChapter>,
{
    let mut results = Vec::new();
    let mut failed = Vec::new();
    let trace = std::env::var_os("NOVELFORGE_QA_TRACE").is_some();

    for page in pages {
        let mut chapters = None;
        for attempt in 1..=TOC_ATTEMPTS {
            let page_url = page_url(&base, page);
            let started = std::time::Instant::now();
            match fetch_html(client, &page_url).await {
                Ok(html) => {
                    let document = Html::parse_document(&html);
                    let parsed = parse(&document, &base, page);
                    if trace {
                        eprintln!(
                            "[toc] page {page} attempt {attempt} bytes={} chapters={} elapsed={:.1}s",
                            html.len(),
                            parsed.len(),
                            started.elapsed().as_secs_f64()
                        );
                    }
                    if !parsed.is_empty() {
                        chapters = Some(parsed);
                        break;
                    }
                }
                Err(error) => {
                    if trace {
                        eprintln!(
                            "[toc] page {page} attempt {attempt} error elapsed={:.1}s: {error}",
                            started.elapsed().as_secs_f64()
                        );
                    }
                    // Transport already retried with the source policy and
                    // honoured 429/Retry-After. Do not start a second retry
                    // loop around it.
                    break;
                }
            }

            if attempt < TOC_ATTEMPTS {
                // The response was readable but had no chapter links; a
                // single re-read catches a truncated/placeholder page.
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        if let Some(parsed) = chapters {
            results.push((page, parsed));
            // Report after the page is in hand: a counter that runs ahead of
            // the walk would claim progress that has not happened yet.
            if let Some(report) = progress {
                report(page, total_pages.max(page));
            }
        } else {
            failed.push(page);
        }
    }

    (results, failed)
}

fn preview_pages(pages: &[u32]) -> String {
    let preview = pages
        .iter()
        .take(8)
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if pages.len() > 8 {
        format!("{preview}, … (+{})", pages.len() - 8)
    } else {
        preview
    }
}

fn parse_novelfire_chapters(document: &Html, base: &Url, page: u32) -> Vec<NovelChapter> {
    let selector = Selector::parse("ul.chapter-list li a").expect("valid selector");
    let mut chapters = Vec::new();
    for (index, anchor) in document.select(&selector).enumerate() {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Ok(absolute) = base.join(href) else {
            continue;
        };
        let title = anchor
            .value()
            .attr("title")
            .map(clean_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| clean_text(&anchor.text().collect::<String>()));
        if title.is_empty() {
            continue;
        }
        let number = chapter_number_from_url(absolute.as_str())
            .or_else(|| chapter_number_from_text(&title))
            .unwrap_or((page - 1) * 100 + index as u32 + 1);
        chapters.push(NovelChapter {
            number,
            title,
            url: absolute.to_string(),
            volume: Some(page),
            downloaded: false,
            file: None,
        });
    }
    chapters
}

fn parse_novelphoenix_chapters(document: &Html, base: &Url, page: u32) -> Vec<NovelChapter> {
    let selector = Selector::parse("#idData li a").expect("valid selector");
    let mut chapters = Vec::new();
    for (index, anchor) in document.select(&selector).enumerate() {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Ok(absolute) = base.join(href) else {
            continue;
        };
        let title = anchor
            .value()
            .attr("title")
            .map(clean_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| clean_text(&anchor.text().collect::<String>()));
        if title.is_empty() {
            continue;
        }
        let number = chapter_number_from_url(absolute.as_str())
            .or_else(|| chapter_number_from_text(&title))
            .unwrap_or((page - 1) * 40 + index as u32 + 1);
        chapters.push(NovelChapter {
            number,
            title,
            url: absolute.to_string(),
            volume: Some(page),
            downloaded: false,
            file: None,
        });
    }
    chapters
}

fn page_url(base: &Url, page: u32) -> String {
    let mut url = base.clone();
    url.set_query(Some(&format!("page={page}")));
    url.to_string()
}

fn last_page_number(document: &Html) -> Option<u32> {
    let selector = Selector::parse("a[href]").ok()?;
    let pattern = Regex::new(r"[?&]page=(\d+)").expect("valid page regex");
    document
        .select(&selector)
        .filter_map(|anchor| anchor.value().attr("href"))
        .filter_map(|href| pattern.captures(href))
        .filter_map(|captures| captures.get(1)?.as_str().parse::<u32>().ok())
        .max()
}

fn dedupe_chapters(chapters: Vec<NovelChapter>) -> Vec<NovelChapter> {
    let mut seen = std::collections::HashSet::new();
    chapters
        .into_iter()
        .filter(|chapter| seen.insert(chapter.url.clone()))
        .collect()
}

fn chapter_number_from_url(url: &str) -> Option<u32> {
    static CHAPTER: OnceLock<Regex> = OnceLock::new();
    let regex = CHAPTER
        .get_or_init(|| Regex::new(r"(?i)chapter[-_/](\d+)").expect("valid chapter URL regex"));
    regex
        .captures(url)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

fn chapter_number_from_text(text: &str) -> Option<u32> {
    static CHAPTER: OnceLock<Regex> = OnceLock::new();
    let regex = CHAPTER.get_or_init(|| {
        Regex::new(r"(?i)\bchapter\s*#?\s*(\d+)").expect("valid chapter text regex")
    });
    regex
        .captures(text)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

/// Some sources (NovelPhoenix for several Korean novels) publish chapter names
/// for part of a table of contents but leave an early run as "Chapter 12". The
/// name of those chapters exists nowhere on that host, so borrow them from a
/// source that publishes a full list. The source's own names are never
/// replaced.
fn title_lookup_needed(index: &NovelChapterIndex) -> bool {
    index.complete && !index.title_lookup_attempted && needs_title_lookup(index)
}

/// The full title pass runs for a donor-sized gap, or for a short tail of
/// numbered rows that can be read straight from their chapter pages.
fn title_pass_needed(index: &NovelChapterIndex) -> bool {
    title_lookup_needed(index)
        || (index.complete && !index.title_recovery_attempted && needs_title_recovery(index))
}

fn needs_title_recovery(index: &NovelChapterIndex) -> bool {
    let bare = numbered_chapter_count(index);
    (1..=TITLE_RECOVERY_MAX_PAGES).contains(&bare)
}

fn needs_title_lookup(index: &NovelChapterIndex) -> bool {
    if index.chapters.len() < TITLE_ENRICHMENT_MIN_BARE {
        return false;
    }
    let bare = numbered_chapter_count(index);
    bare >= TITLE_ENRICHMENT_MIN_BARE
        && (bare as f64) / (index.chapters.len() as f64) >= TITLE_ENRICHMENT_MIN_BARE_SHARE
}

fn numbered_chapter_count(index: &NovelChapterIndex) -> usize {
    index
        .chapters
        .iter()
        .filter(|chapter| bare_chapter_title(&chapter.title))
        .count()
}

/// True for titles that carry no name at all: "Chapter 9", "Chapter 9 (END)",
/// "Chapter #9", "Chapter 9 - 9" or a bare "9".
fn bare_chapter_title(title: &str) -> bool {
    static BARE: OnceLock<Regex> = OnceLock::new();
    let bare = BARE.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:chapter\s*#?\s*\d+(?:\s*\(end\))?(?:\s*[-:–—]\s*\d+)?|\d+)\s*$")
            .expect("valid bare chapter title regex")
    });
    bare.is_match(title.trim())
}

/// The chapter number a reader sees, independent of whichever URL scheme a
/// source uses. NovelFire can number the same chapter `chapter-2` with the
/// visible title "Chapter 1"; matching on the visible number keeps the two
/// sources aligned.
fn display_chapter_number(chapter: &NovelChapter) -> Option<u32> {
    chapter_number_from_text(&chapter.title)
        .or_else(|| chapter_number_from_url(&chapter.url))
        .or(Some(chapter.number))
}

/// Strip the numbering a donor repeats before the real name:
/// "Chapter 1 - 1: The heart of a demon…" → "The heart of a demon…".
fn donor_chapter_name(title: &str, number: u32) -> Option<String> {
    static PREFIX: OnceLock<Regex> = OnceLock::new();
    let prefix = PREFIX.get_or_init(|| {
        Regex::new(r"(?i)^\s*chapter\s*#?\s*(\d+)\s*(?:\(end\))?\s*(?:[-:–—.]\s*)?(.*)$")
            .expect("valid chapter prefix regex")
    });
    static REPEAT: OnceLock<Regex> = OnceLock::new();
    let repeat = REPEAT.get_or_init(|| {
        Regex::new(r"^\s*(\d+)\s*[-:–—.]\s*(.*)$").expect("valid repeated number regex")
    });

    let mut rest = title.trim().to_string();
    if let Some(captures) = prefix.captures(&rest) {
        let stated = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<u32>().ok());
        rest = captures
            .get(2)
            .map(|value| value.as_str().to_string())
            .unwrap_or_default();
        if let Some(captures) = repeat.captures(&rest) {
            let repeated = captures
                .get(1)
                .and_then(|value| value.as_str().parse::<u32>().ok());
            if repeated == Some(number) || repeated == stated {
                rest = captures
                    .get(2)
                    .map(|value| value.as_str().to_string())
                    .unwrap_or_default();
            }
        }
    }

    let name = rest
        .trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | ':' | '–' | '—' | '.' | ',')
        })
        .trim();
    if name.is_empty()
        || name.chars().all(|character| character.is_ascii_digit())
        || bare_chapter_title(name)
    {
        return None;
    }
    Some(name.chars().take(160).collect())
}

/// Copy donor titles onto the target by visible chapter number, filling only
/// rows the target left numbered. Existing names and all URLs/numbers stay
/// untouched. Returns false (and changes nothing) when the donor covers too
/// little of the target's numbered rows to trust.
fn adopt_chapter_titles(target: &mut NovelChapterIndex, donor: &NovelChapterIndex) -> bool {
    if target.chapters.is_empty() || donor.chapters.is_empty() {
        return false;
    }
    let mut donor_names: HashMap<u32, String> = HashMap::new();
    for chapter in &donor.chapters {
        let Some(number) = display_chapter_number(chapter) else {
            continue;
        };
        if bare_chapter_title(&chapter.title) {
            continue;
        }
        let Some(name) = donor_chapter_name(&chapter.title, number) else {
            continue;
        };
        donor_names.entry(number).or_insert(name);
    }
    if donor_names.is_empty() {
        return false;
    }

    // Guard against a donor that is one chapter off (or a different
    // translation): compare the names both sides already publish for the same
    // visible number. A mirrored source agrees exactly; a shifted list does
    // not. Books that are entirely numbered have no sample and skip this.
    if !names_align_with_donor(target, &donor_names) {
        return false;
    }

    let mut candidate = target.clone();
    let mut numbered = 0_usize;
    let mut adopted = 0_usize;
    for chapter in &mut candidate.chapters {
        if !bare_chapter_title(&chapter.title) {
            continue;
        }
        numbered += 1;
        let Some(number) = display_chapter_number(chapter) else {
            continue;
        };
        let Some(name) = donor_names.get(&number) else {
            continue;
        };
        chapter.title = name.clone();
        adopted += 1;
    }
    if numbered == 0 || adopted < TITLE_ENRICHMENT_MIN_BARE {
        return false;
    }
    let coverage = adopted as f64 / numbered as f64;
    if coverage < TITLE_ENRICHMENT_MIN_COVERAGE {
        return false;
    }

    let previous = candidate.detail.take().unwrap_or_default();
    let note = format!(
        "Filled {adopted} numbered chapter name(s) from {}; this source's table of contents leaves those rows unnamed.",
        donor.source
    );
    let detail = if previous.is_empty() {
        note
    } else {
        format!("{previous} {note}")
    };
    candidate.title_source = Some(donor.source.clone());
    candidate.detail = Some(detail);
    *target = candidate;
    true
}

/// True unless the two sources clearly disagree about the names they both
/// publish. At least five overlapping named rows are needed before a verdict;
/// below that there is nothing to compare and the caller relies on the
/// title/author match and number coverage instead.
fn names_align_with_donor(target: &NovelChapterIndex, donor_names: &HashMap<u32, String>) -> bool {
    let mut compared = 0_usize;
    let mut agreed = 0_usize;
    for chapter in &target.chapters {
        if bare_chapter_title(&chapter.title) {
            continue;
        }
        let Some(number) = display_chapter_number(chapter) else {
            continue;
        };
        let Some(donor_name) = donor_names.get(&number) else {
            continue;
        };
        let Some(target_name) = donor_chapter_name(&chapter.title, number) else {
            continue;
        };
        compared += 1;
        if normalized_name(&target_name) == normalized_name(donor_name) {
            agreed += 1;
        }
    }
    compared < 5 || (agreed as f64) / (compared as f64) >= 0.8
}

fn normalized_name(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

/// Full title pass: borrow names from a verified donor, then (for the few rows
/// no donor could name) read the source's own chapter pages. Both passes share
/// one budget so a slow donor cannot double the index fetch time.
async fn enrich_index_titles(settings: &Settings, index: &mut NovelChapterIndex) {
    let started = Instant::now();
    let budget = Duration::from_secs(TITLE_ENRICHMENT_BUDGET_SECS);
    if title_lookup_needed(index) {
        let _ = tokio::time::timeout(budget, enrich_chapter_titles(settings, index)).await;
    }
    if !needs_title_recovery(index) || index.title_recovery_attempted {
        return;
    }
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining < Duration::from_secs(5) {
        return;
    }
    let recovery = tokio::time::timeout(
        remaining.min(Duration::from_secs(TITLE_RECOVERY_MAX_SECS)),
        recover_chapter_titles(index),
    )
    .await;
    // A timeout leaves this false: the rows it did not reach are still
    // numbered, so a later cache hit resumes with only those pages.
    if recovery.is_ok() {
        index.title_recovery_attempted = true;
    }
}

/// Some sources leave a few table-of-contents rows as bare numbers even though
/// the chapter page itself opens with the real name ("Episode 1 – Starting the
/// Paid Service (1)"). Read only those pages, with a hard page cap so a fully
/// numbered book never becomes a crawl.
async fn recover_chapter_titles(index: &mut NovelChapterIndex) -> usize {
    let targets: Vec<(usize, String)> = index
        .chapters
        .iter()
        .enumerate()
        .filter(|(_, chapter)| bare_chapter_title(&chapter.title))
        .map(|(position, chapter)| (position, chapter.url.clone()))
        .collect();
    if targets.is_empty() || targets.len() > TITLE_RECOVERY_MAX_PAGES {
        return 0;
    }
    let Ok(client) = http_client() else {
        return 0;
    };

    let mut queue = targets.into_iter();
    let mut jobs = tokio::task::JoinSet::new();
    let mut recovered = 0_usize;
    let base_detail = index.detail.clone().unwrap_or_default();
    const CONCURRENCY: usize = 3;
    loop {
        while jobs.len() < CONCURRENCY {
            let Some((position, url)) = queue.next() else {
                break;
            };
            let client = client.clone();
            jobs.spawn(async move {
                let html = fetch_html(&client, &url).await.ok()?;
                chapter_name_from_html(&html).map(|name| (position, name))
            });
        }
        if jobs.is_empty() {
            break;
        }
        if let Some(Ok(Some((position, name)))) = jobs.join_next().await
            && let Some(chapter) = index.chapters.get_mut(position)
        {
            chapter.title = name;
            recovered += 1;
            // Written after every hit so a budget timeout still leaves an
            // accurate note for the rows that were recovered.
            let note = format!(
                "Recovered {recovered} chapter name(s) from this source's chapter pages; the table of contents leaves those rows unnamed."
            );
            index.detail = Some(if base_detail.is_empty() {
                note
            } else {
                format!("{base_detail} {note}")
            });
        }
    }
    recovered
}

fn chapter_name_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    static CONTAINERS: OnceLock<Vec<Selector>> = OnceLock::new();
    let containers = CONTAINERS.get_or_init(|| {
        [
            "#chapter-content",
            ".chapter-c",
            ".chapter-content",
            "article",
        ]
        .iter()
        .filter_map(|selector| Selector::parse(selector).ok())
        .collect()
    });
    static PARAGRAPH: OnceLock<Selector> = OnceLock::new();
    let paragraph = PARAGRAPH.get_or_init(|| Selector::parse("p").expect("valid p selector"));

    for container in containers {
        for element in document.select(container) {
            let mut checked = 0;
            for node in element.select(paragraph) {
                let text = clean_text(&node.text().collect::<String>());
                if text.is_empty() {
                    continue;
                }
                checked += 1;
                if let Some(name) = name_from_header(&text) {
                    return Some(name);
                }
                // Some chapters print only the name on the first line
                // ("Starting the Paid Service (3)") without an Episode label.
                // Only the first paragraph is allowed this weaker test; later
                // paragraphs are body prose.
                if checked == 1
                    && let Some(name) = looks_like_chapter_name(&text)
                {
                    return Some(name);
                }
                if checked >= 3 {
                    break;
                }
            }
        }
    }
    None
}

/// Conservative fallback for a title line with no "Episode"/"Prologue" label.
/// A sentence ends with punctuation; a name does not. The word/length caps and
/// rejection of leading quotes/brackets keep body prose out.
fn looks_like_chapter_name(line: &str) -> Option<String> {
    let name = line.trim();
    let length = name.chars().count();
    if !(3..=80).contains(&length) {
        return None;
    }
    if name
        .chars()
        .any(|character| matches!(character, '.' | '!' | '?' | ',' | ';' | ':'))
    {
        return None;
    }
    if name.starts_with(|character: char| {
        matches!(
            character,
            '“' | '”' | '"' | '\'' | '‘' | '’' | '[' | '(' | '<' | '-' | '–' | '—'
        )
    }) {
        return None;
    }
    let first = name.chars().next()?;
    if !first.is_uppercase() && !first.is_ascii_digit() {
        return None;
    }
    let words = name.split_whitespace().count();
    if !(3..=10).contains(&words) || bare_chapter_title(name) {
        return None;
    }
    Some(name.to_string())
}

/// Recognise the reader-facing heading a chapter page repeats at the top of
/// its text: "Episode 12 – War of Kings (3)", "Prologue – …", or the same
/// behind a "Chapter 276:" prefix. Body prose is rejected because the line
/// must start with one of those labels.
fn name_from_header(line: &str) -> Option<String> {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    let header = HEADER.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:chapter\s*#?\s*\d+\s*[:.\-–—]\s*)?(?:episode\s*#?\s*\d+\s*[:.\-–—]\s*|prologue\s*[:.\-–—]\s*)(.+)$",
        )
        .expect("valid chapter header regex")
    });
    let name = header.captures(line.trim())?.get(1)?.as_str().trim();
    let name = name.trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '"' | '\'' | '“' | '”' | '‘' | '’' | '-' | '–' | '—' | ':' | '.'
            )
    });
    if name.is_empty() || name.chars().count() > 160 || bare_chapter_title(name) {
        return None;
    }
    Some(name.to_string())
}

fn host_of(value: &str) -> Option<String> {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
}

fn same_author(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .to_lowercase()
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect::<String>()
    };
    let left = normalize(left);
    let right = normalize(right);
    !left.is_empty() && left == right
}

async fn enrich_chapter_titles(settings: &Settings, index: &mut NovelChapterIndex) -> bool {
    // QA-visible trace; silent in normal use because the flag is unset.
    let debug = std::env::var_os("NOVELFORGE_TITLE_DEBUG").is_some();
    macro_rules! note {
        ($($arg:tt)*) => {
            if debug {
                eprintln!("[titles] {}", format!($($arg)*));
            }
        };
    }

    if index.title_source.is_some() || index.title_lookup_attempted {
        return false;
    }
    // Record the attempt even when no donor matches, so a numbering-only
    // source is not searched again on every cache hit.
    index.title_lookup_attempted = true;
    if !needs_title_lookup(index) {
        note!(
            "skip: {} of {} rows numbered",
            numbered_chapter_count(index),
            index.chapters.len()
        );
        return false;
    }
    note!(
        "start: {}/{} rows numbered, source {}",
        numbered_chapter_count(index),
        index.chapters.len(),
        index.source
    );

    let Ok(target) = fetch_novel_summary(&index.source_url).await else {
        note!("target summary failed");
        return false;
    };
    let title = target.title.trim().to_string();
    if title.chars().count() < 2 {
        note!("target title too short: {title:?}");
        return false;
    }
    note!("target title {title:?} author {:?}", target.author);
    let current_host = host_of(&index.source_url);

    let response = crate::resolver::search_novels_from(settings, &title, &[]).await;
    note!(
        "search results={} failures={}",
        response.results.len(),
        response.failures.len()
    );
    let mut candidates = response
        .results
        .into_iter()
        .filter(|result| host_of(&result.url).is_some() && host_of(&result.url) != current_host)
        // Exact normalised title only. A sequel such as "…: Dream Gu" or a
        // fan-fiction must never contribute names.
        .filter(|result| crate::resolver::relevance(&title, &result.title) >= 1.0)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|result| {
        let host = host_of(&result.url).unwrap_or_default();
        (sources::rank_for_host(&host), host)
    });
    let mut seen = HashSet::new();
    candidates.retain(|result| seen.insert(result.url.clone()));
    note!(
        "exact candidates: {:?}",
        candidates
            .iter()
            .map(|c| (&c.source, &c.url))
            .collect::<Vec<_>>()
    );

    for candidate in candidates {
        let Some(host) = host_of(&candidate.url) else {
            continue;
        };
        let profile = sources::for_host(&host);
        if profile.id == "generic"
            || !profile.capabilities.index
            || !profile.support.is_index_proven()
        {
            note!("skip {}: profile index not proven", candidate.url);
            continue;
        }
        let Ok(donor_meta) = fetch_novel_summary(&candidate.url).await else {
            note!("skip {}: donor summary failed", candidate.url);
            continue;
        };
        note!(
            "donor {} title {:?} author {:?}",
            candidate.url,
            donor_meta.title,
            donor_meta.author
        );
        if crate::resolver::relevance(&title, &donor_meta.title) < 1.0 {
            note!("skip {}: title mismatch", candidate.url);
            continue;
        }
        if let (Some(left), Some(right)) = (target.author.as_deref(), donor_meta.author.as_deref())
            && !same_author(left, right)
        {
            note!(
                "skip {}: author mismatch {left:?} vs {right:?}",
                candidate.url
            );
            continue;
        }
        // Donor lookup never enriches itself: one hop only, so a pair of
        // numbering-only sources cannot bounce names back and forth.
        let Ok(donor) = Box::pin(fetch_chapter_index_with(
            settings,
            &candidate.url,
            false,
            false,
            None,
        ))
        .await
        else {
            note!("skip {}: donor index failed", candidate.url);
            continue;
        };
        note!(
            "donor index {} complete={} chapters={}",
            candidate.url,
            donor.complete,
            donor.chapters.len()
        );
        if !donor.complete || donor.chapters.len() < 2 {
            continue;
        }
        if adopt_chapter_titles(index, &donor) {
            note!("adopted names from {}", donor.source);
            return true;
        }
        note!("coverage rejected for {}", donor.source);
    }
    false
}

fn first_child_text(element: &ElementRef<'_>, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    element
        .select(&selector)
        .next()
        .map(|child| clean_text(&child.text().collect::<String>()))
}

fn parse_number(value: &str) -> Option<u32> {
    let digits = value
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn clean_text(value: &str) -> String {
    value
        .replace(['\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}'], " ")
        .replace('\u{00a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Drops a trailing `| SiteName` / `- SiteName` from a title meta tag. Only
/// applied by source-specific parsers; the generic parser keeps the tag as-is.
fn strip_site_suffix(title: &str, site: &str) -> String {
    let title = title.trim();
    for separator in ['|', '-', '–', '—'] {
        if let Some((head, tail)) = title.rsplit_once(separator)
            && tail.trim().eq_ignore_ascii_case(site)
        {
            return head.trim().to_string();
        }
    }
    title.to_string()
}

fn unique_values(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

// ---------------------------------------------------------------------------
// JSON-LD
// ---------------------------------------------------------------------------

fn json_ld(html: &str) -> Option<Value> {
    json_ld_values(html).into_iter().next()
}

fn json_ld_values(html: &str) -> Vec<Value> {
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    let regex = SCRIPT.get_or_init(|| {
        Regex::new(r#"(?is)<script[^>]+type=["']application/ld\+json["'][^>]*>(.*?)</script>"#)
            .expect("valid JSON-LD regex")
    });
    regex
        .captures_iter(html)
        .filter_map(|captures| {
            let body = captures.get(1)?.as_str().trim();
            serde_json::from_str::<Value>(body).ok()
        })
        .collect()
}

fn json_ld_chapter_list(html: &str, base: &Url) -> (Vec<NovelChapter>, Option<u32>) {
    let values = json_ld_values(html);
    let mut lists = Vec::new();
    for value in &values {
        collect_item_lists(value, &mut lists);
    }
    let lists = lists
        .into_iter()
        .filter(|list| item_list_looks_like_chapters(list, base))
        .collect::<Vec<_>>();

    let mut chapters = Vec::new();
    let mut total = None;
    for list in lists {
        let items = list.get("itemListElement").and_then(Value::as_array);
        let declared = list
            .get("numberOfItems")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        let count = declared.or_else(|| items.map(|items| items.len() as u32));
        if let Some(count) = count {
            total = Some(total.unwrap_or(0).max(count));
        }
        let Some(items) = items else {
            continue;
        };
        for (index, item) in items.iter().enumerate() {
            let Some(raw_url) = json_item_url(item) else {
                continue;
            };
            let Ok(absolute) = base.join(raw_url) else {
                continue;
            };
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| {
                    item.get("item")
                        .and_then(|value| value.get("name"))
                        .and_then(Value::as_str)
                })
                .map(clean_text)
                .filter(|value| !value.is_empty());
            let number = item
                .get("position")
                .and_then(Value::as_u64)
                .map(|value| value as u32)
                .or_else(|| chapter_number_from_url(absolute.as_str()))
                .or_else(|| name.as_deref().and_then(chapter_number_from_text))
                .unwrap_or(index as u32 + 1);
            chapters.push(NovelChapter {
                number,
                title: name.unwrap_or_else(|| format!("Chapter {number}")),
                url: absolute.to_string(),
                volume: None,
                downloaded: false,
                file: None,
            });
        }
    }
    (chapters, total)
}

fn item_list_looks_like_chapters(list: &Value, base: &Url) -> bool {
    for key in ["name", "description"] {
        if list
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| text.to_ascii_lowercase().contains("chapter"))
        {
            return true;
        }
    }
    let Some(items) = list.get("itemListElement").and_then(Value::as_array) else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let chapter_like = items
        .iter()
        .filter(|item| {
            let Some(raw_url) = json_item_url(item) else {
                return false;
            };
            if raw_url.to_ascii_lowercase().contains("chapter") {
                return true;
            }
            base.join(raw_url)
                .ok()
                .is_some_and(|absolute| same_toc_path(base, &absolute))
        })
        .count();
    chapter_like * 2 >= items.len()
}

fn json_item_url(item: &Value) -> Option<&str> {
    item.get("url")
        .and_then(Value::as_str)
        .or_else(|| {
            item.get("item")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            item.get("item")
                .and_then(|value| value.get("@id"))
                .and_then(Value::as_str)
        })
        .or_else(|| item.get("@id").and_then(Value::as_str))
}

fn collect_item_lists<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_item_lists(item, out);
            }
        }
        Value::Object(map) => {
            let is_item_list = match map.get("@type") {
                Some(Value::String(kind)) => kind.eq_ignore_ascii_case("ItemList"),
                Some(Value::Array(kinds)) => kinds
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|kind| kind.eq_ignore_ascii_case("ItemList")),
                _ => false,
            };
            if is_item_list {
                out.push(value);
            }
            for item in map.values() {
                collect_item_lists(item, out);
            }
        }
        _ => {}
    }
}

fn json_value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(clean_text)
        .filter(|value| !value.is_empty())
}

fn json_strings(value: &Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(clean_text)
            .filter(|value| !value.is_empty())
            .collect(),
        Some(Value::String(item)) => vec![clean_text(item)],
        _ => Vec::new(),
    }
}

/// Author names from whichever JSON-LD block carries them. The first block on
/// a page is often a `WebSite`/`BreadcrumbList`, so scan until a `Book`-shaped
/// entry answers.
fn json_ld_authors(html: &str) -> Vec<String> {
    json_ld_values(html)
        .iter()
        .find_map(|value| {
            let authors = json_authors(value);
            (!authors.is_empty()).then_some(authors)
        })
        .unwrap_or_default()
}

fn json_authors(value: &Value) -> Vec<String> {
    match value.get("author") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("name").and_then(Value::as_str))
            .map(clean_text)
            .filter(|value| !value.is_empty())
            .collect(),
        Some(Value::Object(_)) => value
            .get("author")
            .and_then(|item| item.get("name"))
            .and_then(Value::as_str)
            .map(clean_text)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

const ALIAS_LABELS: &[&str] = &[
    "alternative names",
    "alternative titles",
    "alternative title",
    "associated names",
    "also known as",
    "aliases",
    "alias",
];

/// Alternative titles advertised by the page. Prefers schema.org
/// `alternateName`, then common “Alternative names: …” rows. This is evidence
/// from the source itself, not a guessed translation.
fn parse_aliases(html: &str, document: &Html, title: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    for value in json_ld_values(html) {
        collect_alternate_names(&value, &mut aliases);
    }

    if let Ok(selector) = Selector::parse("li, p, div, tr, dt, dd, span") {
        for element in document.select(&selector) {
            let text = clean_text(&element.text().collect::<String>());
            if text.len() > 280 {
                continue;
            }
            let lower = text.to_lowercase();
            let Some(label) = ALIAS_LABELS.iter().find(|label| lower.starts_with(**label)) else {
                continue;
            };
            let rest = text[label.len()..].trim_start();
            let Some(value) = rest
                .strip_prefix(':')
                .or_else(|| rest.strip_prefix('：'))
                .or_else(|| rest.strip_prefix(" - "))
                .or_else(|| rest.strip_prefix(" – "))
            else {
                continue;
            };
            push_alias_values(value, &mut aliases);
        }
    }

    let title_key = alias_key(title);
    unique_values(aliases.into_iter())
        .into_iter()
        .filter(|alias| alias.chars().count() >= 2 && alias.chars().count() <= 120)
        .filter(|alias| !title_key.is_empty() && alias_key(alias) != title_key)
        .take(12)
        .collect()
}

/// Case/punctuation-insensitive key used only to drop an “alternative name”
/// row that merely repeats the novel's own title.
fn alias_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn collect_alternate_names(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(alternate) = map.get("alternateName") {
                match alternate {
                    Value::Array(items) => {
                        for item in items {
                            if let Some(text) = item.as_str() {
                                push_alias_values(text, out);
                            }
                        }
                    }
                    Value::String(text) => push_alias_values(text, out),
                    _ => {}
                }
            }
            for nested in map.values() {
                collect_alternate_names(nested, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_alternate_names(item, out);
            }
        }
        _ => {}
    }
}

fn push_alias_values(value: &str, out: &mut Vec<String>) {
    for part in value.split(['/', ';', '|', '、', ',']) {
        let alias = clean_text(
            part.trim()
                .trim_matches(|character: char| {
                    matches!(
                        character,
                        '"' | '\'' | '“' | '”' | '‘' | '’' | '[' | ']' | '(' | ')'
                    )
                })
                .trim(),
        );
        if alias.is_empty()
            || alias.len() > 120
            || alias.to_ascii_lowercase().starts_with("http")
            || ALIAS_LABELS.contains(&alias.to_ascii_lowercase().as_str())
        {
            continue;
        }
        out.push(alias);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases_from_json_ld_and_labelled_rows() {
        let html = r#"
            <html><body>
              <script type="application/ld+json">
                {"@context":"https://schema.org","@type":"Book",
                 "name":"Reverend Insanity",
                 "alternateName":["Master of Gu","Gu Zhen Ren's Masterpiece"]}
              </script>
              <div class="info">
                <div class="item"><span>Alternative names:</span>
                  <span>Reverend Insanity / Dao of Gu / 蛊真人</span>
                </div>
              </div>
            </body></html>
        "#;
        let document = Html::parse_document(html);
        let aliases = parse_aliases(html, &document, "Reverend Insanity");
        assert!(aliases.contains(&"Master of Gu".to_string()));
        assert!(aliases.contains(&"Dao of Gu".to_string()));
        assert!(aliases.contains(&"蛊真人".to_string()));
        assert!(!aliases.contains(&"Reverend Insanity".to_string()));
        assert_eq!(
            aliases
                .iter()
                .filter(|alias| *alias == "Master of Gu")
                .count(),
            1
        );
    }

    #[test]
    fn parses_novelfire_detail_page() {
        let html = r#"
            <html><body>
              <figure class="cover"><img src="https://novelfire.net/server-1/reverend-insanity.jpg"></figure>
              <div class="novel-info">
                <h1 class="novel-title">Reverend Insanity</h1>
                <div class="author"><span>Author:</span>
                  <a href="/author/gu-zhen-ren"><span itemprop="author">Gu Zhen Ren</span></a>
                  <a href="/author/other"><span itemprop="author">蛊真人</span></a>
                </div>
                <div class="header-stats">
                  <span><strong>2334</strong><small>Chapters</small></span>
                  <span><strong class="completed">Completed</strong><small>Status</small></span>
                </div>
                <div class="categories"><ul><li><a>Xianxia</a></li><li><a>Martial Arts</a></li></ul></div>
              </div>
              <meta itemprop="description" content="A story about a time traveler.">
            </body></html>
        "#;
        let url = Url::parse("https://novelfire.net/book/reverend-insanity").unwrap();
        let metadata = parse_novelfire_metadata(&url, html);
        assert_eq!(metadata.title, "Reverend Insanity");
        assert_eq!(metadata.author.as_deref(), Some("Gu Zhen Ren"));
        assert_eq!(metadata.chapter_count, Some(2334));
        assert_eq!(metadata.status.as_deref(), Some("Completed"));
        assert_eq!(metadata.tags, vec!["Xianxia", "Martial Arts"]);
        assert!(metadata.synopsis.is_some());
    }

    #[test]
    fn parses_novelphoenix_json_ld() {
        let html = r#"
            <html><body>
              <script type="application/ld+json">
              {"@type":"Book","name":"Reverend Insanity","image":"https://novelphoenix.net/cover.jpg",
               "author":[{"@type":"Person","name":"Gu Zhen Ren"}],
               "genre":["Fantasy","Xianxia"],"creativeWorkStatus":"Completed",
               "inLanguage":"en","description":"A dark cultivation story."}
              </script>
              <h1 class="tit">Reverend Insanity</h1>
            </body></html>
        "#;
        let url = Url::parse("https://novelphoenix.net/reverend-insanity.html").unwrap();
        let metadata = parse_novelphoenix_metadata(&url, html);
        assert_eq!(metadata.title, "Reverend Insanity");
        assert_eq!(metadata.author.as_deref(), Some("Gu Zhen Ren"));
        assert_eq!(metadata.tags, vec!["Fantasy", "Xianxia"]);
        assert_eq!(metadata.status.as_deref(), Some("Completed"));
    }

    #[test]
    fn generic_metadata_reads_json_ld_and_novel_author_meta() {
        let url = Url::parse("https://freewebnovel.com/novel/reverend-insanity").unwrap();

        let html = r#"
            <html><head>
              <meta property="og:title" content="Reverend Insanity">
              <script type="application/ld+json">
              {"@type":"Website","name":"FreeWebNovel"}
              </script>
              <script type="application/ld+json">
              {"@type":"Book","name":"Reverend Insanity",
               "author":[{"@type":"Person","name":"Gu Zhen Ren"},
                         {"@type":"Person","name":"蛊真人"}]}
              </script>
            </head><body></body></html>
        "#;
        let metadata = parse_generic_metadata(&url, html);
        assert_eq!(metadata.title, "Reverend Insanity");
        assert_eq!(
            metadata.authors,
            vec!["Gu Zhen Ren".to_string(), "蛊真人".to_string()]
        );
        assert_eq!(metadata.author.as_deref(), Some("Gu Zhen Ren"));

        let html = r#"
            <html><head>
              <meta property="og:title" content="Reverend Insanity">
              <meta property="og:novel:author" content="Gu Zhen Ren">
            </head><body></body></html>
        "#;
        let metadata = parse_generic_metadata(&url, html);
        assert_eq!(metadata.author.as_deref(), Some("Gu Zhen Ren"));

        let html = r#"
            <html><head>
              <meta property="og:title" content="Chrysalis">
              <meta property="og:author" content="RinoZ">
            </head><body></body></html>
        "#;
        let metadata = parse_generic_metadata(&url, html);
        assert_eq!(metadata.author.as_deref(), Some("RinoZ"));
    }

    #[test]
    fn extracts_chapter_numbers() {
        assert_eq!(
            chapter_number_from_url("https://novelfire.net/book/x/chapter-2334"),
            Some(2334)
        );
        assert_eq!(
            chapter_number_from_text("Chapter 19 - The beginning"),
            Some(19)
        );
        assert_eq!(
            chapter_number_from_url("https://novelphoenix.net/x/chapter-12-title.html"),
            Some(12)
        );
    }

    #[test]
    fn parses_toc_page() {
        let html = r#"
            <ul class="chapter-list">
              <li><a href="/book/x/chapter-1" title="Chapter 1 - Start">Chapter 1</a></li>
              <li><a href="/book/x/chapter-2" title="Chapter 2 - Next">Chapter 2</a></li>
            </ul>
            <a class="page-link" href="https://novelfire.net/book/x/chapters?page=24">24</a>
        "#;
        let document = Html::parse_document(html);
        let base = Url::parse("https://novelfire.net/book/x/chapters?page=1").unwrap();
        let chapters = parse_novelfire_chapters(&document, &base, 1);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].number, 1);
        assert_eq!(last_page_number(&document), Some(24));
    }

    #[test]
    fn parses_novelfull_detail_metadata() {
        let html = r#"
            <html><head>
              <meta property="og:title" content="Reverend Insanity | NovelFull">
              <meta property="og:image" content="https://novelfull.net/uploads/reverend-insanity.jpg">
              <meta name="description" content="Read Reverend Insanity on NovelFull.">
            </head><body>
              <div class="m-info"><div class="txt">
                <div class="item">
                  <a href="/author/Gu+Zhen+Ren" class="a1" title="Gu Zhen Ren">Gu Zhen Ren</a>,
                  <a href="/author/%E8%9B%8A%E7%9C%9F%E4%BA%BA" class="a1" title="蛊真人">蛊真人</a>
                </div>
                <div class="item">
                  <a href="/genre/Fantasy" class="a1">Fantasy</a>,
                  <a href="/genre/Xianxia" class="a1">Xianxia</a>
                </div>
                <div class="item"><a href="/latest-release-novel">Completed</a></div>
              </div></div>
              <div class="m-desc">
                <h1 class="tit">Reverend Insanity</h1>
                <h4 class="abstract">SUMMARY</h4>
                <div class="txt">Humans are clever in tens of thousands of ways, Gu are the true refined essences.</div>
              </div>
              <div id="list-chapter" data-total-chapters="2334"></div>
            </body></html>
        "#;
        let url = Url::parse("https://novelfull.net/reverend-insanity.html").unwrap();
        let metadata = parse_novelfull_metadata(&url, html);

        assert_eq!(metadata.title, "Reverend Insanity");
        assert_eq!(metadata.author.as_deref(), Some("Gu Zhen Ren"));
        assert_eq!(
            metadata.authors,
            vec!["Gu Zhen Ren".to_string(), "蛊真人".to_string()]
        );
        assert_eq!(metadata.chapter_count, Some(2334));
        assert_eq!(metadata.status.as_deref(), Some("Completed"));
        assert!(
            metadata
                .synopsis
                .as_deref()
                .unwrap()
                .starts_with("Humans are clever")
        );
        assert!(metadata.tags.contains(&"Fantasy".to_string()));
    }

    #[test]
    fn novelfull_title_falls_back_to_sanitised_og_title() {
        let html = r#"
            <html><head>
              <meta property="og:title" content="Reverend Insanity | Novelfull">
            </head><body></body></html>
        "#;
        let url = Url::parse("https://novelfull.net/reverend-insanity.html").unwrap();
        let metadata = parse_novelfull_metadata(&url, html);

        assert_eq!(metadata.title, "Reverend Insanity");
        assert_eq!(strip_site_suffix("A | B | NovelFull", "novelfull"), "A | B");
        assert_eq!(strip_site_suffix("Clean Title", "novelfull"), "Clean Title");
    }

    #[test]
    fn parses_novelfull_style_toc_and_ajax() {
        let html = r#"
            <div id="truyen" data-page-size="40" data-total-page="3" data-total-chapters="81"></div>
            <div id="list-chapter" data-novel-id="265" data-total-page="3" data-total-chapters="81"
                 data-list-url="/ajax-chapter-list">
              <ul id="idData">
                <li><a href="/book/chapter-1.html" title="Chapter 1"><span class="chapter-text">Chapter 1</span></a></li>
                <li><a href="/book/chapter-2.html" title="Chapter 2"><span class="chapter-text">Chapter 2</span></a></li>
              </ul>
              <a id="nextBtn" rel="next" href="/book.html?page=2">Next</a>
            </div>
        "#;
        let url = Url::parse("https://novelfull.net/book.html").unwrap();
        let document = Html::parse_document(html);
        let signals = generic_toc_signals(&document, html, &url);

        assert_eq!(signals.advertised_total, Some(81));
        assert_eq!(signals.pagination.total_pages, Some(3));
        assert_eq!(
            signals.pagination.page_template.as_deref(),
            Some("https://novelfull.net/book.html?page={page}")
        );
        let chapters = parse_generic_chapters(&document, &url, 1);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].number, 1);

        let archive = signals.ajax.expect("ajax archive");
        assert_eq!(archive.endpoint, "https://novelfull.net/ajax-chapter-list");
        assert_eq!(archive.novel_id, "265");
        assert_eq!(archive.total_pages, 3);
        assert!(archive.paged);
    }

    #[test]
    fn detects_path_and_next_page_pagination() {
        let path_html = r#"
            <ul class="chapter-list"><li><a href="/book/x/chapter-1">Chapter 1</a></li></ul>
            <a href="/book/x/page/2">2</a><a href="/book/x/page/7">7</a>
        "#;
        let path_document = Html::parse_document(path_html);
        let base = Url::parse("https://example.com/book/x").unwrap();
        let signals = pagination_signals(&path_document, &base);
        assert_eq!(signals.total_pages, Some(7));
        assert_eq!(
            signals.page_template.as_deref(),
            Some("https://example.com/book/x/page/{page}")
        );
        assert!(signals.detected);

        let next_html = r#"
            <ul class="chapter-list"><li><a href="/book/x/chapter-1">Chapter 1</a></li></ul>
            <link rel="next" href="/book/x/chapters/2">
        "#;
        let next_document = Html::parse_document(next_html);
        let signals = pagination_signals(&next_document, &base);
        assert_eq!(signals.total_pages, None);
        assert_eq!(
            signals.next_url.as_deref(),
            Some("https://example.com/book/x/chapters/2")
        );
        assert!(signals.detected);
    }

    #[test]
    fn parses_json_ld_chapter_item_list() {
        let html = r#"
            <script type="application/ld+json">
            {"@context":"https://schema.org","@graph":[
              {"@type":"ItemList","name":"Chapters","numberOfItems":2,"itemListElement":[
                {"@type":"ListItem","position":1,"url":"https://example.com/story/x/chapter-1/"},
                {"@type":"ListItem","position":2,"url":"https://example.com/story/x/chapter-2/"}
              ]}
            ]}
            </script>
        "#;
        let base = Url::parse("https://example.com/story/x/").unwrap();
        let (chapters, total) = json_ld_chapter_list(html, &base);
        assert_eq!(total, Some(2));
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].number, 1);
        assert_eq!(chapters[1].title, "Chapter 2");
    }

    #[test]
    fn expands_template_chapter_items() {
        let html = r#"
            <div class="chapter-archive-grid" data-chapter-archive-grid></div>
            <template data-chapter-item-template>
              <li data-chapter-item><a href="/book/x/chapter-1" title="Chapter 1">Chapter 1</a></li>
              <li data-chapter-item><a href="/book/x/chapter-2" title="Chapter 2">Chapter 2</a></li>
            </template>
        "#;
        let document = Html::parse_document(&expand_templates(html));
        let base = Url::parse("https://example.com/book/x").unwrap();
        let chapters = parse_generic_chapters(&document, &base, 1);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[1].number, 2);
    }

    #[test]
    fn discovers_novelping_chapter_archive() {
        let html = r#"
            <div id="rating" data-novel-id="prison-of-eternal-sky"></div>
            <div id="chapter-archive"></div>
            <script type="text/javascript">var ajaxUrl = 'https://novelping.com/ajax';</script>
        "#;
        let document = Html::parse_document(html);
        let base = Url::parse("https://novelping.com/book/prison-of-eternal-sky").unwrap();
        let archive = discover_ajax_archive(&document, html, &base).expect("archive");
        assert_eq!(
            archive.endpoint,
            "https://novelping.com/ajax/chapter-archive"
        );
        assert_eq!(archive.novel_id, "prison-of-eternal-sky");
        assert!(!archive.paged);
    }

    #[test]
    fn advertised_total_ignores_author_stats() {
        let html = r#"
            <div class="mfa-author__stats"><span>53 chapters</span></div>
            <button data-fictioneer-story-target="tab" data-target="chapters">31 Chapters</button>
        "#;
        let document = Html::parse_document(html);
        assert_eq!(advertised_chapter_total(&document, None), Some(31));
        assert_eq!(parse_chapter_count_text("1,226 Chapters"), Some(1226));
        assert_eq!(parse_chapter_count_text("Chapters: 12"), Some(12));
        assert_eq!(parse_chapter_count_text("Chapter 53"), None);
    }

    #[test]
    fn scoped_chapter_container_beats_page_navigation() {
        let html = r#"
            <div class="l-chapter"><a class="chapter-title" href="/book/x/chapter-53">Chapter 53</a></div>
            <div class="chapter-group__list">
              <a href="/book/x/chapter-1">Chapter 1</a>
              <a href="/book/x/chapter-2">Chapter 2</a>
            </div>
        "#;
        let document = Html::parse_document(html);
        let base = Url::parse("https://example.com/book/x").unwrap();
        let chapters = parse_generic_chapters(&document, &base, 1);
        assert_eq!(chapters.len(), 2);
        assert!(chapters.iter().all(|chapter| chapter.number != 53));
    }

    #[test]
    fn completeness_requires_real_evidence() {
        let empty = PaginationOutcome::default();
        let (complete, detail) = assess_generic_index(
            31,
            31,
            Some(31),
            false,
            None,
            None,
            None,
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(!complete);
        assert!(detail.contains("cannot be proven complete"));

        let (complete, detail) = assess_generic_index(
            31,
            31,
            Some(31),
            false,
            Some(53),
            None,
            None,
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(!complete);
        assert!(detail.contains("31 of 53 advertised"));

        let (complete, _) = assess_generic_index(
            53,
            53,
            Some(53),
            true,
            Some(53),
            None,
            None,
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(complete);

        let (complete, detail) = assess_generic_index(
            31,
            31,
            Some(31),
            false,
            None,
            None,
            Some(53),
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(!complete);
        assert!(detail.contains("latest chapter 53"));

        let (complete, _) = assess_generic_index(
            53,
            0,
            Some(53),
            false,
            None,
            Some(53),
            None,
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(complete);

        let (complete, detail) = assess_generic_index(
            31,
            0,
            Some(31),
            false,
            None,
            Some(53),
            None,
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(!complete);
        assert!(detail.contains("31 of 53"));

        let (complete, detail) = assess_generic_index(
            31,
            31,
            Some(40),
            false,
            None,
            Some(10),
            Some(53),
            &empty,
            false,
            &[],
            &[],
            None,
            false,
        );
        assert!(!complete);
        assert!(detail.contains("latest chapter 53"));

        let terminal = PaginationOutcome {
            expected_pages: Some(2),
            walked_pages: 2,
            reached_terminal: true,
            limit_exceeded: false,
        };
        let (complete, _) = assess_generic_index(
            2,
            2,
            Some(31),
            false,
            None,
            None,
            None,
            &terminal,
            true,
            &[],
            &[],
            None,
            false,
        );
        assert!(complete);

        let (complete, _) = assess_generic_index(
            31,
            31,
            Some(31),
            false,
            None,
            None,
            None,
            &empty,
            false,
            &[2],
            &[],
            None,
            false,
        );
        assert!(!complete);
    }

    #[test]
    fn ignores_non_chapter_json_ld_item_lists() {
        let html = r#"
            <script type="application/ld+json">
            {"@graph":[{"@type":"ItemList","name":"Related Novels","numberOfItems":9,"itemListElement":[
              {"@type":"ListItem","url":"https://example.com/story/other/"},
              {"@type":"ListItem","url":"https://example.com/story/another/"}
            ]}]}
            </script>
        "#;
        let base = Url::parse("https://example.com/story/x/").unwrap();
        let (chapters, total) = json_ld_chapter_list(html, &base);
        assert!(chapters.is_empty());
        assert_eq!(total, None);
    }

    #[test]
    fn pagination_ignores_unrelated_links_and_wordpress_ids() {
        let html = r#"
            <ul class="chapter-list"><li><a href="/book/x/chapter-1">Chapter 1</a></li></ul>
            <a href="/genre/fantasy?page=12">12</a>
            <a href="/book/x?p=123">post id</a>
        "#;
        let document = Html::parse_document(html);
        let base = Url::parse("https://example.com/book/x").unwrap();
        let signals = pagination_signals(&document, &base);
        assert_eq!(signals.total_pages, None);
        assert!(!signals.detected);

        let wordpress = Url::parse("https://example.com/book/x?p=123").unwrap();
        assert_eq!(generic_toc_url(&wordpress).query(), Some("p=123"));
    }

    #[test]
    fn generic_toc_url_drops_only_pagination_query() {
        let url = Url::parse("https://example.com/book/x?page=3&tab=chapters").unwrap();
        let clean = generic_toc_url(&url);
        assert_eq!(clean.query(), Some("tab=chapters"));

        let url = Url::parse("https://example.com/book/x?tab=chapters").unwrap();
        assert_eq!(generic_toc_url(&url).query(), Some("tab=chapters"));
    }

    #[test]
    fn reads_ajax_json_fragments() {
        let body = r#"{"code":200,"html":"<ul id=\"idData\"><li><a href=\"/x/chapter-1\">Chapter 1</a></li></ul>"}"#;
        let fragment = archive_fragment(body).unwrap();
        assert!(fragment.contains("chapter-1"));
        let error =
            archive_fragment(r#"{"code":404,"html":"","message":"Novel not found."}"#).unwrap_err();
        assert!(error.contains("404"));
    }

    fn index_fixture(complete: bool) -> NovelChapterIndex {
        NovelChapterIndex {
            source_url: "https://novelphoenix.net/mother-of-learning.html".to_string(),
            source: "novelphoenix.net".to_string(),
            title: None,
            total: 1,
            complete,
            detail: Some("fixture".to_string()),
            chapters: vec![NovelChapter {
                number: 1,
                title: "Chapter 1".to_string(),
                url: "https://novelphoenix.net/mother-of-learning/chapter-1".to_string(),
                volume: None,
                downloaded: false,
                file: None,
            }],
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        }
    }

    #[test]
    fn one_walk_lock_per_index_key() {
        let first = index_walk_lock("novelphoenix.net|https://novelphoenix.net/a.html");
        let again = index_walk_lock("novelphoenix.net|https://novelphoenix.net/a.html");
        let other = index_walk_lock("novelphoenix.net|https://novelphoenix.net/b.html");
        assert!(std::sync::Arc::ptr_eq(&first, &again));
        assert!(!std::sync::Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn index_cache_key_normalizes_pagination_and_fragment() {
        let base = Url::parse("https://novelphoenix.net/mother-of-learning.html").unwrap();
        let paged =
            Url::parse("https://novelphoenix.net/mother-of-learning.html?page=3#chapters").unwrap();
        assert_eq!(index_cache_key(&base), index_cache_key(&paged));

        let other = Url::parse("https://novelfire.net/book/reverend-insanity").unwrap();
        assert_ne!(index_cache_key(&base), index_cache_key(&other));
    }

    #[test]
    fn complete_index_round_trips_and_expires() {
        let directory = std::env::temp_dir().join(format!(
            "novelforge-index-cache-{}-{}",
            std::process::id(),
            now_unix_seconds()
        ));
        let key = "novelphoenix.net|https://novelphoenix.net/mother-of-learning.html";
        let index = index_fixture(true);
        write_cached_index_at(&directory, key, &index, 1_000);

        let hit = read_cached_index_at(&directory, key, 1_060, 3_600).expect("fresh cache hit");
        assert!(hit.cached);
        assert_eq!(hit.cache_age_seconds, Some(60));
        assert_eq!(hit.total, 1);
        assert_eq!(hit.chapters.len(), 1);

        assert!(read_cached_index_at(&directory, key, 1_000 + 3_601, 3_600).is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn incomplete_index_is_never_reused() {
        let directory = std::env::temp_dir().join(format!(
            "novelforge-index-cache-incomplete-{}-{}",
            std::process::id(),
            now_unix_seconds()
        ));
        let key = "example.com|https://example.com/book";
        write_cached_index_at(&directory, key, &index_fixture(false), 1_000);

        assert!(read_cached_index_at(&directory, key, 1_000, 3_600).is_none());
        assert!(!index_cache_path(&directory, key).exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn cache_fields_reach_the_frontend_payload() {
        let mut index = index_fixture(true);
        index.cached = true;
        index.cache_age_seconds = Some(42);
        let json = serde_json::to_value(&index).expect("serialise index");
        assert_eq!(json["cached"], true);
        assert_eq!(json["cacheAgeSeconds"], 42);
    }

    #[test]
    fn author_page_reads_only_the_author_shelf() {
        let html = r#"
            <html><body>
              <div class="breadcrumb">
                <a title="Author: Gu Zhen Ren" href="https://novelfire.net/author/gu-zhen-ren">Author: Gu Zhen Ren</a>
              </div>
              <h1>Gu Zhen Ren Author Novels</h1>
              <ul class="novel-list grid">
                <li><a title="Reverend Insanity" href="/book/reverend-insanity"><h4>Reverend Insanity</h4></a></li>
                <li><a title="Infinite Bloodcore" href="/book/infinite-bloodcore"><h4>Infinite Bloodcore</h4></a></li>
                <li><a title="Mysteries of Immortal Puppet Master" href="/book/mysteries-of-immortal-puppet-master"><h4>Mysteries of Immortal Puppet Master</h4></a></li>
                <li><a title="Advent of Immortal Truth" href="/book/advent-of-immortal-truth"><h4>Advent of Immortal Truth</h4></a></li>
              </ul>
              <h2>You may also Like</h2>
              <ul class="novel-list grid">
                <li><a title="No Game No Life" href="/book/no-game-no-life"><h4>No Game No Life</h4></a></li>
              </ul>
            </body></html>
        "#;
        let page = Url::parse("https://novelfire.net/author/gu-zhen-ren").unwrap();
        let parsed = parse_author_page(&page, html);
        assert_eq!(parsed.author.as_deref(), Some("Gu Zhen Ren"));
        let titles: Vec<&str> = parsed
            .novels
            .iter()
            .map(|novel| novel.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec![
                "Reverend Insanity",
                "Infinite Bloodcore",
                "Mysteries of Immortal Puppet Master",
                "Advent of Immortal Truth"
            ]
        );
        assert_eq!(
            parsed.novels[0].url,
            "https://novelfire.net/book/reverend-insanity"
        );
    }

    #[test]
    fn author_page_prefers_json_ld_item_list() {
        let html = r#"
            <html><body>
              <script type="application/ld+json">
                {"@context":"https://schema.org","@graph":[
                  {"@type":"CollectionPage","name":"Novels by Gu Zhen Ren | NovelPhoenix"},
                  {"@type":"ItemList","numberOfItems":2,"itemListElement":[
                    {"@type":"ListItem","position":1,"url":"https://novelphoenix.net/mysteries-of-immortal-puppet-master.html","name":"Mysteries of Immortal Puppet Master"},
                    {"@type":"ListItem","position":2,"url":"https://novelphoenix.net/reverend-insanity.html","name":"Reverend Insanity"}]}]}
              </script>
              <a href="/recommended.html" title="Unrelated">Unrelated</a>
            </body></html>
        "#;
        let page = Url::parse("https://novelphoenix.net/author/Gu+Zhen+Ren").unwrap();
        let parsed = parse_author_page(&page, html);
        assert_eq!(parsed.author.as_deref(), Some("Gu Zhen Ren"));
        assert_eq!(parsed.novels.len(), 2);
        assert_eq!(
            parsed.novels[0].title,
            "Mysteries of Immortal Puppet Master"
        );
        assert_eq!(parsed.novels[1].title, "Reverend Insanity");
        assert_eq!(
            parsed.novels[1].url,
            "https://novelphoenix.net/reverend-insanity.html"
        );
    }

    fn titled_chapter(number: u32, title: &str, url: &str) -> NovelChapter {
        NovelChapter {
            number,
            title: title.to_string(),
            url: url.to_string(),
            volume: None,
            downloaded: false,
            file: None,
        }
    }

    fn index_of(source: &str, chapters: Vec<NovelChapter>) -> NovelChapterIndex {
        let total = chapters.len() as u32;
        NovelChapterIndex {
            source_url: format!("https://{source}/book"),
            source: source.to_string(),
            title: None,
            total,
            complete: true,
            detail: None,
            chapters,
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        }
    }

    #[test]
    fn bare_chapter_title_matches_numbering_only_titles() {
        for title in [
            "Chapter 1",
            "Chapter 9 (END)",
            "Chapter #12",
            "Chapter 3 - 3",
            "Chapter 3 – 3",
            "12",
            "  Chapter 40  ",
        ] {
            assert!(bare_chapter_title(title), "{title:?} should be bare");
        }
        for title in [
            "Chapter 1 - Crimson",
            "Chapter 1: The heart of a demon",
            "The heart of a demon never has regret even in death",
            "Chapter 2 Situation",
        ] {
            assert!(!bare_chapter_title(title), "{title:?} carries a name");
        }
    }

    #[test]
    fn donor_chapter_name_strips_repeated_numbering() {
        assert_eq!(
            donor_chapter_name(
                "Chapter 1 - 1: The heart of a demon never has regret even in death",
                1
            )
            .as_deref(),
            Some("The heart of a demon never has regret even in death")
        );
        assert_eq!(
            donor_chapter_name(
                "Chapter 2 - Going back in time with 500 years of knowledge",
                2
            )
            .as_deref(),
            Some("Going back in time with 500 years of knowledge")
        );
        assert_eq!(
            donor_chapter_name("Chapter 5 - Crimson", 5).as_deref(),
            Some("Crimson")
        );
        assert_eq!(
            donor_chapter_name(
                "Chapter 551 (END) - Epilogue 5 – The Eternity and Epilogue (Complete)",
                551
            )
            .as_deref(),
            Some("Epilogue 5 – The Eternity and Epilogue (Complete)")
        );
        // A donor that only repeats its own number has no name to give.
        assert_eq!(donor_chapter_name("Chapter 7", 7), None);
        assert_eq!(donor_chapter_name("Chapter 7 - 7", 7), None);
    }

    #[test]
    fn display_chapter_number_prefers_the_visible_number() {
        // NovelFire numbers ORV's first chapter `chapter-1` but labels it
        // "Chapter 0"; NovelPhoenix labels it "Chapter 0" too.
        let novelfire = titled_chapter(
            1,
            "Chapter 0",
            "https://novelfire.net/book/omniscient-readers-viewpoint/chapter-1",
        );
        assert_eq!(display_chapter_number(&novelfire), Some(0));
    }

    #[test]
    fn adopt_chapter_titles_matches_by_visible_number_and_keeps_existing_names() {
        let mut target = index_of(
            "novelphoenix.net",
            vec![
                titled_chapter(
                    1,
                    "Chapter 1",
                    "https://novelphoenix.net/book/chapter-1.html",
                ),
                titled_chapter(
                    2,
                    "Chapter 2",
                    "https://novelphoenix.net/book/chapter-2.html",
                ),
                titled_chapter(
                    3,
                    "Chapter 3",
                    "https://novelphoenix.net/book/chapter-3.html",
                ),
                titled_chapter(
                    4,
                    "Chapter 4 - The source already named this",
                    "https://novelphoenix.net/book/chapter-4.html",
                ),
            ],
        );
        // NovelFire's URLs are one ahead of the visible chapter number.
        let donor = index_of(
            "novelfire.net",
            vec![
                titled_chapter(
                    2,
                    "Chapter 1 - First",
                    "https://novelfire.net/book/chapter-2",
                ),
                titled_chapter(
                    3,
                    "Chapter 2 - Second",
                    "https://novelfire.net/book/chapter-3",
                ),
                titled_chapter(
                    4,
                    "Chapter 3 - Third",
                    "https://novelfire.net/book/chapter-4",
                ),
                titled_chapter(
                    5,
                    "Chapter 4 - Donor copy",
                    "https://novelfire.net/book/chapter-5",
                ),
            ],
        );

        assert!(adopt_chapter_titles(&mut target, &donor));
        let titles: Vec<&str> = target
            .chapters
            .iter()
            .map(|chapter| chapter.title.as_str())
            .collect();
        assert_eq!(
            titles,
            [
                "First",
                "Second",
                "Third",
                "Chapter 4 - The source already named this"
            ]
        );
        // The target keeps its own numbering and download URLs.
        assert_eq!(target.chapters[0].number, 1);
        assert_eq!(
            target.chapters[0].url,
            "https://novelphoenix.net/book/chapter-1.html"
        );
        assert_eq!(target.title_source.as_deref(), Some("novelfire.net"));
        assert!(
            target
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("Filled 3 numbered chapter name(s) from novelfire.net")
        );
    }

    #[test]
    fn adopt_chapter_titles_rejects_low_coverage_and_changes_nothing() {
        let original = index_of(
            "novelphoenix.net",
            vec![
                titled_chapter(
                    1,
                    "Chapter 1",
                    "https://novelphoenix.net/book/chapter-1.html",
                ),
                titled_chapter(
                    2,
                    "Chapter 2",
                    "https://novelphoenix.net/book/chapter-2.html",
                ),
                titled_chapter(
                    3,
                    "Chapter 3",
                    "https://novelphoenix.net/book/chapter-3.html",
                ),
                titled_chapter(
                    4,
                    "Chapter 4",
                    "https://novelphoenix.net/book/chapter-4.html",
                ),
            ],
        );
        let mut target = original.clone();
        let donor = index_of(
            "novelfire.net",
            vec![titled_chapter(
                1,
                "Chapter 1 - Only one",
                "https://novelfire.net/book/chapter-1",
            )],
        );

        assert!(!adopt_chapter_titles(&mut target, &donor));
        let unchanged: Vec<(&str, &str)> = target
            .chapters
            .iter()
            .map(|chapter| (chapter.title.as_str(), chapter.url.as_str()))
            .collect();
        let expected: Vec<(&str, &str)> = original
            .chapters
            .iter()
            .map(|chapter| (chapter.title.as_str(), chapter.url.as_str()))
            .collect();
        assert_eq!(unchanged, expected);
        assert_eq!(target.title_source, None);
    }

    #[test]
    fn adopt_chapter_titles_rejects_a_shifted_donor() {
        // Target: 3 numbered rows, then 7 rows it already names.
        let mut chapters = Vec::new();
        for number in 1..=3 {
            chapters.push(titled_chapter(
                number,
                &format!("Chapter {number}"),
                &format!("https://novelphoenix.net/book/chapter-{number}.html"),
            ));
        }
        for number in 4..=10 {
            chapters.push(titled_chapter(
                number,
                &format!("Chapter {number} - Real {number}"),
                &format!("https://novelphoenix.net/book/chapter-{number}.html"),
            ));
        }
        let original = index_of("novelphoenix.net", chapters.clone());
        let mut target = original.clone();

        // Donor is one visible number behind: its "Chapter 3" carries the name
        // that belongs to target chapter 4.
        let donor = index_of(
            "novelfire.net",
            (1..=10)
                .map(|number| {
                    titled_chapter(
                        number - 1,
                        &format!("Chapter {} - Real {number}", number - 1),
                        &format!("https://novelfire.net/book/chapter-{number}"),
                    )
                })
                .collect(),
        );

        assert!(!adopt_chapter_titles(&mut target, &donor));
        let unchanged: Vec<(&str, &str)> = target
            .chapters
            .iter()
            .map(|chapter| (chapter.title.as_str(), chapter.url.as_str()))
            .collect();
        let expected: Vec<(&str, &str)> = original
            .chapters
            .iter()
            .map(|chapter| (chapter.title.as_str(), chapter.url.as_str()))
            .collect();
        assert_eq!(unchanged, expected);
        assert_eq!(target.title_source, None);
    }

    #[test]
    fn chapter_header_names_are_recognised_from_content() {
        assert_eq!(
            name_from_header("Episode 1 – Starting the Paid Service (1)").as_deref(),
            Some("Starting the Paid Service (1)")
        );
        assert_eq!(
            name_from_header("Chapter 2: Episode 1 – Starting the Paid Service (2)").as_deref(),
            Some("Starting the Paid Service (2)")
        );
        assert_eq!(
            name_from_header("Prologue – There are three ways to survive in a ruined world.")
                .as_deref(),
            Some("There are three ways to survive in a ruined world")
        );
        assert_eq!(
            name_from_header("Chapter 276: Episode 52. ■■ (1)").as_deref(),
            Some("■■ (1)")
        );
        // Prose and already-named rows must never be mistaken for a heading.
        assert_eq!(name_from_header("Chapter 62 - War of Kings (4)"), None);
        assert_eq!(
            name_from_header("I usually introduced myself to people."),
            None
        );
        assert_eq!(name_from_header("Chapter 1"), None);
        assert_eq!(
            name_from_header("Episode 13 – War of Kings (3)").as_deref(),
            Some("War of Kings (3)")
        );
    }

    #[test]
    fn chapter_name_is_read_from_the_content_container() {
        let html = r#"
            <html><body>
              <div class="txt fwn-reader-txt">
                <div id="chapter-content" class="chapter-c">
                  <p>Please support the translator.</p>
                  <p>Episode 1 – Starting the Paid Service (1)</p>
                  <p>“I’m Dokja.”</p>
                </div>
              </div>
            </body></html>
        "#;
        assert_eq!(
            chapter_name_from_html(html).as_deref(),
            Some("Starting the Paid Service (1)")
        );

        // Some rows print only the name on the first line.
        let unlabelled = r#"
            <html><body><div id="chapter-content">
              <p>Starting the Paid Service (3)</p>
              <p>People reacted differently after the dokkaebi disappeared.</p>
            </div></body></html>
        "#;
        assert_eq!(
            chapter_name_from_html(unlabelled).as_deref(),
            Some("Starting the Paid Service (3)")
        );

        let prose_only = r#"
            <html><body><div id="chapter-content"><p>“I’m Dokja.”</p></div></body></html>
        "#;
        assert_eq!(chapter_name_from_html(prose_only), None);

        // A short sentence must not be mistaken for a name.
        let sentence = r#"
            <html><body><div id="chapter-content">
              <p>Laughter emerged.</p>
              <p>The file extension was TXT.</p>
            </div></body></html>
        "#;
        assert_eq!(chapter_name_from_html(sentence), None);
    }

    #[test]
    fn title_lookup_runs_only_when_the_source_leaves_rows_unnamed() {
        // A mixed index (NovelPhoenix RI: names from 348 on, numbers before
        // that) still needs the donor lookup for its numbered rows.
        let mixed = index_of(
            "novelphoenix.net",
            vec![
                titled_chapter(
                    1,
                    "Chapter 1",
                    "https://novelphoenix.net/book/chapter-1.html",
                ),
                titled_chapter(
                    2,
                    "Chapter 2",
                    "https://novelphoenix.net/book/chapter-2.html",
                ),
                titled_chapter(
                    3,
                    "Chapter 3 - Please go aside and scram",
                    "https://novelphoenix.net/book/chapter-3.html",
                ),
                titled_chapter(
                    4,
                    "Chapter 4 - Gu Yue Fang Yuan",
                    "https://novelphoenix.net/book/chapter-4.html",
                ),
            ],
        );
        assert!(title_lookup_needed(&mixed));

        // Lord of the Mysteries carries real names on NovelPhoenix already.
        let named = index_of(
            "novelphoenix.net",
            vec![
                titled_chapter(
                    1,
                    "Chapter 1 - Crimson",
                    "https://novelphoenix.net/book/chapter-1-crimson.html",
                ),
                titled_chapter(
                    2,
                    "Chapter 2 - Situation",
                    "https://novelphoenix.net/book/chapter-2-situation.html",
                ),
            ],
        );
        assert!(!title_lookup_needed(&named));

        // One failed lookup is remembered; a cache hit must not search again.
        let mut attempted = mixed.clone();
        attempted.title_lookup_attempted = true;
        assert!(!title_lookup_needed(&attempted));

        // An incomplete index is never worth borrowing names for.
        let mut incomplete = mixed.clone();
        incomplete.complete = false;
        assert!(!title_lookup_needed(&incomplete));

        // A single chapter has nothing to align against.
        let single = index_of(
            "novelphoenix.net",
            vec![titled_chapter(
                1,
                "Chapter 1",
                "https://novelphoenix.net/book/chapter-1.html",
            )],
        );
        assert!(!title_lookup_needed(&single));

        // One stray numbered row in a long, otherwise named book is not worth
        // a donor search, but its own chapter page is a single request.
        let mut long = Vec::new();
        for number in 1..=200 {
            let title = if number == 90 {
                "Chapter 90".to_string()
            } else {
                format!("Chapter {number} - A real name")
            };
            long.push(titled_chapter(
                number,
                &title,
                &format!("https://novelphoenix.net/book/chapter-{number}.html"),
            ));
        }
        let long = index_of("novelphoenix.net", long);
        assert!(!title_lookup_needed(&long));
        assert!(title_pass_needed(&long));
    }
}
