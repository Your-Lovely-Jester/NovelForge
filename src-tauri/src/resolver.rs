//! Turns a plain title or synonym into a direct crawler URL.
//!
//! The local crawler is the authority on what it can download, so the resolver
//! first asks the two required novel sources directly. Only when that fails does
//! it consult public title databases (AniList, Wikipedia) to expand a synonym
//! into a canonical English title and search the sites again.

use crate::cache;
use crate::catalogue;
use crate::error::{AppError, AppResult};
use crate::metadata;
use crate::model::{
    Engine, MangaSource, MangaSources, NovelMetadata, NovelResult, ResolvedTitle, SearchResponse,
    SourceSearchFailure,
};
use crate::settings::{Settings, now_ms, now_secs};
use crate::sources;
use crate::spawn;
use crate::tools::{ResolvedTool, resolve_tool};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use url::Url;

const SEARCH_LIMIT: &str = "10";
const SEARCH_TIMEOUT_SECS: &str = "20";
const SEARCH_COMMAND_BUDGET_SECS: u64 = 40;
const SEARCH_CONCURRENCY: usize = 4;
const SEARCH_CACHE_TTL_SECONDS: u64 = 15 * 60;
const SUMMARY_CACHE_TTL_SECONDS: u64 = 15 * 60;
const AUTHOR_PAGE_CACHE_TTL_SECONDS: u64 = 15 * 60;
/// Minimum title score a plain title search must reach to count as a match.
const MATCH_THRESHOLD: f32 = 0.72;
/// Stronger bar for an alias/synonym match, where a false positive is likelier.
/// A token-level alias match stays below [`CONFIRM_THRESHOLD`], so the studio
/// asks for confirmation and the automatic download path refuses it.
const ALIAS_THRESHOLD: f32 = 0.8;
/// Minimum author similarity before an author-mode result is accepted.
const AUTHOR_THRESHOLD: f32 = 0.7;
/// Matches below this are usable in the UI but are refused by the automatic
/// download path, so a title job can never silently pick a weak guess.
pub const CONFIRM_THRESHOLD: f32 = 0.85;
/// How many search hits to inspect when verifying authors or aliases.
const RESOLVE_CANDIDATES: usize = 8;
const RESOLVE_FETCH_CONCURRENCY: usize = 6;
const RESOLVE_SUMMARY_TIMEOUT_SECS: u64 = 8;
/// The first verification wave is deliberately small: an exact author match
/// ends verification, so no more than these three pages are fetched.
const RESOLVE_FIRST_WAVE: usize = 3;
/// An author match this strong is treated as exact and ends verification.
const EXACT_AUTHOR_SCORE: f32 = 0.95;
/// Author pages replace search + detail verification for sources that expose
/// one, so they get their own (shorter) budget.
const AUTHOR_PAGE_TIMEOUT_SECS: u64 = 12;
const AUTHOR_PAGE_CONCURRENCY: usize = 4;

/// Title mode is the default. Author mode verifies the author field on the
/// source page before accepting a hit instead of trusting loose search text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveMode {
    Auto,
    Author,
}

impl ResolveMode {
    pub fn parse(value: Option<&str>) -> Self {
        match value
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "author" | "authors" | "by" => Self::Author,
            _ => Self::Auto,
        }
    }
}

/// Describes how much of a response came from the short-lived disk cache.
#[derive(Debug, Clone, Copy, Default)]
struct CacheStamp {
    cached: bool,
    age_seconds: Option<u64>,
}

impl CacheStamp {
    fn observe(&mut self, age_seconds: Option<u64>) {
        if let Some(age) = age_seconds {
            self.cached = true;
            self.age_seconds = Some(self.age_seconds.map_or(age, |current| current.max(age)));
        }
    }

    fn merge(&mut self, other: Self) {
        if other.cached {
            self.observe(other.age_seconds);
        }
    }

    fn from_search(response: &SearchResponse) -> Self {
        Self {
            cached: response.cached,
            age_seconds: response.cache_age_seconds,
        }
    }
}

fn resolver_cache_dir() -> Option<std::path::PathBuf> {
    cache::directory("NOVELFORGE_SEARCH_CACHE_DIR", "search-cache")
}

fn search_cache_key(source: &str, query: &str) -> String {
    format!(
        "search::{}::{}",
        source.trim().trim_end_matches('/').to_ascii_lowercase(),
        normalize(query)
    )
}

fn cached_search_batch(source: &str, query: &str, now: u64) -> Option<(SourceBatch, u64)> {
    let directory = resolver_cache_dir()?;
    let key = search_cache_key(source, query);
    let (batch, age) = cache::read_at::<SourceBatch>(
        &directory,
        &key,
        now,
        cache::ttl_seconds("NOVELFORGE_SEARCH_TTL_SECS", SEARCH_CACHE_TTL_SECONDS),
    )?;
    Some((batch, age))
}

fn store_search_batch(batch: &SourceBatch, query: &str) {
    if batch.failure.is_some() {
        return;
    }
    let Some(directory) = resolver_cache_dir() else {
        return;
    };
    cache::write_at(
        &directory,
        &search_cache_key(&batch.source, query),
        batch,
        now_secs(),
    );
}

fn summary_cache_key(url: &str) -> String {
    format!("summary::{url}")
}

async fn fetch_summary_cached(url: &str) -> FetchedSummary {
    let now = now_secs();
    if let Some(directory) = resolver_cache_dir()
        && let Some((metadata, age)) = cache::read_at::<NovelMetadata>(
            &directory,
            &summary_cache_key(url),
            now,
            cache::ttl_seconds("NOVELFORGE_SEARCH_TTL_SECS", SUMMARY_CACHE_TTL_SECONDS),
        )
    {
        return FetchedSummary {
            metadata: Some(metadata),
            cache_age_seconds: Some(age),
        };
    }
    match metadata::fetch_novel_summary(url).await {
        Ok(metadata) => {
            if let Some(directory) = resolver_cache_dir() {
                cache::write_at(&directory, &summary_cache_key(url), &metadata, now_secs());
            }
            FetchedSummary {
                metadata: Some(metadata),
                cache_age_seconds: None,
            }
        }
        Err(_) => FetchedSummary {
            metadata: None,
            cache_age_seconds: None,
        },
    }
}

fn author_cache_key(url: &str) -> String {
    format!("author::{url}")
}

fn cached_author_page(url: &str, now: u64) -> Option<(metadata::AuthorPage, u64)> {
    let directory = resolver_cache_dir()?;
    cache::read_at::<metadata::AuthorPage>(
        &directory,
        &author_cache_key(url),
        now,
        cache::ttl_seconds("NOVELFORGE_SEARCH_TTL_SECS", AUTHOR_PAGE_CACHE_TTL_SECONDS),
    )
}

fn store_author_page(url: &str, page: &metadata::AuthorPage) {
    let Some(directory) = resolver_cache_dir() else {
        return;
    };
    cache::write_at(&directory, &author_cache_key(url), page, now_secs());
}

/// Searches exactly the selected source URLs. An empty selection means the
/// NovelForge registry sources; the literal `all` means every enabled,
/// search-capable, non-manga lncrawl source from the catalogue snapshot.
pub async fn search_novels_from(
    settings: &Settings,
    query: &str,
    selected: &[String],
) -> SearchResponse {
    let query = query.trim().to_string();
    let (targets, failures) = resolve_targets(selected);
    search_targets(settings, query, targets, failures).await
}

/// Turns the UI selection into concrete source base URLs. An empty selection
/// is the NovelForge registry (not “all”); `all` expands to every searchable
/// non-manga catalogue source.
fn resolve_targets(selected: &[String]) -> (Vec<String>, Vec<SourceSearchFailure>) {
    let mut failures = Vec::new();
    let mut targets: Vec<String> = Vec::new();
    if selected.is_empty() {
        targets.extend(
            sources::search_sources()
                .iter()
                .filter_map(|profile| profile.search_url.map(str::to_string)),
        );
    } else {
        for value in selected {
            if value.eq_ignore_ascii_case("all") {
                targets.extend(catalogue::searchable().into_iter().map(|entry| entry.url));
                continue;
            }
            match catalogue::find_searchable(value) {
                Some(entry) => targets.push(entry.url),
                None => failures.push(SourceSearchFailure {
                    source: value.clone(),
                    label: source_label(value),
                    reason: "This source is direct-URL only; it does not offer title search."
                        .to_string(),
                }),
            }
        }
    }
    targets.sort();
    targets.dedup();
    (targets, failures)
}

