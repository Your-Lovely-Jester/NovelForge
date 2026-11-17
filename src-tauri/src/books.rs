//! Book catalogue search and series resolution (NF-028).
//!
//! The books engine no longer treats a download-site search as the catalogue.
//! Search resolves a real record first (Hardcover when a local token exists,
//! Open Library otherwise), shows verified candidates, and only then hands a
//! title/author/ISBN to `aget`. Series resolution returns the books in reading
//! order, including fractional positions such as `3.5`.

use crate::cache;
use crate::error::{AppError, AppResult};
use crate::model::SourceSearchFailure;
use crate::settings::{Settings, now_secs};
use crate::timeutil::civil_from_days;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const HARDCOVER_ENDPOINT: &str = "https://api.hardcover.app/v1/graphql";
const OPENLIBRARY_SEARCH: &str = "https://openlibrary.org/search.json";
const CACHE_TTL_SECONDS: u64 = 60 * 60;
const HTTP_TIMEOUT_SECONDS: u64 = 20;
const SERIES_MATCH_THRESHOLD: f32 = 0.8;
const TOKEN_FILE: &str = "hardcover-token";

const BOOK_FIELDS: &str = r#"
    id
    title
    subtitle
    slug
    release_date
    release_year
    users_count
    image { url }
    contributions(where: {contributor_role: {slug: {_eq: "author"}}}) {
        author { name }
    }
    default_ebook_edition { isbn_13 isbn_10 edition_format publisher { name } }
    featured_book_series { position series { name } }
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookCandidate {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub language: Option<String>,
    pub series: Option<String>,
    pub series_position: Option<f64>,
    pub cover_url: Option<String>,
    pub source: String,
    pub url: Option<String>,
    pub release_date: Option<String>,
    pub released: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSeries {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub source: String,
    pub books: Vec<BookCandidate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSearchResponse {
    pub query: String,
    pub candidates: Vec<BookCandidate>,
    pub series: Option<BookSeries>,
    pub searched: Vec<String>,
    pub failures: Vec<SourceSearchFailure>,
    pub cached: bool,
    pub cache_age_seconds: Option<u64>,
    pub hardcover_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardcoverStatus {
    pub configured: bool,
    pub path: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookSearchPayload {
    candidates: Vec<BookCandidate>,
    series: Option<BookSeries>,
    searched: Vec<String>,
    failures: Vec<SourceSearchFailure>,
}

impl BookSearchPayload {
    fn empty() -> Self {
        Self {
            candidates: Vec::new(),
            series: None,
            searched: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.candidates.is_empty() && self.series.is_none()
    }
}

// ---------------------------------------------------------------------------
// Local Hardcover token
// ---------------------------------------------------------------------------

pub fn token_path(config_file: &Path) -> PathBuf {
    config_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(TOKEN_FILE)
}

pub fn read_token(config_file: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(token_path(config_file)).ok()?;
    let token = raw.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

pub fn write_token(config_file: &Path, token: &str) -> AppResult<HardcoverStatus> {
    let token = token.trim();
    if token.len() < 20 || token.len() > 4096 {
        return Err(AppError::msg(
            "Hardcover tokens are between 20 and 4096 characters",
        ));
    }
    if !token
        .chars()
        .all(|character| character.is_ascii_graphic() && !character.is_whitespace())
    {
        return Err(AppError::msg("Hardcover token contains invalid characters"));
    }
    let path = token_path(config_file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::fsutil::write_text_atomic(&path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(status(config_file))
}

pub fn clear_token(config_file: &Path) -> AppResult<HardcoverStatus> {
    let path = token_path(config_file);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(AppError::from(error)),
    }
    Ok(status(config_file))
}

pub fn status(config_file: &Path) -> HardcoverStatus {
    let path = token_path(config_file);
    let configured = read_token(config_file).is_some();
    HardcoverStatus {
        configured,
        path: path.to_string_lossy().to_string(),
        detail: Some(if configured {
            "Stored locally; sent only to api.hardcover.app".to_string()
        } else {
            "No token. Open Library is used for catalogue data.".to_string()
        }),
    }
}

// ---------------------------------------------------------------------------
// Search entry point
// ---------------------------------------------------------------------------

pub async fn search(
    _settings: &Settings,
    config_file: &Path,
    query: &str,
    mode: &str,
) -> AppResult<BookSearchResponse> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Err(AppError::msg("Enter a book or series title"));
    }
    let mode = match mode.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => "auto",
        "books" | "book" => "books",
        "series" => "series",
        other => return Err(AppError::msg(format!("Unknown book search mode: {other}"))),
    };

    let token = read_token(config_file);
    let cache_dir = cache::directory("NOVELFORGE_BOOK_CACHE_DIR", "book-cache");
    let cache_key = format!("books::v7::{mode}::{}", query.to_lowercase());
    if let Some(directory) = cache_dir.as_deref()
        && let Some((payload, age)) = cache::read_at::<BookSearchPayload>(
            directory,
            &cache_key,
            now_secs(),
            cache::ttl_seconds("NOVELFORGE_BOOK_TTL_SECS", CACHE_TTL_SECONDS),
        )
    {
        return Ok(response_from(
            payload,
            query,
            token.is_some(),
            true,
            Some(age),
        ));
    }

    let mut failures = Vec::new();
    let mut payload = BookSearchPayload::empty();

    if let Some(token) = token.as_deref() {
        match hardcover_search(token, query, mode).await {
            Ok(found) => payload = found,
            Err(error) => failures.push(SourceSearchFailure {
                source: "hardcover".to_string(),
                label: "Hardcover".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if payload.is_empty() {
        match openlibrary_search(query, mode).await {
            Ok(found) => {
                payload.searched.extend(found.searched);
                payload.candidates = found.candidates;
                payload.series = found.series;
            }
            Err(error) => failures.push(SourceSearchFailure {
                source: "openlibrary".to_string(),
                label: "Open Library".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    payload.failures.extend(failures);

    if !payload.is_empty()
        && let Some(directory) = cache_dir.as_deref()
    {
        cache::write_at(directory, &cache_key, &payload, now_secs());
    }

    Ok(response_from(payload, query, token.is_some(), false, None))
}

fn response_from(
    payload: BookSearchPayload,
    query: &str,
    hardcover_configured: bool,
    cached: bool,
    cache_age_seconds: Option<u64>,
) -> BookSearchResponse {
    BookSearchResponse {
        query: query.to_string(),
        candidates: payload.candidates,
        series: payload.series,
        searched: payload.searched,
        failures: payload.failures,
        cached,
        cache_age_seconds,
        hardcover_configured,
    }
}

// ---------------------------------------------------------------------------
// Hardcover (GraphQL + Typesense search)
// ---------------------------------------------------------------------------

async fn hardcover_search(token: &str, query: &str, mode: &str) -> AppResult<BookSearchPayload> {
    let mut payload = BookSearchPayload {
        searched: vec!["hardcover".to_string()],
        ..BookSearchPayload::empty()
    };

    if mode != "books" {
        match hardcover_series(token, query).await {
            Ok(Some(series)) => {
                payload.series = Some(series);
                if mode == "series" {
                    return Ok(payload);
                }
            }
            Ok(None) => {}
            Err(error) => payload.failures.push(SourceSearchFailure {
                source: "hardcover".to_string(),
                label: "Hardcover series".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    payload.candidates = hardcover_books(token, query).await?;
    Ok(payload)
}

async fn hardcover_books(token: &str, query: &str) -> AppResult<Vec<BookCandidate>> {
    let value = hardcover_query(
        token,
        r#"query SearchBooks($q: String!) {
            search(query: $q, query_type: "Book", per_page: 12, page: 1) { results }
        }"#,
        json!({ "q": query }),
    )
    .await?;

    let results = value
        .get("search")
        .and_then(|search| search.get("results"))
        .cloned()
        .unwrap_or(Value::Null);
    let ids = typesense_ids(&results);

    let books = if ids.is_empty() {
        // Typesense can return no ids for an unusual query; a direct title
        // match is slower but keeps the search useful.
        let fallback = format!(
            r#"query BooksByTitle($q: String!) {{
                books(where: {{title: {{_ilike: $q}}}}, order_by: {{users_count: desc}}, limit: 12) {{
                    {BOOK_FIELDS}
                }}
            }}"#
        );
        hardcover_query(token, &fallback, json!({ "q": format!("%{}%", query) }))
            .await?
            .get("books")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    } else {
        let query_text = format!(
            r#"query BooksByIds($ids: [Int!]!) {{
                books(where: {{id: {{_in: $ids}}}}, limit: 20) {{
                    {BOOK_FIELDS}
                }}
            }}"#
        );
        let value = hardcover_query(token, &query_text, json!({ "ids": ids })).await?;
        let mut by_id: HashMap<i64, Value> = HashMap::new();
        for book in value
            .get("books")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = book.get("id").and_then(Value::as_i64) {
                by_id.insert(id, book.clone());
            }
        }
        ids.iter().filter_map(|id| by_id.get(id).cloned()).collect()
    };

    let mut candidates = books
        .iter()
        .filter_map(parse_hardcover_book)
        .collect::<Vec<_>>();
    candidates.truncate(20);
    Ok(candidates)
}

async fn hardcover_series(token: &str, query: &str) -> AppResult<Option<BookSeries>> {
    let value = hardcover_query(
        token,
        r#"query SearchSeries($q: String!) {
            search(query: $q, query_type: "Series", per_page: 5, page: 1) { results }
        }"#,
        json!({ "q": query }),
    )
    .await?;
    let results = value
        .get("search")
        .and_then(|search| search.get("results"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut best: Option<(f32, i64, String, Option<String>)> = None;
    for document in typesense_documents(&results) {
        let Some(id) = document
            .get("id")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        else {
            continue;
        };
        let Some(name) = string_field(document, "name") else {
            continue;
        };
        let books_count = document
            .get("books_count")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
            .unwrap_or(0);
        if books_count < 2 {
            continue;
        }
        let score = crate::resolver::relevance(query, &name);
        if score < SERIES_MATCH_THRESHOLD {
            continue;
        }
        let author = string_field(document, "author_name");
        if best
            .as_ref()
            .is_none_or(|(current, _, _, _)| score > *current)
        {
            best = Some((score, id, name, author));
        }
    }

    let Some((_, id, name, author)) = best else {
        return Ok(None);
    };

    let query_text = format!(
        r#"query SeriesById($id: Int!) {{
            series(where: {{id: {{_eq: $id}}}}) {{
                id
                name
                author {{ name }}
                book_series(
                    distinct_on: position
                    order_by: [{{position: asc}}, {{book: {{users_count: desc}}}}]
                    where: {{book: {{canonical_id: {{_is_null: true}}, is_partial_book: {{_eq: false}}}}, compilation: {{_eq: false}}}}
                ) {{
                    position
                    book {{
                        {BOOK_FIELDS}
                    }}
                }}
            }}
        }}"#
    );
    let value = hardcover_query(token, &query_text, json!({ "id": id })).await?;
    let series = value
        .get("series")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(Value::Null);

    let mut books: Vec<(Option<f64>, BookCandidate)> = series
        .get("book_series")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let position = entry.get("position").and_then(Value::as_f64);
            let mut book = parse_hardcover_book(entry.get("book")?)?;
            book.series = Some(
                series
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&name)
                    .to_string(),
            );
            book.series_position = position;
            Some((position, book))
        })
        .collect();
    books.sort_by(|left, right| {
        left.0
            .unwrap_or(f64::MAX)
            .partial_cmp(&right.0.unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Some(BookSeries {
        id: series
            .get("id")
            .and_then(|value| value.as_i64())
            .map(|value| value.to_string())
            .unwrap_or_else(|| id.to_string()),
        name: series
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&name)
            .to_string(),
        author: series
            .pointer("/author/name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(author),
        source: "hardcover".to_string(),
        books: books.into_iter().map(|(_, book)| book).collect(),
    }))
}

async fn hardcover_query(token: &str, query: &str, variables: Value) -> AppResult<Value> {
    let response = catalogue_client()?
        .post(HARDCOVER_ENDPOINT)
        .bearer_auth(token)
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|error| AppError::msg(format!("Hardcover request failed: {error}")))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AppError::msg(format!("Hardcover body failed: {error}")))?;
    if !status.is_success() {
        let snippet = body.chars().take(220).collect::<String>();
        return Err(AppError::msg(format!(
            "Hardcover returned {status}: {snippet}"
        )));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| AppError::msg(format!("Hardcover returned invalid JSON: {error}")))?;
    if let Some(errors) = value.get("errors") {
        let snippet = errors.to_string().chars().take(220).collect::<String>();
        return Err(AppError::msg(format!("Hardcover API error: {snippet}")));
    }
    value
        .get("data")
        .cloned()
        .ok_or_else(|| AppError::msg("Hardcover response has no data"))
}

fn typesense_documents(results: &Value) -> Vec<&Value> {
    if let Some(hits) = results.get("hits").and_then(Value::as_array) {
        return hits.iter().filter_map(|hit| hit.get("document")).collect();
    }
    results
        .as_array()
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn typesense_ids(results: &Value) -> Vec<i64> {
    let mut ids = Vec::new();
    for document in typesense_documents(results) {
        let Some(id) = document
            .get("id")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        else {
            continue;
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
        if ids.len() >= 12 {
            break;
        }
    }
    ids
}

fn parse_hardcover_book(value: &Value) -> Option<BookCandidate> {
    let id = value.get("id")?.as_i64()?;
    let title = string_field(value, "title")?;
    let authors = value
        .get("contributions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.pointer("/author/name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let isbn = value
        .pointer("/default_ebook_edition/isbn_13")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/default_ebook_edition/isbn_10")
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    let publisher = value
        .pointer("/default_ebook_edition/publisher/name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let series = value
        .pointer("/featured_book_series/series/name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let series_position = value
        .pointer("/featured_book_series/position")
        .and_then(Value::as_f64);
    let cover_url = value
        .pointer("/image/url")
        .and_then(Value::as_str)
        .map(str::to_string);
    let release_date = value
        .get("release_date")
        .and_then(Value::as_str)
        .map(str::to_string);
    let year = value
        .get("release_year")
        .and_then(Value::as_i64)
        .map(|value| value as i32)
        .or_else(|| {
            release_date
                .as_deref()
                .and_then(|date| date.get(0..4))
                .and_then(|year| year.parse().ok())
        });
    let slug = value.get("slug").and_then(Value::as_str);

    Some(BookCandidate {
        id: format!("hardcover:{id}"),
        title,
        authors,
        year,
        isbn,
        publisher,
        language: None,
        series,
        series_position,
        cover_url,
        source: "hardcover".to_string(),
        url: slug.map(|slug| format!("https://hardcover.app/books/{slug}")),
        released: release_date.as_deref().map(is_released).unwrap_or(true),
        release_date,
    })
}

// ---------------------------------------------------------------------------
// Open Library (no token)
// ---------------------------------------------------------------------------

async fn openlibrary_search(query: &str, mode: &str) -> AppResult<BookSearchPayload> {
    let mut payload = BookSearchPayload {
        searched: vec!["openlibrary".to_string()],
        ..BookSearchPayload::empty()
    };

    if mode != "books"
        && let Some(series) = openlibrary_series(query).await?
    {
        payload.series = Some(series);
        if mode == "series" {
            return Ok(payload);
        }
    }

    payload.candidates = openlibrary_books(query).await?;
    Ok(payload)
}

async fn openlibrary_books(query: &str) -> AppResult<Vec<BookCandidate>> {
    let docs = openlibrary_docs(query).await?;
    let mut candidates = Vec::new();
    let mut seen = Vec::new();
    for document in docs.iter() {
        let Some(candidate) = parse_openlibrary_book(document) else {
            continue;
        };
        let key = (
            candidate.title.to_lowercase(),
            candidate
                .authors
                .first()
                .cloned()
                .unwrap_or_default()
                .to_lowercase(),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        candidates.push(candidate);
        if candidates.len() >= 20 {
            break;
        }
    }
    Ok(candidates)
}

async fn openlibrary_series(query: &str) -> AppResult<Option<BookSeries>> {
    // Open Library rarely tags works with their series, so `series:"X"` misses
    // the actual volumes (it returned only History of Middle-earth books for
    // The Lord of the Rings). Run both queries and confirm membership against
    // each work's editions below.
    // Title results come first: they hold the actual volumes, while the series
    // query often returns companion/reference books that would fill the cap.
    // Both queries run together because each one is a separate round trip.
    let series_query = format!("series:\"{query}\"");
    let (title_docs, series_docs) =
        tokio::join!(openlibrary_docs(query), openlibrary_docs(&series_query));
    let mut docs: Vec<Value> = Vec::new();
    let mut title_error = None;
    match title_docs {
        Ok(found) => docs.extend(found),
        Err(error) => title_error = Some(error),
    }
    match series_docs {
        Ok(found) => docs.extend(found),
        Err(_) if docs.is_empty() => {
            return Err(
                title_error.unwrap_or_else(|| AppError::msg("Open Library series search failed"))
            );
        }
        Err(_) => {}
    }

    let mut parsed: Vec<BookCandidate> = Vec::new();
    let mut seen = HashSet::new();
    for document in docs.iter() {
        let Some(candidate) = parse_openlibrary_book(document) else {
            continue;
        };
        if !seen.insert(candidate.id.clone()) {
            continue;
        }
        parsed.push(candidate);
        if parsed.len() >= 18 {
            break;
        }
    }
    if parsed.len() < 2 {
        return Ok(None);
    }

    let mut books = verify_openlibrary_series(query, parsed).await;
    if books.len() < 2 {
        return Ok(None);
    }

    // A work titled exactly like the series is normally the omnibus/boxed set.
    let series_key = normalize_series_name(query);
    if books.len() > 2 {
        books.retain(|book| {
            normalize_series_name(&book.title) != series_key && !has_compilation_marker(&book.title)
        });
    }
    if books.len() < 2 {
        return Ok(None);
    }

    books.sort_by(|left, right| {
        left.series_position
            .unwrap_or(f64::MAX)
            .partial_cmp(&right.series_position.unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.year
                    .unwrap_or(i32::MAX)
                    .cmp(&right.year.unwrap_or(i32::MAX))
            })
            .then_with(|| left.title.cmp(&right.title))
    });

    let author = books.iter().find_map(|book| book.authors.first().cloned());
    let expected_name = normalize_series_name(query);
    let name = books
        .iter()
        .filter_map(|book| book.series.as_deref())
        .filter(|series| series_names_match(query, series))
        .map(strip_series_position)
        .filter(|name| !name.is_empty())
        .min_by_key(|name| {
            (
                usize::from(!name.trim().eq_ignore_ascii_case(query.trim())),
                usize::from(normalize_series_name(name) != expected_name),
                name.len(),
            )
        })
        .unwrap_or_else(|| query.to_string());

    Ok(Some(BookSeries {
        id: format!("openlibrary:{query}"),
        name,
        author,
        source: "openlibrary".to_string(),
        books,
    }))
}

/// Keep only works whose editions actually name the requested series, and read
/// the position from those editions. This is the verification step that stops
/// a fuzzy title match from becoming a fake series row.
async fn verify_openlibrary_series(query: &str, books: Vec<BookCandidate>) -> Vec<BookCandidate> {
    struct Verified {
        index: usize,
        book: BookCandidate,
        work: String,
        position: Option<f64>,
        name: String,
    }

    let pool: Vec<(usize, BookCandidate)> = books.into_iter().take(14).enumerate().collect();
    let mut verified: Vec<Verified> = Vec::new();
    // Edition fetches are cheap but Open Library throttles bursts. Five at a
    // time keeps the search responsive without turning a 429 into a gap.
    for batch in pool.chunks(5) {
        let mut shallow = tokio::task::JoinSet::new();
        for (index, book) in batch {
            let Some(work) = work_id(book) else {
                continue;
            };
            let query = query.to_string();
            let task_work = work.clone();
            let index = *index;
            let book = book.clone();
            shallow.spawn(async move {
                let (position, name) = lookup_series(&query, &task_work, 40).await?;
                Some(Verified {
                    index,
                    book,
                    work,
                    position,
                    name,
                })
            });
        }
        while let Some(joined) = shallow.join_next().await {
            if let Ok(Some(entry)) = joined {
                verified.push(entry);
            }
        }
    }
    verified.sort_by_key(|entry| entry.index);

    // Only deepen rows that are already members but still lack a number. That
    // keeps the big edition payloads to the one or two works that need them.
    let mut deep = tokio::task::JoinSet::new();
    for (slot, entry) in verified.iter().enumerate() {
        if entry.position.is_some() {
            continue;
        }
        let query = query.to_string();
        let work = entry.work.clone();
        deep.spawn(async move { (slot, lookup_series(&query, &work, 200).await) });
    }
    while let Some(joined) = deep.join_next().await {
        let Ok((slot, Some((position, name)))) = joined else {
            continue;
        };
        if let Some(position) = position {
            verified[slot].position = Some(position);
            if !name.is_empty() {
                verified[slot].name = name;
            }
        }
    }

    verified
        .into_iter()
        .map(|entry| {
            let mut book = entry.book;
            book.series_position = entry.position.or(book.series_position);
            book.series = Some(entry.name);
            book
        })
        .collect()
}

fn work_id(book: &BookCandidate) -> Option<String> {
    book.id
        .strip_prefix("openlibrary:/works/")
        .filter(|work| work.starts_with("OL"))
        .map(str::to_string)
}

async fn lookup_series(query: &str, work: &str, limit: u32) -> Option<(Option<f64>, String)> {
    let value = openlibrary_get(&format!(
        "https://openlibrary.org/works/{work}/editions.json?limit={limit}"
    ))
    .await
    .ok()?;
    edition_series_match(query, &value)
}

/// Find a series entry on any edition of a work. A position is preferred, but
/// an edition that names the series without a number still proves membership.
/// The display name is chosen separately because positioned strings are often
/// messy ("The lord of the rings / J.R.R. Tolkien -- pt. 1").
fn edition_series_match(query: &str, editions: &Value) -> Option<(Option<f64>, String)> {
    let entries = editions.get("entries")?.as_array()?;
    let expected = normalize_series_name(query);
    let mut position: Option<f64> = None;
    let mut names: Vec<String> = Vec::new();

    for entry in entries {
        let Some(series) = entry.get("series").and_then(Value::as_array) else {
            continue;
        };
        for value in series.iter().filter_map(Value::as_str) {
            if !series_names_match(query, value) {
                continue;
            }
            if position.is_none() {
                position = series_position(value);
            }
            let name = strip_series_position(value);
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    if names.is_empty() {
        return None;
    }

    names.sort_by_key(|name| {
        let normalized = normalize_series_name(name);
        let literal = name.trim().eq_ignore_ascii_case(query.trim());
        let messy = name.contains('/')
            || name.contains(';')
            || name.contains("--")
            || name.to_lowercase().contains("pt.")
            || name.contains(',');
        let score = if literal {
            0
        } else if normalized == expected {
            1
        } else if messy {
            3
        } else {
            2
        };
        (score, name.len())
    });
    names.into_iter().next().map(|name| (position, name))
}

fn normalize_series_name(value: &str) -> String {
    strip_series_position(value)
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty() && !matches!(*token, "the" | "a" | "an"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_compilation_marker(title: &str) -> bool {
    let title = title.to_lowercase();
    ["collection", "boxed set", "box set", "omnibus"]
        .iter()
        .any(|marker| title.contains(marker))
}

/// Compare a requested series name with an Open Library series string. A
/// position suffix (" #3.5", ", Book 2") is ignored, and short generic words
/// such as "the" do not make two different series equal.
fn series_names_match(expected: &str, actual: &str) -> bool {
    let expected = normalize_series_name(expected);
    let actual = normalize_series_name(actual);
    if expected.is_empty() || actual.is_empty() {
        return false;
    }
    expected == actual
        || actual.starts_with(&format!("{expected} "))
        || expected.starts_with(&format!("{actual} "))
}

fn catalogue_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("NovelForge/0.1 (desktop download studio)")
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .build()
        .map_err(|error| AppError::msg(format!("Could not create catalogue client: {error}")))
}

async fn openlibrary_get(url: &str) -> AppResult<Value> {
    let client = catalogue_client()?;
    let mut last_error = AppError::msg("Open Library request failed");
    for attempt in 0..3u32 {
        let result = client.get(url).send().await;
        match result {
            Ok(response) => {
                let status = response.status();
                let body = match response.text().await {
                    Ok(body) => body,
                    Err(error) => {
                        last_error = AppError::msg(format!("Open Library body failed: {error}"));
                        if attempt < 2 {
                            tokio::time::sleep(Duration::from_millis(700 * u64::from(attempt + 1)))
                                .await;
                        }
                        continue;
                    }
                };
                if status.is_success() {
                    return serde_json::from_str(&body).map_err(|error| {
                        AppError::msg(format!("Open Library returned invalid JSON: {error}"))
                    });
                }
                let snippet = body.chars().take(160).collect::<String>();
                let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
                last_error = AppError::msg(format!("Open Library returned {status}: {snippet}"));
                if !retryable {
                    return Err(last_error);
                }
            }
            Err(error) => {
                last_error = AppError::msg(format!("Open Library request failed: {error}"));
            }
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(700 * u64::from(attempt + 1))).await;
        }
    }
    Err(last_error)
}

async fn openlibrary_docs(query: &str) -> AppResult<Vec<Value>> {
    let mut url = url::Url::parse(OPENLIBRARY_SEARCH)
        .map_err(|error| AppError::msg(format!("Invalid Open Library URL: {error}")))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair(
            "fields",
            "key,title,author_name,first_publish_year,isbn,publisher,language,series,cover_i,edition_count",
        )
        .append_pair("limit", "50");
    let value = openlibrary_get(url.as_str()).await?;
    Ok(value
        .get("docs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn parse_openlibrary_book(value: &Value) -> Option<BookCandidate> {
    let key = string_field(value, "key")?;
    let title = string_field(value, "title")?;
    let authors = value
        .get("author_name")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let isbn = value
        .get("isbn")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    let publisher = value
        .get("publisher")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    let language = value
        .get("language")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::to_string)
        .map(|code| {
            if code.eq_ignore_ascii_case("eng") || code.eq_ignore_ascii_case("/languages/eng") {
                "English".to_string()
            } else {
                code
            }
        });
    let series_raw = value
        .get("series")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    let series_position = series_raw.as_deref().and_then(series_position);
    let series = series_raw.map(|series| strip_series_position(&series));
    let cover_url = value
        .get("cover_i")
        .and_then(Value::as_i64)
        .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-M.jpg"));
    let year = value
        .get("first_publish_year")
        .and_then(Value::as_i64)
        .map(|year| year as i32);

    Some(BookCandidate {
        id: format!("openlibrary:{key}"),
        title,
        authors,
        year,
        isbn,
        publisher,
        language,
        series,
        series_position,
        cover_url,
        source: "openlibrary".to_string(),
        url: Some(format!("https://openlibrary.org{key}")),
        release_date: None,
        released: true,
    })
}

fn series_position(value: &str) -> Option<f64> {
    // Series strings appear as "The Empyrean #3.5", "The Empyrean, Book 2"
    // and "The Empyrean Book 2". Take the last numeric token, which is the
    // position in all of those forms.
    let mut best = None;
    let mut current = String::new();
    let flush = |current: &mut String, best: &mut Option<f64>| {
        let token = current.trim_matches('.').to_string();
        current.clear();
        if let Ok(position) = token.parse::<f64>() {
            *best = Some(position);
        }
    };
    for character in value.chars() {
        if character.is_ascii_digit()
            || (character == '.' && !current.is_empty() && !current.ends_with('.'))
        {
            current.push(character);
        } else if !current.is_empty() {
            flush(&mut current, &mut best);
        }
    }
    if !current.is_empty() {
        flush(&mut current, &mut best);
    }
    best
}

fn strip_series_position(value: &str) -> String {
    let mut name = value.split(" #").next().unwrap_or(value).to_string();
    // Also handle "Series, Book 3" and "Series Book 3".
    for marker in [", Book", " Book"] {
        if let Some(index) = name.find(marker) {
            let (head, tail) = name.split_at(index);
            if tail.chars().any(|character| character.is_ascii_digit()) {
                name = head.to_string();
                break;
            }
        }
    }
    // Editions often append the position as "Series (#1)".
    if let Some(open) = name.rfind('(') {
        let tail = &name[open..];
        if tail.ends_with(')') && tail.chars().any(|character| character.is_ascii_digit()) {
            name.truncate(open);
        }
    }
    name.trim()
        .trim_end_matches(['#', '-', ':', ','])
        .trim()
        .to_string()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

fn is_released(date: &str) -> bool {
    let Some(day) = date.get(0..10) else {
        return true;
    };
    day <= today_string().as_str()
}

fn today_string() -> String {
    let days = (now_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_typesense_hits() {
        let results = json!({
            "found": 2,
            "hits": [
                { "document": { "id": 7 } },
                { "document": { "id": "9" } }
            ]
        });
        assert_eq!(typesense_ids(&results), vec![7, 9]);
    }

    #[test]
    fn parses_a_hardcover_book() {
        let value = json!({
            "id": 101,
            "title": "Fourth Wing",
            "slug": "fourth-wing",
            "release_date": "2023-05-02",
            "release_year": 2023,
            "image": { "url": "https://example.test/cover.jpg" },
            "contributions": [
                { "author": { "name": "Rebecca Yarros" } }
            ],
            "default_ebook_edition": {
                "isbn_13": "9781649374042",
                "publisher": { "name": "Entangled" }
            },
            "featured_book_series": {
                "position": 1.0,
                "series": { "name": "The Empyrean" }
            }
        });
        let book = parse_hardcover_book(&value).expect("book");
        assert_eq!(book.title, "Fourth Wing");
        assert_eq!(book.authors, vec!["Rebecca Yarros".to_string()]);
        assert_eq!(book.isbn.as_deref(), Some("9781649374042"));
        assert_eq!(book.series.as_deref(), Some("The Empyrean"));
        assert_eq!(book.series_position, Some(1.0));
        assert_eq!(book.year, Some(2023));
        assert!(book.released);
    }

    #[test]
    fn reads_positions_from_edition_series_strings() {
        let editions = json!({
            "entries": [
                { "series": ["The Empyrean"] },
                { "series": ["The Empyrean (#1)"] }
            ]
        });
        assert_eq!(
            edition_series_match("The Empyrean", &editions),
            Some((Some(1.0), "The Empyrean".to_string()))
        );
        let novella = json!({ "entries": [{ "series": ["The Empyrean #3.5"] }] });
        assert_eq!(
            edition_series_match("The Empyrean", &novella).map(|item| item.0),
            Some(Some(3.5))
        );
        let unrelated = json!({ "entries": [{ "series": ["Painted Edges Paperback"] }] });
        assert_eq!(edition_series_match("The Empyrean", &unrelated), None);
        let messy = json!({
            "entries": [
                { "series": ["The lord of the rings / J.R.R. Tolkien -- pt. 1"] },
                { "series": ["The Lord of the Rings, #1"] }
            ]
        });
        assert_eq!(
            edition_series_match("The Lord of the Rings", &messy),
            Some((Some(1.0), "The Lord of the Rings".to_string()))
        );
        let article = json!({
            "entries": [{ "series": ["Empyrean"] }, { "series": ["The Empyrean (#1)"] }]
        });
        assert_eq!(
            edition_series_match("The Empyrean", &article),
            Some((Some(1.0), "The Empyrean".to_string()))
        );
        let history = json!({
            "entries": [{ "series": ["The history of the Lord of the Rings -- 1"] }]
        });
        assert_eq!(
            edition_series_match("The Lord of the Rings", &history),
            None
        );
    }

    #[test]
    fn matches_series_names_without_accepting_prefix_noise() {
        assert!(series_names_match("The Empyrean", "The Empyrean #3.5"));
        assert!(series_names_match(
            "The Lord of the Rings",
            "The Lord of the Rings, Book 2"
        ));
        assert!(!series_names_match(
            "The Empyrean",
            "Chronicles of the Empyrean: the Guardians"
        ));
        assert!(!series_names_match("The Empyrean", "The"));
    }

    #[test]
    fn parses_series_positions_including_fractional() {
        assert_eq!(series_position("The Empyrean #3.5"), Some(3.5));
        assert_eq!(series_position("The Empyrean, Book 2"), Some(2.0));
        assert_eq!(series_position("The Empyrean"), None);
        assert_eq!(strip_series_position("The Empyrean #3.5"), "The Empyrean");
        assert_eq!(
            strip_series_position("The Empyrean, Book 2"),
            "The Empyrean"
        );
    }

    #[test]
    fn parses_openlibrary_series_rows() {
        let value = json!({
            "key": "/works/OL1W",
            "title": "Threshing Day",
            "author_name": ["Rebecca Yarros"],
            "first_publish_year": 2026,
            "series": ["The Empyrean #3.5"],
            "cover_i": 1234,
            "isbn": ["9780000000000"]
        });
        let book = parse_openlibrary_book(&value).expect("book");
        assert_eq!(book.series_position, Some(3.5));
        assert_eq!(book.series.as_deref(), Some("The Empyrean"));
        assert_eq!(
            book.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/1234-M.jpg")
        );
    }

    #[test]
    fn future_release_dates_are_upcoming() {
        assert!(is_released("2020-01-01"));
        assert!(!is_released("2999-01-01"));
    }

    #[test]
    fn token_round_trip_uses_a_local_file() {
        let dir = std::env::temp_dir().join(format!("novelforge-books-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        let config = dir.join("settings.json");
        let token = "hc_live_abcdefghijklmnopqrstuvwxyz0123456789";
        let status = write_token(&config, token).expect("write");
        assert!(status.configured);
        assert!(status.path.ends_with("hardcover-token"));
        assert_eq!(read_token(&config).as_deref(), Some(token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(&config))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        clear_token(&config).expect("clear");
        assert!(read_token(&config).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_tokens_with_whitespace() {
        let dir = std::env::temp_dir().join(format!("novelforge-books-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        let config = dir.join("settings.json");
        assert!(write_token(&config, "bad token value with spaces and length").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