async fn search_targets(
    settings: &Settings,
    query: String,
    targets: Vec<String>,
    mut failures: Vec<SourceSearchFailure>,
) -> SearchResponse {
    let now = now_secs();
    let mut batches = Vec::new();
    let mut pending = Vec::new();
    for source in &targets {
        match cached_search_batch(source, &query, now) {
            Some((mut batch, age)) => {
                batch.cache_age_seconds = Some(age);
                batches.push(batch);
            }
            None => pending.push(source.clone()),
        }
    }
    let mut stamp = CacheStamp::default();
    for batch in &batches {
        stamp.observe(batch.cache_age_seconds);
    }

    if !pending.is_empty() {
        match resolve_tool(settings, Engine::Webnovel) {
            Some(tool) => {
                let semaphore = Arc::new(Semaphore::new(SEARCH_CONCURRENCY));
                let mut jobs = JoinSet::new();
                for source in &pending {
                    let tool = tool.clone();
                    let query = query.clone();
                    let source = source.clone();
                    let semaphore = Arc::clone(&semaphore);
                    jobs.spawn(async move {
                        let _permit = semaphore.acquire_owned().await;
                        search_source(&tool, &source, &query).await
                    });
                }

                while let Some(outcome) = jobs.join_next().await {
                    match outcome {
                        Ok(batch) => {
                            store_search_batch(&batch, &query);
                            batches.push(batch);
                        }
                        Err(error) => failures.push(SourceSearchFailure {
                            source: "search task".to_string(),
                            label: "search task".to_string(),
                            reason: format!("The source search task stopped unexpectedly: {error}"),
                        }),
                    }
                }
            }
            None => failures.push(SourceSearchFailure {
                source: "lncrawl".to_string(),
                label: "lncrawl".to_string(),
                reason: "The crawler is not installed, so no source could be searched.".to_string(),
            }),
        }
    }

    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let mut searched = Vec::new();
    for batch in batches {
        searched.push(batch.source.clone());
        if let Some(reason) = batch.failure {
            failures.push(SourceSearchFailure {
                source: batch.source,
                label: batch.label,
                reason,
            });
        }
        for result in batch.results {
            if seen.insert(result.url.clone()) {
                results.push(result);
            }
        }
    }
    searched.sort();
    failures.sort_by(|left, right| left.source.cmp(&right.source));

    results.sort_by(|left, right| {
        relevance(&query, &right.title)
            .partial_cmp(&relevance(&query, &left.title))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| source_rank(&left.source).cmp(&source_rank(&right.source)))
            .then_with(|| left.title.len().cmp(&right.title.len()))
    });

    SearchResponse {
        query,
        results,
        searched,
        failures,
        cached: stamp.cached,
        cache_age_seconds: stamp.age_seconds,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceBatch {
    source: String,
    label: String,
    results: Vec<NovelResult>,
    failure: Option<String>,
    #[serde(skip, default)]
    cache_age_seconds: Option<u64>,
}

async fn search_source(tool: &ResolvedTool, source: &str, query: &str) -> SourceBatch {
    let started = Instant::now();
    let label = source_label(source);
    let mut command = Command::new(spawn::native_path(&tool.program));
    command
        .args(&tool.prefix)
        .args([
            "search",
            query,
            "-s",
            source,
            "-l",
            SEARCH_LIMIT,
            "-t",
            SEARCH_TIMEOUT_SECS,
        ])
        .kill_on_drop(true)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env("TERM", "dumb")
        .env("COLUMNS", "240")
        .env("PYTHONUNBUFFERED", "1");
    spawn::hide_console(&mut command);

    let output = match tokio::time::timeout(
        Duration::from_secs(SEARCH_COMMAND_BUDGET_SECS),
        command.output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return SourceBatch {
                source: source.to_string(),
                label,
                results: Vec::new(),
                failure: Some(format!("Could not start lncrawl: {error}")),
                cache_age_seconds: None,
            };
        }
        Err(_) => {
            return SourceBatch {
                source: source.to_string(),
                label,
                results: Vec::new(),
                failure: Some(format!(
                    "No answer within {} s (elapsed {:.1} s).",
                    SEARCH_COMMAND_BUDGET_SECS,
                    started.elapsed().as_secs_f64()
                )),
                cache_age_seconds: None,
            };
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if std::env::var_os("NOVELFORGE_QA_SEARCH_DEBUG").is_some() {
        eprintln!(
            "[search:debug] {source} tool={} status={} stdout={} stderr={}\n{}",
            tool.display,
            output.status,
            text.len(),
            stderr.len(),
            text.chars().take(600).collect::<String>()
        );
    }
    let results = parse_search_output(&text, &label);
    let failure = if !output.status.success() {
        Some(
            last_error_line(&stderr)
                .unwrap_or_else(|| format!("lncrawl exited with status {}", output.status)),
        )
    } else if results.is_empty() && !stderr.trim().is_empty() {
        // lncrawl can print a traceback and still exit 0 for a rejected source.
        last_error_line(&stderr)
    } else {
        None
    };

    SourceBatch {
        source: source.to_string(),
        label,
        results,
        failure,
        cache_age_seconds: None,
    }
}

fn last_error_line(stderr: &str) -> Option<String> {
    const ERROR_WORDS: &[&str] = &[
        "error",
        "exception",
        "traceback",
        "failed",
        "failure",
        "refused",
        "timeout",
        "timed out",
        "forbidden",
        "blocked",
        "unavailable",
    ];
    for raw in stderr.lines().rev() {
        let stripped = strip_ansi_escapes::strip(raw);
        let line = String::from_utf8_lossy(&stripped);
        let line = line.rsplit('\r').next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with("Searching:") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if ERROR_WORDS.iter().any(|word| lower.contains(word)) {
            return Some(line.chars().take(240).collect());
        }
    }
    None
}

fn parse_search_output(text: &str, source: &str) -> Vec<NovelResult> {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    // The leading emoji is optional so a title that Rich wrapped onto a second
    // line can be completed from its continuation.
    let title_re = TITLE.get_or_init(|| {
        Regex::new(r"^(?:📖\s*)?(.+?)\s+\((\d+)\s+results?\)\s*$").expect("valid title regex")
    });

    let mut results = Vec::new();
    let mut current: Option<NovelResult> = None;
    let mut pending_title: Option<String> = None;
    let mut expect_url = false;
    let mut url_buffer = String::new();

    for raw in text.lines() {
        let stripped = strip_ansi_escapes::strip(raw);
        let line = String::from_utf8_lossy(&stripped);
        let line = line.rsplit('\r').next().unwrap_or_default();
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if expect_url {
            if line.starts_with(' ') || line.starts_with('\t') || trimmed.contains('|') {
                if let Some(result) = current.as_mut()
                    && result.url.is_empty()
                {
                    result.url = std::mem::take(&mut url_buffer);
                }
                expect_url = false;
                url_buffer.clear();
            } else {
                url_buffer.push_str(trimmed);
                continue;
            }
        }

        if let Some(rest) = trimmed.strip_prefix('➡') {
            let rest = rest.trim();
            if let Some(result) = current.as_mut() {
                if rest.is_empty() {
                    expect_url = true;
                } else {
                    result.url = rest.to_string();
                }
            }
            continue;
        }

        let candidate = match pending_title.take() {
            Some(previous) => format!("{previous} {}", trimmed.trim_start_matches('📖').trim()),
            None => trimmed.trim_start_matches('📖').trim().to_string(),
        };

        if let Some(captures) = title_re.captures(&candidate) {
            if let Some(result) = current.take()
                && !result.url.is_empty()
            {
                results.push(result);
            }
            current = Some(NovelResult {
                title: captures[1].trim().to_string(),
                url: String::new(),
                source: source.to_string(),
                detail: None,
            });
            continue;
        }

        if pending_title.is_some() || trimmed.starts_with('📖') {
            // The title is longer than the terminal width; keep collecting
            // until the "(N results)" suffix arrives.
            pending_title = Some(candidate);
            continue;
        }

        let Some(result) = current.as_mut() else {
            continue;
        };

        if !result.url.is_empty() && result.detail.is_none() {
            let detail = trimmed.trim_matches('|').trim();
            if !detail.is_empty() && detail != "|" && !detail.starts_with("Searching:") {
                result.detail = Some(detail.to_string());
            }
        }
    }

    if expect_url
        && let Some(result) = current.as_mut()
        && result.url.is_empty()
    {
        result.url = url_buffer;
    }

    if let Some(result) = current
        && !result.url.is_empty()
    {
        results.push(result);
    }

    results
}

/// Resolve a request input into a concrete source URL using the registry
/// sources. Downloads use this strict form; the studio calls
/// [`resolve_request_from`] so a source filter and author mode can be passed.
pub async fn resolve_request(
    settings: &Settings,
    engine: Engine,
    input: &str,
) -> AppResult<ResolvedTitle> {
    resolve_request_from(settings, engine, input, ResolveMode::Auto, &[]).await
}

/// Resolve with an explicit mode and source selection. `selected` accepts the
/// same values as `search_novels_from`; an empty slice still means the
/// NovelForge registry, not “all”.
pub async fn resolve_request_from(
    settings: &Settings,
    engine: Engine,
    input: &str,
    mode: ResolveMode,
    selected: &[String],
) -> AppResult<ResolvedTitle> {
    let (prefixed_mode, stripped) = split_author_prefix(input);
    let mode = if prefixed_mode == ResolveMode::Author {
        ResolveMode::Author
    } else {
        mode
    };
    let input = stripped.trim();
    if input.chars().count() < 2 {
        return Err(AppError::msg("Enter a title, author, or valid URL"));
    }

    if let Some(url) = as_web_url(input) {
        return Ok(ResolvedTitle {
            input: input.to_string(),
            title: title_from_source(&url),
            source: input.to_string(),
            detail: "Direct source link".to_string(),
            confidence: 1.0,
            matched_by: "url".to_string(),
            searched: Vec::new(),
            alternatives: Vec::new(),
            cached: false,
            cache_age_seconds: None,
        });
    }

    match engine {
        Engine::Webnovel => resolve_webnovel(settings, input, mode, selected).await,
        Engine::Manga => Err(AppError::msg(
            "Manga jobs need a direct page URL. Pick a source below and paste its link.",
        )),
        Engine::Books => Ok(ResolvedTitle {
            input: input.to_string(),
            title: input.chars().take(140).collect(),
            source: input.to_string(),
            detail: "Book title search".to_string(),
            confidence: 1.0,
            matched_by: "title".to_string(),
            searched: Vec::new(),
            alternatives: Vec::new(),
            cached: false,
            cache_age_seconds: None,
        }),
    }
}

fn split_author_prefix(input: &str) -> (ResolveMode, &str) {
    let trimmed = input.trim();
    for prefix in ["author:", "by:"] {
        if trimmed.len() > prefix.len() && trimmed[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return (ResolveMode::Author, &trimmed[prefix.len()..]);
        }
    }
    (ResolveMode::Auto, trimmed)
}

async fn resolve_webnovel(
    settings: &Settings,
    input: &str,
    mode: ResolveMode,
    selected: &[String],
) -> AppResult<ResolvedTitle> {
    if mode == ResolveMode::Author {
        return resolve_author(settings, input, selected).await;
    }

    // Author pages and the crawler search run together. If the query is an
    // exact author name, the author page is authoritative and no detail-page
    // verification is needed; otherwise the page rows become alternatives.
    let (targets, failures) = resolve_targets(selected);
    let (pages, direct) = tokio::join!(
        collect_author_pages(input, &targets),
        search_targets(settings, input.to_string(), targets.clone(), failures)
    );
    let (page_hits, page_consulted, _, page_cache) = pages;
    let mut cache = CacheStamp::from_search(&direct);
    cache.merge(page_cache);
    let mut searched = searched_labels(&direct.searched);
    for label in searched_labels(&page_consulted) {
        if !searched.contains(&label) {
            searched.push(label);
        }
    }

    if page_hits.iter().any(|hit| {
        hit.author
            .as_deref()
            .is_some_and(|author| normalize(author) == normalize(input))
    }) {
        return Ok(author_page_resolved(input, page_hits, searched, cache));
    }

    if let Some((hit, score)) = best_match(input, &direct.results) {
        // A colon or dash suffix is a sequel/fan-fiction. Canonicalise short
        // acronyms ("LOTM: Before the Protagonist") before accepting those.
        if !is_spinoff(input, &hit.title) {
            return Ok(resolved_hit(
                input,
                hit,
                score,
                "title",
                format!("Matched on {}", hit.source),
                searched,
                alternatives_from(input, &direct.results, &hit.url),
                cache,
            ));
        }
    }

    // The search row can be a truncated title or a synonym the source uses in
    // its detail page. Inspect a few candidates for a stronger title, an
    // advertised alias, or an author match before consulting public databases.
    if !direct.results.is_empty() {
        let (evidence, evidence_cache) = verify_candidates(input, &direct.results).await;
        cache.merge(evidence_cache);
        if let Some(best) = evidence
            .iter()
            .find(|candidate| candidate.title_score >= MATCH_THRESHOLD)
            && !is_spinoff(input, &best.result.title)
        {
            return Ok(resolved_hit(
                input,
                &best.result,
                best.title_score,
                "title",
                format!("Matched on {}", best.result.source),
                searched,
                alternatives_from(input, &direct.results, &best.result.url),
                cache,
            ));
        }
        if let Some(best) = evidence
            .iter()
            .find(|candidate| candidate.alias_score >= ALIAS_THRESHOLD)
        {
            return Ok(resolved_hit(
                input,
                &best.result,
                best.alias_score,
                "alias",
                format!(
                    "Alias “{}” on {}",
                    best.alias.as_deref().unwrap_or(input),
                    best.result.source
                ),
                searched,
                alternatives_from(input, &direct.results, &best.result.url),
                cache,
            ));
        }
        if let Some(best) = evidence
            .iter()
            .find(|candidate| candidate.author_score >= AUTHOR_THRESHOLD)
        {
            // The same author usually has more novels than the site search
            // returns; add the source author pages as alternatives.
            let alternatives = merge_author_page_alternatives(
                &page_hits,
                &best.result,
                alternatives_from(input, &direct.results, &best.result.url),
            );
            return Ok(resolved_hit(
                input,
                &best.result,
                best.author_score,
                "author",
                format!(
                    "Author “{}” on {}",
                    best.author.as_deref().unwrap_or(input),
                    best.result.source
                ),
                searched,
                alternatives,
                cache,
            ));
        }
    }

    let (anilist, wikipedia) = tokio::join!(anilist_title(input), wikipedia_title(input));
    // Record the title databases even when they return nothing: the failure
    // message must name every source that was actually consulted.
    searched.push("AniList".to_string());
    searched.push("Wikipedia".to_string());
    dedupe_labels(&mut searched);
    let mut candidates: Vec<(&str, String)> = Vec::new();
    if let Some(title) = anilist {
        candidates.push(("AniList", title));
    }
    if let Some(title) = wikipedia
        && !candidates
            .iter()
            .any(|(_, existing)| same_title(existing, &title))
    {
        candidates.push(("Wikipedia", title));
    }

    for (via, title) in candidates {
        searched.push(via.to_string());
        dedupe_labels(&mut searched);

        // The first search already answered this title, or supplied a row
        // with exactly this name. Re-running the crawler would return the same
        // rows, so use them and skip the second process.
        if same_title(&title, input) {
            continue;
        }
        if let Some(hit) = direct
            .results
            .iter()
            .find(|row| same_title(&title, &row.title))
        {
            let score = relevance(&title, &hit.title);
            if !is_spinoff(&title, &hit.title) {
                return Ok(resolved_hit(
                    input,
                    hit,
                    score,
                    "title",
                    format!("{via} → {} → {}", hit.title, hit.source),
                    searched,
                    alternatives_from(input, &direct.results, &hit.url),
                    cache,
                ));
            }
        }

        let found = search_novels_from(settings, &title, selected).await;
        cache.merge(CacheStamp::from_search(&found));
        for label in searched_labels(&found.searched) {
            if !searched.contains(&label) {
                searched.push(label);
            }
        }
        if let Some((hit, score)) = best_match(&title, &found.results)
            && !is_spinoff(&title, &hit.title)
        {
            return Ok(resolved_hit(
                input,
                hit,
                score,
                "title",
                format!("{via} → {} → {}", hit.title, hit.source),
                searched,
                alternatives_from(&title, &found.results, &hit.url),
                cache,
            ));
        }
        // AniList/Wikipedia may give a synonym that only the source detail
        // page advertises as an alternative title.
        let (evidence, evidence_cache) = verify_candidates(&title, &found.results).await;
        cache.merge(evidence_cache);
        if let Some(best) = evidence
            .iter()
            .find(|candidate| candidate.alias_score >= ALIAS_THRESHOLD)
        {
            return Ok(resolved_hit(
                input,
                &best.result,
                best.alias_score,
                "alias",
                format!(
                    "{via} → alias “{}” on {}",
                    best.alias.as_deref().unwrap_or(&title),
                    best.result.source
                ),
                searched,
                alternatives_from(&title, &found.results, &best.result.url),
                cache,
            ));
        }
    }

    Err(no_match_error(input, &searched, false))
}

#[allow(clippy::too_many_arguments)]
fn resolved_hit(
    input: &str,
    hit: &NovelResult,
    confidence: f32,
    matched_by: &str,
    detail: String,
    searched: Vec<String>,
    alternatives: Vec<NovelResult>,
    cache: CacheStamp,
) -> ResolvedTitle {
    ResolvedTitle {
        input: input.to_string(),
        title: hit.title.clone(),
        source: hit.url.clone(),
        detail,
        confidence,
        matched_by: matched_by.to_string(),
        searched,
        alternatives: alternatives
            .into_iter()
            .filter(|alternative| alternative.url != hit.url)
            .take(5)
            .collect(),
        cached: cache.cached,
        cache_age_seconds: cache.age_seconds,
    }
}

fn searched_labels(searched: &[String]) -> Vec<String> {
    let mut labels: Vec<String> = searched.iter().map(|url| source_label(url)).collect();
    labels.sort();
    labels.dedup();
    labels
}

fn dedupe_labels(labels: &mut Vec<String>) {
    labels.sort();
    labels.dedup();
}

fn alternatives_from(query: &str, results: &[NovelResult], chosen_url: &str) -> Vec<NovelResult> {
    let mut rows: Vec<(&NovelResult, f32)> = results
        .iter()
        .filter(|result| result.url != chosen_url)
        .map(|result| (result, relevance(query, &result.title)))
        .filter(|(_, score)| *score >= 0.4)
        .collect();
    rows.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| source_rank(&left.0.source).cmp(&source_rank(&right.0.source)))
    });
    rows.into_iter()
        .map(|(result, _)| result.clone())
        .take(5)
        .collect()
}

fn no_match_error(input: &str, searched: &[String], author_mode: bool) -> AppError {
    let sources = if searched.is_empty() {
        "no source answered".to_string()
    } else {
        searched.join(", ")
    };
    if author_mode {
        AppError::msg(format!(
            "No author match found for “{input}”. Searched: {sources}. Some sources do not index authors; try a title instead, or paste the page URL."
        ))
    } else {
        AppError::msg(format!(
            "No source link found for “{input}”. Searched: {sources}. Try the full title, an author name with “author:”, or paste the page URL."
        ))
    }
}

/// Round-robin candidates across sources so one noisy source cannot fill the
/// whole verification budget. Each source keeps its own relevance order, and
/// the first hit from every source is inspected before any source's second.
fn interleave_by_source(results: &[NovelResult], limit: usize) -> Vec<NovelResult> {
    let mut groups: Vec<(String, Vec<NovelResult>)> = Vec::new();
    for result in results {
        match groups
            .iter_mut()
            .find(|(source, _)| *source == result.source)
        {
            Some((_, rows)) => rows.push(result.clone()),
            None => groups.push((result.source.clone(), vec![result.clone()])),
        }
    }

    let mut out = Vec::with_capacity(limit.min(results.len()));
    let mut layer = 0;
    while out.len() < limit {
        let mut progressed = false;
        for (_, rows) in &groups {
            let Some(row) = rows.get(layer) else {
                continue;
            };
            out.push(row.clone());
            progressed = true;
            if out.len() == limit {
                break;
            }
        }
        if !progressed {
            break;
        }
        layer += 1;
    }
    out
}

struct CandidateEvidence {
    result: NovelResult,
    title_score: f32,
    alias: Option<String>,
    alias_score: f32,
    author: Option<String>,
    author_score: f32,
}

#[derive(Default)]
struct FetchedSummary {
    metadata: Option<NovelMetadata>,
    cache_age_seconds: Option<u64>,
}

/// Read the detail page for the strongest search hits and score their real
/// title, advertised aliases and author. A source that does not expose one of
/// these simply contributes zero for it.
async fn verify_candidates(
    query: &str,
    results: &[NovelResult],
) -> (Vec<CandidateEvidence>, CacheStamp) {
    verify_candidates_with(query, results, |url: String| async move {
        fetch_summary_cached(&url).await
    })
    .await
}

/// Same as [`verify_candidates`], with an injectable fetch used by the offline
/// tests. The first wave is three candidates; an exact author match there ends
/// verification before any later candidate is requested.
async fn verify_candidates_with<F, Fut>(
    query: &str,
    results: &[NovelResult],
    fetch: F,
) -> (Vec<CandidateEvidence>, CacheStamp)
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = FetchedSummary> + Send + 'static,
{
    let planned = interleave_by_source(results, RESOLVE_CANDIDATES);
    let fetch = Arc::new(fetch);
    let mut evidence = Vec::new();
    let mut cache = CacheStamp::default();
    let mut cursor = 0;

    while cursor < planned.len() {
        let first_wave = cursor == 0;
        let end = if first_wave {
            (cursor + RESOLVE_FIRST_WAVE).min(planned.len())
        } else {
            planned.len()
        };
        let wave: Vec<NovelResult> = planned[cursor..end].to_vec();
        cursor = end;

        let semaphore = Arc::new(Semaphore::new(RESOLVE_FETCH_CONCURRENCY));
        let mut jobs = JoinSet::new();
        for result in wave {
            let fetch = Arc::clone(&fetch);
            let semaphore = Arc::clone(&semaphore);
            jobs.spawn(async move {
                let _permit = semaphore.acquire_owned().await;
                let fetched = tokio::time::timeout(
                    Duration::from_secs(RESOLVE_SUMMARY_TIMEOUT_SECS),
                    fetch(result.url.clone()),
                )
                .await
                .unwrap_or_default();
                (result, fetched)
            });
        }

        while let Some(joined) = jobs.join_next().await {
            let (result, fetched) = match joined {
                Ok(pair) => pair,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[resolve] candidate task failed: {error}");
                    continue;
                }
            };
            cache.observe(fetched.cache_age_seconds);
            let Some(summary) = fetched.metadata else {
                if std::env::var_os("NOVELFORGE_QA_RESOLVE_DEBUG").is_some() {
                    eprintln!("[resolve] summary unavailable for {}", result.url);
                }
                continue;
            };
            let title_score = relevance(query, &summary.title);
            let (alias, alias_score) = summary
                .aliases
                .iter()
                .map(|alias| (alias.clone(), alias_score(query, alias)))
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or((String::new(), 0.0));
            let (author, author_score) = summary
                .authors
                .iter()
                .chain(summary.author.iter())
                .map(|author| (author.clone(), author_score(query, author)))
                .max_by(|left, right| {
                    left.1
                        .partial_cmp(&right.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or((String::new(), 0.0));
            evidence.push(CandidateEvidence {
                result,
                title_score,
                alias: (!alias.is_empty()).then_some(alias),
                alias_score,
                author: (!author.is_empty()).then_some(author),
                author_score,
            });
            #[cfg(debug_assertions)]
            if std::env::var_os("NOVELFORGE_QA_RESOLVE_DEBUG").is_some() {
                eprintln!(
                    "[resolve] candidate title={:?} authors={:?} author_score={:.2}",
                    summary.title, summary.authors, author_score
                );
            }
        }

        if first_wave
            && evidence
                .iter()
                .any(|candidate| candidate.author_score >= EXACT_AUTHOR_SCORE)
        {
            break;
        }
    }

    evidence.sort_by(|left, right| {
        right
            .title_score
            .max(right.alias_score)
            .partial_cmp(&left.title_score.max(left.alias_score))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .author_score
                    .partial_cmp(&left.author_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                search_position(results, &left.result.url)
                    .cmp(&search_position(results, &right.result.url))
            })
            .then_with(|| source_rank(&left.result.source).cmp(&source_rank(&right.result.source)))
    });
    (evidence, cache)
}

/// Position of a URL in the source search output; lower is more relevant.
fn search_position(results: &[NovelResult], url: &str) -> usize {
    results
        .iter()
        .position(|result| result.url == url)
        .unwrap_or(usize::MAX)
}

/// One novel listed on a source author page, already attributed by the site.
struct AuthorPageHit {
    result: NovelResult,
    author: Option<String>,
}

/// Author-mode resolution. Sources that expose an author page are answered
/// from that page alone: it is complete (the search endpoint only returns
/// title matches) and needs no crawler process. Sources without one fall back
/// to search plus detail-page verification.
async fn resolve_author(
    settings: &Settings,
    input: &str,
    selected: &[String],
) -> AppResult<ResolvedTitle> {
    let (targets, _) = resolve_targets(selected);
    let (page_hits, consulted, unresolved, mut cache) = collect_author_pages(input, &targets).await;
    let mut searched = searched_labels(&consulted);

    let mut candidates: Vec<CandidateEvidence> = Vec::new();
    if !unresolved.is_empty() {
        let direct = search_targets(settings, input.to_string(), unresolved, Vec::new()).await;
        cache.merge(CacheStamp::from_search(&direct));
        for label in searched_labels(&direct.searched) {
            if !searched.contains(&label) {
                searched.push(label);
            }
        }
        let (evidence, evidence_cache) = verify_candidates(input, &direct.results).await;
        cache.merge(evidence_cache);
        for candidate in evidence
            .into_iter()
            .filter(|candidate| candidate.author_score >= AUTHOR_THRESHOLD)
        {
            if candidates
                .iter()
                .any(|row| same_title(&row.result.title, &candidate.result.title))
            {
                continue;
            }
            candidates.push(candidate);
        }
    }

    for hit in page_hits {
        if candidates
            .iter()
            .any(|row| same_title(&row.result.title, &hit.result.title))
        {
            continue;
        }
        candidates.push(CandidateEvidence {
            result: hit.result,
            title_score: 0.0,
            alias: None,
            alias_score: 0.0,
            author: hit.author,
            author_score: 1.0,
        });
    }

    dedupe_labels(&mut searched);
    let Some(best) = candidates.first() else {
        return Err(no_match_error(input, &searched, true));
    };
    let alternatives = candidates
        .iter()
        .skip(1)
        .take(5)
        .map(|candidate| candidate.result.clone())
        .collect();
    Ok(resolved_hit(
        input,
        &best.result,
        best.author_score,
        "author",
        format!(
            "Author “{}” on {}{}",
            best.author.as_deref().unwrap_or(input),
            best.result.source,
            if best.result.detail.as_deref() == Some("Author page") {
                " author page"
            } else {
                ""
            }
        ),
        searched,
        alternatives,
        cache,
    ))
}

/// Fetches every selected source's author page. Returns the hits in
/// preference order (download-ready sources first), the source URLs that were
/// consulted, the targets with no usable page (which the caller must verify
/// through search instead), and how much came from the cache.
async fn collect_author_pages(
    query: &str,
    targets: &[String],
) -> (Vec<AuthorPageHit>, Vec<String>, Vec<String>, CacheStamp) {
    let now = now_secs();
    let mut planned: Vec<(String, String, &'static sources::SourceProfile)> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for target in targets {
        match author_page_url(target, query) {
            Some((url, profile)) => planned.push((target.clone(), url, profile)),
            None => unresolved.push(target.clone()),
        }
    }
    // Registry order, because the first source's shelf order becomes the
    // bibliography order. The representative URL for each novel is chosen
    // afterwards, preferring download-ready sources.
    planned.sort_by_key(|(target, _, _)| source_rank(target));

    let mut pages: HashMap<String, (AppResult<metadata::AuthorPage>, Option<u64>)> = HashMap::new();
    let mut pending = Vec::new();
    for (target, url, _) in &planned {
        match cached_author_page(url, now) {
            Some((page, age)) => {
                pages.insert(target.clone(), (Ok(page), Some(age)));
            }
            None => pending.push((target.clone(), url.clone())),
        }
    }

    let semaphore = Arc::new(Semaphore::new(AUTHOR_PAGE_CONCURRENCY));
    let mut jobs = JoinSet::new();
    for (target, url) in pending {
        let semaphore = Arc::clone(&semaphore);
        jobs.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            let page = tokio::time::timeout(
                Duration::from_secs(AUTHOR_PAGE_TIMEOUT_SECS),
                metadata::fetch_author_page(&url),
            )
            .await
            .unwrap_or_else(|_| {
                Err(AppError::msg(format!(
                    "author page timed out after {AUTHOR_PAGE_TIMEOUT_SECS} s"
                )))
            });
            if let Ok(page) = &page {
                store_author_page(&url, page);
            }
            (target, page)
        });
    }

    while let Some(joined) = jobs.join_next().await {
        match joined {
            Ok((target, page)) => {
                pages.insert(target, (page, None));
            }
            Err(error) => {
                #[cfg(debug_assertions)]
                eprintln!("[resolve] author page task failed: {error}");
            }
        }
    }

    let mut hits = Vec::new();
    let mut consulted = Vec::new();
    let mut cache = CacheStamp::default();
    for (target, url, profile) in planned {
        let Some((page, age)) = pages.remove(&target) else {
            unresolved.push(target);
            continue;
        };
        cache.observe(age);
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                #[cfg(debug_assertions)]
                eprintln!("[resolve] author page {url} failed: {error}");
                unresolved.push(target);
                continue;
            }
        };
        if !author_page_is_trusted(query, &url, &page) {
            unresolved.push(target);
            continue;
        }
        // The site resolved this author slug to an author page that names the
        // requested author. An empty shelf is then authoritative: title search
        // only returns title matches and cannot reveal a novel the page omits.
        consulted.push(target);
        if page.novels.is_empty() {
            continue;
        }
        let source = profile
            .hosts
            .first()
            .map(|host| (*host).to_string())
            .unwrap_or_default();
        for novel in page.novels {
            hits.push(AuthorPageHit {
                result: NovelResult {
                    title: novel.title,
                    url: novel.url,
                    source: source.clone(),
                    detail: Some("Author page".to_string()),
                },
                author: page.author.clone(),
            });
        }
    }
    (order_author_page_hits(hits), consulted, unresolved, cache)
}

/// Sites answer an unknown author slug with their home page, so both the
/// canonical path and the stated author must agree with the query before the
/// page (including an empty shelf) is believed.
fn author_page_is_trusted(query: &str, url: &str, page: &metadata::AuthorPage) -> bool {
    if let Some(canonical) = page.canonical.as_deref()
        && !canonical.to_ascii_lowercase().contains("/author")
    {
        #[cfg(debug_assertions)]
        eprintln!("[resolve] author page {url} landed on {canonical}; ignoring");
        return false;
    }
    let Some(label) = page.author.as_deref() else {
        return false;
    };
    if author_score(query, label) < AUTHOR_THRESHOLD {
        #[cfg(debug_assertions)]
        eprintln!("[resolve] author page {url} is for {label:?}; ignoring");
        return false;
    }
    true
}

/// Builds the bibliography: novels keep the shelf order of the first source
/// that lists them, and each title is represented by its best source (a
/// download-ready one when several sources carry the same novel).
fn order_author_page_hits(hits: Vec<AuthorPageHit>) -> Vec<AuthorPageHit> {
    let mut groups: Vec<Vec<AuthorPageHit>> = Vec::new();
    for hit in hits {
        match groups
            .iter_mut()
            .find(|group| same_title(&group[0].result.title, &hit.result.title))
        {
            Some(group) => group.push(hit),
            None => groups.push(vec![hit]),
        }
    }
    groups
        .into_iter()
        .filter_map(|mut group| {
            group.sort_by_key(|hit| {
                (
                    support_rank(sources::for_host(&hit.result.source).support),
                    sources::rank_for_host(&hit.result.source),
                )
            });
            group.into_iter().next()
        })
        .collect()
}

fn support_rank(support: sources::SupportLevel) -> u8 {
    match support {
        sources::SupportLevel::Verified => 0,
        sources::SupportLevel::IndexVerified => 1,
        sources::SupportLevel::MetadataOnly => 2,
        sources::SupportLevel::Unverified => 3,
    }
}

/// Author-page hits become the result directly when the query is the author's
/// name. The site's own attribution is the strongest possible evidence.
fn author_page_resolved(
    input: &str,
    hits: Vec<AuthorPageHit>,
    searched: Vec<String>,
    cache: CacheStamp,
) -> ResolvedTitle {
    let Some(best) = hits.first() else {
        // Callers only use this when at least one hit matched the query.
        return resolved_hit(
            input,
            &NovelResult {
                title: input.to_string(),
                url: String::new(),
                source: String::new(),
                detail: None,
            },
            0.0,
            "author",
            "No author page match".to_string(),
            searched,
            Vec::new(),
            cache,
        );
    };
    let alternatives = hits
        .iter()
        .skip(1)
        .filter(|hit| !same_title(&hit.result.title, &best.result.title))
        .map(|hit| hit.result.clone())
        .fold(Vec::new(), |mut rows: Vec<NovelResult>, row| {
            if !rows
                .iter()
                .any(|existing| same_title(&existing.title, &row.title))
            {
                rows.push(row);
            }
            rows
        });
    resolved_hit(
        input,
        &best.result,
        1.0,
        "author",
        format!(
            "Author “{}” on {} author page",
            best.author.as_deref().unwrap_or(input),
            best.result.source
        ),
        searched,
        alternatives,
        cache,
    )
}

/// Adds author-page novels to a resolved title's alternatives. Used by auto
/// mode after a search hit matched on author, so the studio offers the whole
/// bibliography instead of just the search rows.
fn merge_author_page_alternatives(
    hits: &[AuthorPageHit],
    chosen: &NovelResult,
    existing: Vec<NovelResult>,
) -> Vec<NovelResult> {
    let mut rows = existing;
    for hit in hits {
        if same_title(&hit.result.title, &chosen.title) {
            continue;
        }
        if rows
            .iter()
            .any(|row| row.url == hit.result.url || same_title(&row.title, &hit.result.title))
        {
            continue;
        }
        rows.push(hit.result.clone());
        if rows.len() >= 5 {
            break;
        }
    }
    rows
}

/// Builds a concrete author-page URL from a source base URL, when the profile
/// declares one.
fn author_page_url(
    target: &str,
    author: &str,
) -> Option<(String, &'static sources::SourceProfile)> {
    let target = Url::parse(target).ok()?;
    let profile = sources::for_host(target.host_str()?);
    let template = profile.author_page?;
    let slug = profile.author_slug.build(author);
    if slug.is_empty() {
        return None;
    }
    let page = template.replace("{author}", &slug);
    let page = Url::parse(&page).ok()?;
    Some((page.to_string(), profile))
}

fn author_score(query: &str, author: &str) -> f32 {
    let score = relevance(query, author);
    if score >= 0.8 {
        return score;
    }
    let query = normalize(query);
    let author = normalize(author);
    if query.is_empty() || author.is_empty() {
        return score;
    }
    if author
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(query.split_whitespace().count().max(1))
        .any(|window| window.join(" ") == query)
    {
        0.9
    } else {
        score
    }
}

fn best_match<'a>(query: &str, results: &'a [NovelResult]) -> Option<(&'a NovelResult, f32)> {
    let mut best: Option<(&NovelResult, f32)> = None;
    for result in results {
        let score = relevance(query, &result.title);
        if score < MATCH_THRESHOLD {
            continue;
        }
        // Keep the first result on a tie: sources arrive in preference order.
        if best.is_none_or(|(_, current)| score > current) {
            best = Some((result, score));
        }
    }
    best
}

fn source_rank(source: &str) -> u8 {
    sources::rank_for_host(source)
}

fn as_web_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() {
        Some(url)
    } else {
        None
    }
}

pub fn title_from_source(url: &Url) -> String {
    if let Some(segment) = url
        .path_segments()
        .and_then(|segments| segments.rev().find(|part| !part.is_empty()))
    {
        let stem = segment
            .split_once('.')
            .map(|(value, _)| value)
            .unwrap_or(segment);
        let words = stem
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut characters = part.chars();
                match characters.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !words.is_empty() {
            return words.chars().take(140).collect();
        }
    }

    url.host_str().unwrap_or("Untitled download").to_string()
}

fn source_label(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .map(|host| host.trim_start_matches("www.").to_string())
        .unwrap_or_else(|| url.to_string())
}

// ---------------------------------------------------------------------------
// Title databases
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AniListResponse {
    data: Option<AniListData>,
}

#[derive(Deserialize)]
struct AniListData {
    #[serde(rename = "Page")]
    page: AniListPage,
}

#[derive(Deserialize)]
struct AniListPage {
    #[serde(default)]
    media: Vec<AniListMedia>,
}

#[derive(Deserialize)]
struct AniListMedia {
    title: AniListTitle,
    #[serde(default)]
    synonyms: Vec<String>,
}

#[derive(Deserialize)]
struct AniListTitle {
    english: Option<String>,
    romaji: Option<String>,
    native: Option<String>,
}

async fn anilist_title(query: &str) -> Option<String> {
    let client = http_client().ok()?;
    let response = client
        .post("https://graphql.anilist.co")
        .json(&serde_json::json!({
            "query": "query ($search: String) { Page(perPage: 5) { media(search: $search, type: MANGA) { title { english romaji } synonyms } } }",
            "variables": { "search": query },
        }))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<AniListResponse>()
        .await
        .ok()?;

    let mut matches: Vec<(f32, bool, String)> = Vec::new();
    for media in response.data?.page.media {
        let english = media.title.english.clone();
        let romaji = media.title.romaji.clone();
        let mut candidates = Vec::new();
        if let Some(english) = &english {
            candidates.push(english.clone());
        }
        if let Some(romaji) = &romaji {
            candidates.push(romaji.clone());
        }
        // `native` is the Korean/Japanese/Chinese title. It lets a Hangul or
        // CJK query match here and come back as the English/romaji title even
        // though the sources themselves only index the English name.
        if let Some(native) = &media.title.native {
            candidates.push(native.clone());
        }
        candidates.extend(media.synonyms.iter().cloned());

        let score = candidates
            .iter()
            .map(|candidate| alias_score(query, candidate))
            .fold(0.0_f32, f32::max);
        if score < 0.8 {
            continue;
        }

        let Some(title) = english.or(romaji) else {
            continue;
        };
        let spinoff = title.contains(':') || title.contains('–');
        matches.push((score, spinoff, title));
    }

    // A canonical title should always beat a colon-suffixed spin-off when both
    // are plausible matches.
    matches.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.2.len().cmp(&right.2.len()))
    });
    matches.into_iter().next().map(|(_, _, title)| title)
}

fn alias_score(query: &str, candidate: &str) -> f32 {
    let left = normalize(query);
    let right = normalize(candidate);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    // Short inputs are often acronyms ("LOTM", "TBATE"). Compare against the
    // candidate's initials rather than a substring: "ri" must not match
    // "mysteries".
    if left.len() <= 8 && !left.contains(' ') {
        let initials = right
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .collect::<String>();
        if initials == left {
            return 0.85;
        }
    }
    if left.len() >= 4 && (right.starts_with(&left) || right.contains(&left)) {
        return 0.82;
    }
    token_score(&left, &right)
}

#[derive(Deserialize)]
struct WikipediaResponse {
    query: Option<WikipediaQuery>,
}

#[derive(Deserialize)]
struct WikipediaQuery {
    #[serde(default)]
    search: Vec<WikipediaHit>,
}

#[derive(Deserialize)]
struct WikipediaHit {
    title: String,
    #[serde(default)]
    snippet: String,
}

async fn wikipedia_title(query: &str) -> Option<String> {
    let client = http_client().ok()?;
    let response = client
        .get("https://en.wikipedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("list", "search"),
            ("srsearch", &format!("{query} novel")),
            ("srlimit", "5"),
            ("format", "json"),
        ])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<WikipediaResponse>()
        .await
        .ok()?;

    for hit in response.query?.search {
        let title = hit
            .title
            .split(" (")
            .next()
            .unwrap_or(&hit.title)
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let snippet = hit.snippet.to_lowercase();
        let looks_like_prose = [
            "novel",
            "manga",
            "manhwa",
            "manhua",
            "webcomic",
            "web comic",
            "light novel",
            "serialized",
        ]
        .iter()
        .any(|keyword| snippet.contains(keyword));
        if looks_like_prose && relevance(query, &title) >= 0.4 {
            return Some(title);
        }
    }

    None
}

fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("NovelForge/0.1 (desktop download studio)")
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| AppError::msg(format!("Could not create resolver client: {error}")))
}

// ---------------------------------------------------------------------------
// Text matching
// ---------------------------------------------------------------------------

fn normalize(value: &str) -> String {
    // Lower-case first so the diacritic table only needs the lower-case forms.
    let folded: String = value.to_lowercase().chars().map(fold_char).collect();
    let mut words = Vec::new();
    for word in folded
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if matches!(
            word,
            "novel" | "webnovel" | "web" | "lightnovel" | "read" | "online" | "the" | "a" | "an"
        ) {
            continue;
        }
        words.push(word.to_string());
    }
    words.join(" ")
}

/// Fold compatibility forms and common Latin diacritics so “Pokémon”,
/// “Pokemon” and a full-width “ＰＯＫÉＭＯＮ” compare equal. Hangul and CJK
/// characters are preserved; they are not transliterated here — the AniList
/// step supplies the romanised/English title for those queries.
fn fold_char(character: char) -> char {
    // Full-width ASCII → ASCII, ideographic space → space.
    if ('\u{FF01}'..='\u{FF5E}').contains(&character)
        && let Some(ascii) = char::from_u32(character as u32 - 0xFEE0)
    {
        return ascii;
    }
    if character == '\u{3000}' {
        return ' ';
    }
    // Combining marks (NFD input) disappear.
    if ('\u{0300}'..='\u{036F}').contains(&character) {
        return '\0';
    }
    match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' => 'i',
        'ł' | 'ĺ' | 'ļ' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'š' | 'ş' => 's',
        'ť' | 'ţ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => character,
    }
}

fn token_score(left: &str, right: &str) -> f32 {
    let left: HashSet<&str> = left.split_whitespace().collect();
    let right: HashSet<&str> = right.split_whitespace().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.intersection(&right).count();
    if shared == 0 {
        return 0.0;
    }
    let union = left.union(&right).count();
    let jaccard = shared as f32 / union as f32;
    if jaccard >= 0.5 {
        0.5 + jaccard * 0.3
    } else {
        0.0
    }
}

pub fn relevance(query: &str, title: &str) -> f32 {
    let left = normalize(query);
    let right = normalize(title);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    // Match whole-word prefixes only: "lotm" must not match "narutolotm".
    if right.starts_with(&left)
        && right
            .as_bytes()
            .get(left.len())
            .is_none_or(|next| *next == b' ')
    {
        return 0.9;
    }
    if left.starts_with(&right)
        && right.len() >= 5
        && left
            .as_bytes()
            .get(right.len())
            .is_none_or(|next| *next == b' ')
    {
        return 0.78;
    }
    token_score(&left, &right)
}

fn same_title(left: &str, right: &str) -> bool {
    normalize(left) == normalize(right)
}

fn is_spinoff(query: &str, title: &str) -> bool {
    let query = query.trim().to_lowercase();
    let title = title.trim().to_lowercase();
    let Some(rest) = title.strip_prefix(&query) else {
        return false;
    };
    let rest = rest.trim_start();
    rest.starts_with(':')
        || rest.starts_with('-')
        || rest.starts_with('–')
        || rest.starts_with("the ")
        || rest.starts_with("a ")
}

// ---------------------------------------------------------------------------
// Manga source catalogue
// ---------------------------------------------------------------------------

pub async fn list_manga_sources(settings: &Settings) -> AppResult<MangaSources> {
    let tool = resolve_tool(settings, Engine::Manga).ok_or_else(|| {
        AppError::msg("gallery-dl is not available. Open Guardrails to finish the setup.")
    })?;

    let mut command = Command::new(spawn::native_path(&tool.program));
    command
        .args(&tool.prefix)
        .arg("--list-extractors")
        .kill_on_drop(true)
        .env("NO_COLOR", "1");
    spawn::hide_console(&mut command);

    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| AppError::msg("gallery-dl did not answer in time"))?
        .map_err(|error| AppError::msg(format!("Could not launch gallery-dl: {error}")))?;

    if !output.status.success() && output.stdout.is_empty() {
        return Err(AppError::msg(
            "gallery-dl could not list its extractors. Check the toolchain paths in Guardrails.",
        ));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(MangaSources {
        sources: parse_manga_sources(&text),
        fetched_at: now_ms(),
    })
}

/// Reads `gallery-dl --list-extractors` blocks and keeps the series-page
/// extractors (`Subcategory: manga`). gallery-dl publishes no language matrix,
/// so every entry is honestly labelled `Multi` instead of guessing.
fn parse_manga_sources(text: &str) -> Vec<MangaSource> {
    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut category: Option<String> = None;
    let mut subcategory: Option<String> = None;
    let mut example: Option<String> = None;

    let mut flush = |category: &mut Option<String>,
                     subcategory: &mut Option<String>,
                     example: &mut Option<String>| {
        let is_manga = subcategory
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("manga"));
        if is_manga
            && let (Some(name), Some(url)) = (category.take(), example.take())
            && seen.insert(name.clone())
        {
            let tags = if ["hentai", "porn", "adult", "nsfw"]
                .iter()
                .any(|needle| name.to_ascii_lowercase().contains(needle))
            {
                vec!["adult".to_string()]
            } else {
                Vec::new()
            };
            sources.push(MangaSource {
                name,
                url,
                language: "Multi".to_string(),
                tags,
            });
        }
        *category = None;
        *subcategory = None;
        *example = None;
    };

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut category, &mut subcategory, &mut example);
            continue;
        }
        if let Some(rest) = line.strip_prefix("Category:") {
            let mut parts = rest.split(" - Subcategory:");
            category = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            subcategory = parts.next().map(str::trim).map(str::to_string);
            continue;
        }
        if let Some(rest) = line.strip_prefix("Example") {
            let url = rest.trim_start_matches([' ', ':']).trim();
            if url.starts_with("http") {
                example = Some(url.to_string());
            }
        }
    }
    flush(&mut category, &mut subcategory, &mut example);
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_blocks() {
        let text = "Searching: 100%|██████████| 1/1 [00:01<00:00]\n\
📖 Reverend Insanity  (2 results)\n\
  ➡ https://novelfire.net/book/reverend-insanity\n\
    |\n\
  ➡ https://novelfire.net/book/reverend-insanity\n\
📖 Reverend Insanity: Dream Gu  (1 results)\n\
  ➡ https://novelfire.net/book/reverend-insanity-dream-gu\n\
    Fantasy, Action\n";
        let results = parse_search_output(text, "novelfire.net");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Reverend Insanity");
        assert_eq!(results[1].title, "Reverend Insanity: Dream Gu");
        assert_eq!(results[1].detail.as_deref(), Some("Fantasy, Action"));
    }

    #[test]
    fn parses_wrapped_search_blocks() {
        let text = "Searching: 100%|██████████| 1/1 [00:01<00:00]\n\
📖 At The Beginning Of The Novel, The Ruthless Villain Kicked Out His Sister \n\
From House  (1 results)\n\
  ➡ \n\
https://wondernovels.com/novels/in-the-beginning-of-the-novel-the-ruthless-villa\n\
in-kicks-out-his-sister\n\
    T/N | Rating:\n";
        let results = parse_search_output(text, "wondernovels.com");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].title,
            "At The Beginning Of The Novel, The Ruthless Villain Kicked Out His Sister From House"
        );
        assert_eq!(
            results[0].url,
            "https://wondernovels.com/novels/in-the-beginning-of-the-novel-the-ruthless-villain-kicks-out-his-sister"
        );
    }

    #[test]
    fn progress_bars_are_not_search_failures() {
        let progress = "Searching:   0%|          | 0/1 [00:00<?, ?source/s]\r\
Searching: 100%|██████████| 1/1 [00:01<00:00,  1.60s/source]";
        assert_eq!(last_error_line(progress), None);
        assert_eq!(
            last_error_line("Traceback (most recent call last):\nLNException: source rejected"),
            Some("LNException: source rejected".to_string())
        );
    }

    #[test]
    fn scores_exact_titles_above_fan_fiction() {
        assert_eq!(relevance("reverend insanity", "Reverend Insanity"), 1.0);
        assert!(
            relevance("reverend insanity", "Reverend Insanity")
                > relevance("reverend insanity", "Reverend Insanity: Dream Gu")
        );
        assert_eq!(relevance("lotm", "Lord of the Mysteries"), 0.0);
        assert_eq!(
            relevance("lord of the mysteries", "Lord of the Mysteries"),
            1.0
        );
    }

    #[test]
    fn interleaves_candidates_across_sources() {
        let rows = vec![
            NovelResult {
                title: "A1".to_string(),
                url: "https://a.example/1".to_string(),
                source: "a.example".to_string(),
                detail: None,
            },
            NovelResult {
                title: "A2".to_string(),
                url: "https://a.example/2".to_string(),
                source: "a.example".to_string(),
                detail: None,
            },
            NovelResult {
                title: "B1".to_string(),
                url: "https://b.example/1".to_string(),
                source: "b.example".to_string(),
                detail: None,
            },
        ];
        let picked = interleave_by_source(&rows, 3);
        let titles: Vec<&str> = picked.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, vec!["A1", "B1", "A2"]);
        assert_eq!(interleave_by_source(&rows, 10).len(), 3);
    }

    #[test]
    fn folds_diacritics_and_full_width_forms() {
        assert_eq!(normalize("Pokémon"), "pokemon");
        assert_eq!(normalize("ＰＯＫÉＭＯＮ"), "pokemon");
        assert_eq!(
            normalize("The Beginning After the End"),
            "beginning after end"
        );
        assert_eq!(relevance("pokemon", "Pokémon"), 1.0);
    }

    #[test]
    fn recognises_aliases() {
        assert_eq!(alias_score("lotm", "LOTM"), 1.0);
        assert!(alias_score("master of gu", "Master of Gu") >= 0.8);
        assert_eq!(alias_score("ri", "Lord of the Mysteries"), 0.0);
    }

    #[test]
    fn parses_extractor_catalogue() {
        let text = "MangadexChapterExtractor\n\
                    Extractor for manga chapters from mangadex.org\n\
                    Category: mangadex - Subcategory: chapter\n\
                    Example : https://mangadex.org/chapter/01234567\n\
                    \n\
                    MangadexMangaExtractor\n\
                    Extractor for manga from mangadex.org\n\
                    Category: mangadex - Subcategory: manga\n\
                    Example : https://mangadex.org/title/01234567\n\
                    \n\
                    MangafireMangaExtractor\n\
                    Extractor for manga from mangafire.to\n\
                    Category: mangafire - Subcategory: manga\n\
                    Example : https://mangafire.to/title/ID-MANGA\n\
                    \n\
                    FoolslideMangaExtractor\n\
                    Base class for manga extractors for FoOlSlide based sites\n\
                    Category:  - Subcategory: manga\n\
                    Example : https://read.powermanga.org/series/MANGA/\n\
                    \n\
                    ForiioWorkExtractor\n\
                    Category: foriio - Subcategory: work\n\
                    Example : https://www.foriio.com/works/12345\n";
        let sources = parse_manga_sources(text);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "mangadex");
        assert_eq!(sources[0].url, "https://mangadex.org/title/01234567");
        assert_eq!(sources[0].language, "Multi");
        assert_eq!(sources[1].name, "mangafire");
    }

    #[test]
    fn extracts_titles_from_urls() {
        let url = Url::parse("https://novelfire.net/book/reverend-insanity").unwrap();
        assert_eq!(title_from_source(&url), "Reverend Insanity");
        let url = Url::parse("https://novelphoenix.net/the-beginning-after-the-end.html").unwrap();
        assert_eq!(title_from_source(&url), "The Beginning After The End");
    }

    #[test]
    fn author_page_resolution_lists_the_whole_shelf_once() {
        fn hit(title: &str, url: &str, source: &str) -> AuthorPageHit {
            AuthorPageHit {
                result: NovelResult {
                    title: title.to_string(),
                    url: url.to_string(),
                    source: source.to_string(),
                    detail: Some("Author page".to_string()),
                },
                author: Some("Gu Zhen Ren".to_string()),
            }
        }
        let hits = vec![
            hit(
                "Mysteries of Immortal Puppet Master",
                "https://novelphoenix.net/mysteries-of-immortal-puppet-master.html",
                "novelphoenix.net",
            ),
            hit(
                "Reverend Insanity",
                "https://novelphoenix.net/reverend-insanity.html",
                "novelphoenix.net",
            ),
            hit(
                "Reverend Insanity",
                "https://novelfire.net/book/reverend-insanity",
                "novelfire.net",
            ),
            hit(
                "Infinite Bloodcore",
                "https://novelfire.net/book/infinite-bloodcore",
                "novelfire.net",
            ),
        ];
        let resolved = author_page_resolved(
            "Gu Zhen Ren",
            hits,
            vec!["novelfire.net".to_string(), "novelphoenix.net".to_string()],
            CacheStamp::default(),
        );
        assert_eq!(resolved.title, "Mysteries of Immortal Puppet Master");
        assert_eq!(resolved.matched_by, "author");
        assert_eq!(resolved.confidence, 1.0);
        assert!(resolved.detail.contains("author page"));
        let titles: Vec<&str> = resolved
            .alternatives
            .iter()
            .map(|alternative| alternative.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec!["Reverend Insanity", "Infinite Bloodcore"],
            "the same novel from another source must not be listed twice"
        );
    }

    #[test]
    fn author_page_hits_keep_shelf_order_and_prefer_ready_sources() {
        fn hit(title: &str, source: &str) -> AuthorPageHit {
            AuthorPageHit {
                result: NovelResult {
                    title: title.to_string(),
                    url: format!(
                        "https://{source}/book/{}",
                        title.to_lowercase().replace(' ', "-")
                    ),
                    source: source.to_string(),
                    detail: Some("Author page".to_string()),
                },
                author: Some("Gu Zhen Ren".to_string()),
            }
        }
        let hits = vec![
            hit("Reverend Insanity", "novelfire.net"),
            hit("Infinite Bloodcore", "novelfire.net"),
            hit("Mysteries of Immortal Puppet Master", "novelfire.net"),
            hit("Mysteries of Immortal Puppet Master", "novelphoenix.net"),
            hit("Reverend Insanity", "novelphoenix.net"),
        ];
        let ordered = order_author_page_hits(hits);
        let titles: Vec<&str> = ordered
            .iter()
            .map(|hit| hit.result.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec![
                "Reverend Insanity",
                "Infinite Bloodcore",
                "Mysteries of Immortal Puppet Master"
            ]
        );
        assert_eq!(ordered[0].result.source, "novelphoenix.net");
        assert_eq!(ordered[1].result.source, "novelfire.net");
        assert_eq!(ordered[2].result.source, "novelphoenix.net");
    }

    #[test]
    fn author_page_urls_use_the_source_spelling() {
        let (url, profile) = author_page_url("https://novelfire.net/", "Gu Zhen Ren").unwrap();
        assert_eq!(profile.id, "novelfire");
        assert_eq!(url, "https://novelfire.net/author/gu-zhen-ren");

        let (url, profile) = author_page_url("https://novelphoenix.net/", "Gu Zhen Ren").unwrap();
        assert_eq!(profile.id, "novelphoenix");
        assert_eq!(url, "https://novelphoenix.net/author/Gu+Zhen+Ren");

        // Non-ASCII author names stay on the same host, percent-encoded.
        let (url, _) = author_page_url("https://novelphoenix.net/", "蛊真人").unwrap();
        assert!(url.starts_with("https://novelphoenix.net/author/%E8%9B%8A"));

        // Sources without a declared author page are searched instead.
        assert!(author_page_url("https://novelfull.net/", "Gu Zhen Ren").is_none());
    }

    fn metadata(title: &str, author: Option<&str>) -> NovelMetadata {
        NovelMetadata {
            title: title.to_string(),
            author: author.map(str::to_string),
            authors: author
                .map(|value| vec![value.to_string()])
                .unwrap_or_default(),
            synopsis: None,
            tags: Vec::new(),
            status: None,
            language: None,
            chapter_count: None,
            source_url: String::new(),
            source: String::new(),
            cover: None,
            aliases: Vec::new(),
        }
    }

    fn candidate(title: &str, source: &str, index: usize) -> NovelResult {
        NovelResult {
            title: title.to_string(),
            url: format!("https://{source}/book/{index}"),
            source: source.to_string(),
            detail: None,
        }
    }

    #[tokio::test]
    async fn exact_author_in_the_first_wave_stops_after_three_fetches() {
        let results: Vec<NovelResult> = (0..8)
            .map(|index| candidate(&format!("Novel {index}"), "novelfire.net", index))
            .collect();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);

        let (evidence, _) = verify_candidates_with("Gu Zhen Ren", &results, move |_url| {
            let counter = std::sync::Arc::clone(&counter);
            async move {
                let index = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let author = if index == 0 {
                    Some("Gu Zhen Ren")
                } else {
                    None
                };
                FetchedSummary {
                    metadata: Some(metadata(&format!("Novel {index}"), author)),
                    cache_age_seconds: None,
                }
            }
        })
        .await;

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "an exact author match in the first wave must end verification"
        );
        assert_eq!(evidence.len(), 3);
        assert!(
            evidence
                .iter()
                .any(|row| row.author_score >= EXACT_AUTHOR_SCORE)
        );
    }

    #[tokio::test]
    async fn a_first_wave_without_an_exact_author_keeps_verifying() {
        let results: Vec<NovelResult> = (0..6)
            .map(|index| candidate(&format!("Novel {index}"), "novelfire.net", index))
            .collect();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);

        let (evidence, _) = verify_candidates_with("Somebody Else", &results, move |_url| {
            let counter = std::sync::Arc::clone(&counter);
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                FetchedSummary {
                    metadata: Some(metadata("Unrelated", Some("Nobody"))),
                    cache_age_seconds: None,
                }
            }
        })
        .await;

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 6);
        assert_eq!(evidence.len(), 6);
    }

    #[test]
    fn an_empty_author_page_is_trusted_only_when_path_and_label_agree() {
        let mut page = metadata::AuthorPage {
            canonical: Some("https://novelfire.net/author/gu-zhen-ren".to_string()),
            author: Some("Gu Zhen Ren".to_string()),
            novels: Vec::new(),
        };
        assert!(author_page_is_trusted(
            "Gu Zhen Ren",
            "https://novelfire.net/author/gu-zhen-ren",
            &page
        ));

        // A site that answers an unknown slug with its home page must not be
        // believed even if it happens to print the query somewhere.
        page.canonical = Some("https://novelfire.net/".to_string());
        assert!(!author_page_is_trusted(
            "Gu Zhen Ren",
            "https://novelfire.net/author/gu-zhen-ren",
            &page
        ));

        // The stated author must agree with the query too.
        page.canonical = Some("https://novelfire.net/author/someone-else".to_string());
        page.author = Some("Someone Else".to_string());
        assert!(!author_page_is_trusted(
            "Gu Zhen Ren",
            "https://novelfire.net/author/gu-zhen-ren",
            &page
        ));
    }

    #[test]
    fn search_cache_keys_normalise_spelling() {
        assert_eq!(
            search_cache_key("https://NovelFire.net/", "  Reverend  Insanity "),
            search_cache_key("https://novelfire.net", "reverend insanity")
        );
        assert_ne!(
            search_cache_key("https://novelfire.net", "reverend insanity"),
            search_cache_key("https://novelphoenix.net", "reverend insanity")
        );
    }

    #[test]
    fn cache_stamp_keeps_the_oldest_age_and_marks_cached() {
        let mut stamp = CacheStamp::default();
        assert!(!stamp.cached);
        stamp.observe(Some(30));
        stamp.observe(Some(120));
        stamp.observe(None);
        assert!(stamp.cached);
        assert_eq!(stamp.age_seconds, Some(120));
        stamp.merge(CacheStamp {
            cached: false,
            age_seconds: None,
        });
        assert_eq!(stamp.age_seconds, Some(120));
    }
}
