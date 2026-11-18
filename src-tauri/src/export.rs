//! Turns a crawler artifact into the reader-friendly structured download:
//!
//! ```text
//! <novel>/
//!   metadata.json
//!   index.json
//!   chapters.html
//!   cover.jpg
//!   chapters/0001 - Chapter title.xhtml
//!   <novel>.epub
//! ```
//!
//! The bundled EPUB stays for external readers. The split XHTML files and
//! `metadata.json` power the fast in-app reader. Manga archives are split into
//! image folders the same way.
//!
//! `metadata.json` is the canonical machine record. `index.json` mirrors the
//! chapter list in a compact form and `chapters.html` is a self-contained,
//! searchable page that works outside the app. Both mark every source chapter
//! as downloaded or missing.

use crate::metadata;
use crate::model::{
    ExportMetadata, ExportVerification, NovelChapter, NovelChapterIndex, NovelMetadata,
};
use base64::Engine as _;
use regex::Regex;
use scraper::{Html, Selector};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use url::Url;
use walkdir::WalkDir;
use zip::ZipArchive;

pub const MIN_CHAPTER_CHARS: usize = 64;
const MAX_STRUCTURED_ASSET_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct EpubChapter {
    pub position: u32,
    pub title: String,
    pub archive_path: String,
    pub text_chars: usize,
}

#[derive(Debug, Clone)]
pub struct EpubAudit {
    pub title: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub chapters: Vec<EpubChapter>,
    pub has_cover: bool,
    pub cover_name: Option<String>,
    pub cover_bytes: Option<Vec<u8>>,
    pub opf_path: String,
    pub opf_dir: String,
}

#[derive(Debug, Clone, Default)]
pub struct AuditPlan {
    pub expected: Option<u32>,
    pub found: u32,
    pub empty_serials: Vec<u32>,
    pub empty_positions: Vec<u32>,
    pub missing_serials: Vec<u32>,
    /// NF-044: positions whose EPUB heading names a different chapter than the
    /// selection did. This is how a `--chapters` spec that selected the
    /// neighbouring row is caught instead of exported under the right label.
    pub misnumbered_positions: Vec<u32>,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct CbzChapter {
    pub position: u32,
    pub title: String,
    pub images: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, Default)]
pub struct CbzAudit {
    pub chapters: Vec<CbzChapter>,
    pub image_count: usize,
    pub cover_name: Option<String>,
    pub cover_bytes: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// EPUB audit
// ---------------------------------------------------------------------------

pub fn audit_epub(path: &Path) -> Result<EpubAudit, String> {
    let file = File::open(path).map_err(|error| format!("Cannot open EPUB: {error}"))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .map_err(|error| format!("Invalid EPUB ZIP: {error}"))?;

    let names = archive_names(&archive);
    let container = read_entry(&mut archive, "META-INF/container.xml")
        .ok_or_else(|| "EPUB has no META-INF/container.xml".to_string())?;
    let container = String::from_utf8_lossy(&container);
    let opf_path = parse_container_opf(&container)
        .or_else(|| names.iter().find(|name| name.ends_with(".opf")).cloned())
        .ok_or_else(|| "EPUB has no package document".to_string())?;
    let opf = read_entry(&mut archive, &opf_path)
        .ok_or_else(|| format!("EPUB package document {opf_path} is missing"))?;
    let package = parse_package(&String::from_utf8_lossy(&opf))?;
    let opf_dir = parent_dir(&opf_path);

    let mut cover_bytes = None;
    let mut cover_name = None;
    for candidate in package.cover_candidates() {
        let archive_path = join_archive(&opf_dir, &candidate);
        if let Some(bytes) = read_entry(&mut archive, &archive_path)
            && !bytes.is_empty()
        {
            cover_name = Some(archive_path);
            cover_bytes = Some(bytes);
            break;
        }
    }

    let mut chapters = Vec::new();
    for (index, item) in package.spine_items().into_iter().enumerate() {
        let archive_path = join_archive(&opf_dir, &item.href);
        let Some(bytes) = read_entry(&mut archive, &archive_path) else {
            continue;
        };
        let html = String::from_utf8_lossy(&bytes);
        let text_chars = visible_text_len(&html);
        if !is_chapter_entry(&item.href, &html, text_chars) {
            continue;
        }
        chapters.push(EpubChapter {
            position: (index + 1) as u32,
            title: chapter_title(&html, &item.href),
            archive_path,
            text_chars,
        });
    }

    // Some converters do not put chapter XHTML in the spine. Fall back to all
    // manifest XHTML entries that look like chapters.
    if chapters.is_empty() {
        for (index, item) in package.manifest.iter().enumerate() {
            if !is_xhtml(&item.href) {
                continue;
            }
            let archive_path = join_archive(&opf_dir, &item.href);
            let Some(bytes) = read_entry(&mut archive, &archive_path) else {
                continue;
            };
            let html = String::from_utf8_lossy(&bytes);
            let text_chars = visible_text_len(&html);
            if !is_chapter_entry(&item.href, &html, text_chars) {
                continue;
            }
            chapters.push(EpubChapter {
                position: (index + 1) as u32,
                title: chapter_title(&html, &item.href),
                archive_path,
                text_chars,
            });
        }
    }

    // Renumber the content chapters themselves. The spine also contains cover,
    // front matter and navigation entries, so raw spine positions would start
    // at 4 or 5 and produce confusing chapter filenames.
    for (index, chapter) in chapters.iter_mut().enumerate() {
        chapter.position = (index + 1) as u32;
    }

    Ok(EpubAudit {
        title: package.title,
        author: package.author,
        description: package.description,
        language: package.language,
        has_cover: cover_bytes.is_some(),
        cover_name,
        cover_bytes,
        opf_path,
        opf_dir,
        chapters,
    })
}

pub fn plan_audit(
    audit: &EpubAudit,
    selection: Option<&str>,
    expected_total: Option<u32>,
    index: Option<&NovelChapterIndex>,
) -> AuditPlan {
    let serials = selection.and_then(parse_selection_serials);
    let expected = serials
        .as_ref()
        .map(|values| values.len() as u32)
        .or(expected_total)
        .or_else(|| index.map(|value| value.total));

    let mut plan = AuditPlan {
        expected,
        found: audit.chapters.len() as u32,
        ..Default::default()
    };

    let mut position_to_serial = HashMap::<u32, u32>::new();
    if let Some(serials) = &serials {
        for (offset, serial) in serials.iter().enumerate() {
            position_to_serial.insert(offset as u32 + 1, *serial);
        }
    } else {
        for chapter in &audit.chapters {
            let serial = chapter_number_from_title(&chapter.title).unwrap_or(chapter.position);
            position_to_serial.insert(chapter.position, serial);
        }
    }

    for chapter in &audit.chapters {
        if chapter.text_chars < MIN_CHAPTER_CHARS {
            plan.empty_positions.push(chapter.position);
            if let Some(serial) = position_to_serial.get(&chapter.position) {
                plan.empty_serials.push(*serial);
            }
        }
    }
    plan.empty_positions.sort_unstable();
    plan.empty_serials.sort_unstable();
    plan.empty_serials.dedup();

    // NF-044: the selection uses the numbers the source shows, but lncrawl
    // selected rows by serial. A count-only audit passes when a spec is one row
    // off, so compare the heading of each fetched chapter with the number the
    // selection asked for. Only a fully numbered, strictly increasing run is
    // evidence: volume-local restarts and unnamed headings are left alone.
    let mut first_mismatch: Option<(u32, u32, u32)> = None;
    if let Some(values) = &serials
        && audit.chapters.len() == values.len()
    {
        let numbered = audit
            .chapters
            .iter()
            .map(|chapter| chapter_number_from_title(&chapter.title))
            .collect::<Vec<_>>();
        if numbered.iter().all(Option::is_some) && numbered.windows(2).all(|pair| pair[0] < pair[1])
        {
            for (chapter, (expected, actual)) in audit
                .chapters
                .iter()
                .zip(values.iter().zip(numbered.iter()))
            {
                if let Some(actual) = actual
                    && actual != expected
                {
                    plan.misnumbered_positions.push(chapter.position);
                    first_mismatch.get_or_insert((chapter.position, *actual, *expected));
                }
            }
        }
    }

    let downloaded_serials: HashSet<u32> = position_to_serial.values().copied().collect();
    if let Some(values) = &serials {
        plan.missing_serials = values
            .iter()
            .copied()
            .filter(|serial| !downloaded_serials.contains(serial))
            .collect();
    } else if let Some(index) = index {
        // When the EPUB already has the full source chapter count, minor title
        // differences are not evidence of missing chapters. Only build a
        // missing list when the counts actually disagree.
        if plan.found < index.total {
            let mut matched = HashSet::new();
            for chapter in &audit.chapters {
                if let Some(number) = chapter_number_from_title(&chapter.title) {
                    matched.insert(number);
                }
            }
            for source in &index.chapters {
                if !matched.contains(&source.number) {
                    plan.missing_serials.push(source.number);
                }
            }
            plan.missing_serials.sort_unstable();
            plan.missing_serials.dedup();
        }
    }

    let count_ok = expected.is_none_or(|value| plan.found >= value);
    let missing_known_ok = plan.missing_serials.is_empty();
    plan.ok = count_ok
        && missing_known_ok
        && plan.empty_positions.is_empty()
        && plan.misnumbered_positions.is_empty();
    plan.detail = if plan.ok {
        match expected {
            Some(value) => format!("{}/{} chapters present and non-empty", plan.found, value),
            None => format!("{} chapters present and non-empty", plan.found),
        }
    } else {
        let mut parts = Vec::new();
        if let Some(value) = expected
            && plan.found < value
        {
            parts.push(format!("{} of {} chapters found", plan.found, value));
        }
        if !plan.missing_serials.is_empty() {
            parts.push(format!(
                "missing chapter(s): {}",
                preview_numbers(&plan.missing_serials)
            ));
        }
        if !plan.empty_positions.is_empty() {
            parts.push(format!(
                "empty chapter(s): {}",
                preview_numbers(&plan.empty_positions)
            ));
        }
        if let Some((position, actual, expected)) = first_mismatch {
            parts.push(format!(
                "chapter numbering does not match the selection (position {position} is Chapter {actual}, expected Chapter {expected})"
            ));
        }
        if parts.is_empty() {
            "chapter verification failed".to_string()
        } else {
            parts.join(" · ")
        }
    };
    plan
}

fn preview_numbers(numbers: &[u32]) -> String {
    let preview = numbers
        .iter()
        .take(12)
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if numbers.len() > 12 {
        format!("{preview}, … (+{})", numbers.len() - 12)
    } else {
        preview
    }
}

// ---------------------------------------------------------------------------
// Structured EPUB export
// ---------------------------------------------------------------------------

pub fn export_epub(
    epub_path: &Path,
    destination: &Path,
    metadata: &NovelMetadata,
    index: Option<&NovelChapterIndex>,
    selection: Option<&str>,
    verification: &ExportVerification,
    index_required: bool,
) -> Result<ExportMetadata, String> {
    let mut archive = open_archive(epub_path)?;
    let audit = audit_epub(epub_path)?;
    let chapters_dir = destination.join("chapters");
    std::fs::create_dir_all(&chapters_dir)
        .map_err(|error| format!("Cannot create chapters folder: {error}"))?;

    let title = first_non_empty([
        audit.title.as_deref(),
        Some(metadata.title.as_str()),
        Some("Untitled download"),
    ]);
    // `metadata.author` is the primary credited name; the EPUB creator may be a
    // comma-joined list from the source adapter. Keep `authors` for the list.
    let author = non_empty(first_non_empty([
        metadata.author.as_deref(),
        audit.author.as_deref(),
    ]));
    let language = non_empty(first_non_empty([
        audit.language.as_deref(),
        metadata.language.as_deref(),
    ]));
    let synopsis = non_empty(first_non_empty([
        metadata.synopsis.as_deref(),
        audit.description.as_deref(),
    ]));

    // Copy the stylesheet and image assets so the split XHTML files still render.
    let mut copied_assets = 0_u64;
    for name in archive_names(&archive) {
        if !is_asset(&name) {
            continue;
        }
        if audit.cover_name.as_deref() == Some(name.as_str()) {
            continue;
        }
        if copied_assets >= MAX_STRUCTURED_ASSET_BYTES {
            break;
        }
        let relative = name
            .strip_prefix(&format!("{}/", audit.opf_dir))
            .unwrap_or(&name);
        // A crafted EPUB can name an entry `../../escape` or an absolute
        // Windows path. Keep only normal, relative components.
        let Some(relative) = safe_entry_path(relative) else {
            continue;
        };
        let target = chapters_dir.join(relative);
        if target.exists() {
            continue;
        }
        if let Some(bytes) = read_entry(&mut archive, &name) {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&target, &bytes).is_ok() {
                copied_assets += bytes.len() as u64;
            }
        }
    }

    let mut downloaded_files = HashMap::<u32, String>::new();
    let cleaner = crate::chapter_clean::ChapterCleaner::for_source(Some(metadata.source.as_str()));
    let mut written = Vec::new();
    // NF-043: when the index has a real name the EPUB lacks (borrowed from a
    // verified donor, or recovered from the source's own chapter page), the
    // EPUB lncrawl produced still says "Chapter 12" in its headings and
    // navigation. Collect the rows to rewrite so the bundled EPUB matches the
    // structured folder and index the reader shows.
    let mut retitle = Vec::<(String, String)>::new();
    let selection_serials = selection.and_then(parse_selection_serials);
    for (offset, chapter) in audit.chapters.iter().enumerate() {
        let Some(raw) = read_entry(&mut archive, &chapter.archive_path) else {
            continue;
        };
        let serial = selection_serials
            .as_ref()
            .and_then(|values| values.get(offset).copied())
            .or_else(|| chapter_number_from_title(&chapter.title))
            .unwrap_or(chapter.position);
        // Prefer the source index title for duplicate-title detection: the
        // EPUB heading is often just "Chapter 1" while the index carries the
        // full chapter name.
        let source_title = index
            .and_then(|value| value.chapters.iter().find(|source| source.number == serial))
            .map(|source| source.title.as_str())
            .unwrap_or(&chapter.title);
        let cleaned = cleaner.clean(&raw, source_title);
        let bytes = cleaned.as_deref().unwrap_or(&raw);
        // Filenames use the chapter serial, not the EPUB spine position, so a
        // partial update (chapters 3–4) cannot overwrite preserved files for
        // chapters 1–2. Prefer the source index title: when a numbering-only
        // source borrowed names from a verified donor, the file should carry
        // the same name the EPUB and index show.
        let file_name = chapter_file_name(serial, source_title);
        let relative = format!("chapters/{file_name}");
        let target = destination.join(&relative);
        std::fs::write(&target, bytes)
            .map_err(|error| format!("Cannot write chapter file: {error}"))?;
        written.push((chapter.clone(), relative.clone()));

        if source_title.trim() != chapter.title.trim() && carries_name(source_title) {
            retitle.push((
                chapter.archive_path.clone(),
                epub_heading(serial, source_title),
            ));
        }

        downloaded_files.insert(serial, relative);
    }

    // Match by title as well, which fixes numbering gaps after a retry. Titles
    // are normalised once per chapter instead of once per comparison: for a
    // 2,334-chapter book the nested scan used to normalise 5.4 M strings after
    // every chapter file was already on disk (NF-038).
    let written_paths = written
        .iter()
        .map(|(chapter, relative)| (chapter.archive_path.as_str(), relative.as_str()))
        .collect::<HashMap<_, _>>();
    if let Some(index) = index {
        let normalized_index = index
            .chapters
            .iter()
            .map(|source| normalize_title(&source.title))
            .collect::<Vec<_>>();
        for chapter in &audit.chapters {
            let normalized = normalize_title(&chapter.title);
            let Some(offset) = normalized_index
                .iter()
                .position(|title| *title == normalized)
            else {
                continue;
            };
            let source = &index.chapters[offset];
            if let Some(relative) = written_paths.get(chapter.archive_path.as_str()) {
                downloaded_files.insert(source.number, (*relative).to_string());
            }
        }
    }

    let cover = write_cover(
        destination,
        audit.cover_bytes.as_deref(),
        metadata.cover.as_deref(),
    );

    let mut chapters = if let Some(index) = index {
        index
            .chapters
            .iter()
            .map(|source| {
                let file = downloaded_files.get(&source.number).cloned();
                NovelChapter {
                    number: source.number,
                    title: source.title.clone(),
                    url: source.url.clone(),
                    volume: source.volume,
                    downloaded: file.is_some(),
                    file,
                }
            })
            .collect::<Vec<_>>()
    } else {
        let serials = selection.and_then(parse_selection_serials);
        written
            .iter()
            .enumerate()
            .map(|(offset, (chapter, file))| NovelChapter {
                number: serials
                    .as_ref()
                    .and_then(|values| values.get(offset).copied())
                    .or_else(|| chapter_number_from_title(&chapter.title))
                    .unwrap_or(chapter.position),
                title: chapter.title.clone(),
                url: metadata.source_url.clone(),
                volume: None,
                downloaded: true,
                file: Some(file.clone()),
            })
            .collect()
    };
    chapters.sort_by_key(|chapter| chapter.number);

    let chapter_count = metadata
        .chapter_count
        .or_else(|| index.map(|value| value.total))
        .unwrap_or(audit.chapters.len() as u32)
        .max(audit.chapters.len() as u32);

    let epub_name = epub_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "book.epub".to_string());

    let export = ExportMetadata {
        title: title.to_string(),
        author,
        authors: metadata.authors.clone(),
        synopsis,
        tags: metadata.tags.clone(),
        status: metadata.status.clone(),
        language,
        source_url: non_empty(&metadata.source_url),
        source: non_empty(&metadata.source),
        cover,
        epub: Some(epub_name),
        chapter_count,
        downloaded_chapters: written.len() as u32,
        selection: selection.map(str::to_string),
        index_complete: index.is_some_and(|value| value.complete),
        index_detail: index.and_then(|value| value.detail.clone()),
        complete: verification.ok && (!index_required || index.is_some_and(|value| value.complete)),
        provenance: None,
        crawl: None,
        chapters,
        generated_at: crate::settings::now_secs(),
        verified: verification.clone(),
    };
    let mut export = export;
    // Updating an existing folder keeps chapters that were not part of this
    // run, so a partial refresh never destroys previously downloaded chapters.
    merge_previous_export(destination, &mut export);
    // NF-039: the live structurer names files from the chapter body, which can
    // differ from the authoritative EPUB heading. Drop the provisional names
    // this export replaced, and keep any that belong to chapters the export
    // could not produce.
    // NF-044: lncrawl names the streamed frame by catalogue serial while the
    // export names it by the source's visible number; compare the two through
    // the index so a 0-based book does not leave `0004` behind for Chapter 3.
    let serial_to_visible = index
        .map(|index| {
            index
                .chapters
                .iter()
                .enumerate()
                .map(|(offset, chapter)| (offset as u32 + 1, chapter.number))
                .collect::<HashMap<u32, u32>>()
        })
        .unwrap_or_default();
    crate::structurer::sweep(destination, &export, &serial_to_visible);
    write_metadata(destination, &export)?;
    write_index_files(destination, &export)?;
    if !retitle.is_empty() {
        // Best effort: the EPUB is only rewritten after the replacement has
        // been audited, and a failure leaves lncrawl's original in place. The
        // structured folder and index already carry the borrowed names.
        let _ = retitle_epub(epub_path, &retitle);
    }
    Ok(export)
}

// ---------------------------------------------------------------------------
// EPUB heading retitling
// ---------------------------------------------------------------------------

/// True when a chapter title says more than its number ("Chapter 12" and "12"
/// are not names; "Chapter 12 - A name" and "A name" are).
pub(crate) fn carries_name(title: &str) -> bool {
    title.chars().any(char::is_alphabetic)
        && (chapter_number_from_title(title).is_none() || strip_chapter_prefix(title).is_some())
}

/// The reader-facing heading for a chapter whose name was borrowed: the number
/// is kept so a later audit can still recover the serial from the title.
pub(crate) fn epub_heading(number: u32, title: &str) -> String {
    let title = title.trim();
    if chapter_number_from_title(title) == Some(number) {
        return title.to_string();
    }
    let name = strip_chapter_prefix(title).unwrap_or_else(|| title.to_string());
    format!("Chapter {number} - {name}")
}

/// "Chapter 12 - A name", "Chapter 12: A name" and "Chapter 12 – A name" all
/// carry the same redundant prefix lncrawl repeats; keep only the name.
fn strip_chapter_prefix(value: &str) -> Option<String> {
    static PREFIX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let prefix = PREFIX.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:chapter|ch\.?)\s*#?\s*\d+\s*(?:\(end\))?\s*[-–—:.]\s*")
            .expect("valid chapter prefix regex")
    });
    // `find`, not `replace`: a title with no numbering must not report the
    // whole title as the name behind a prefix that was never there.
    let matched = prefix.find(value)?;
    let name = value[matched.end()..].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Rewrite the headings inside a bundled EPUB. `targets` pairs an archive path
/// with the heading it should carry. The EPUB is rebuilt into a temporary file,
/// audited, and only then swapped in, so a bad rewrite can never replace a
/// valid artifact.
pub(crate) fn retitle_epub(path: &Path, targets: &[(String, String)]) -> Result<usize, String> {
    let labels: HashMap<&str, &str> = targets
        .iter()
        .map(|(archive, label)| (archive.as_str(), label.as_str()))
        .collect();
    if labels.is_empty() {
        return Ok(0);
    }
    let file = File::open(path).map_err(|error| format!("Cannot open EPUB: {error}"))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .map_err(|error| format!("Invalid EPUB ZIP: {error}"))?;

    let temporary = crate::fsutil::sibling_temp(path, "retitle");
    let output =
        File::create(&temporary).map_err(|error| format!("Cannot create EPUB: {error}"))?;
    let mut writer = zip::ZipWriter::new(output);
    let mut changed = 0_usize;

    for index in 0..archive.len() {
        let (name, is_dir, mut bytes): (String, bool, Vec<u8>) = {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| format!("Cannot read EPUB entry: {error}"))?;
            let name = entry
                .name()
                .map_err(|error| format!("Cannot read EPUB entry name: {error}"))?
                .to_string();
            let is_dir = name.ends_with('/');
            let mut bytes: Vec<u8> = Vec::new();
            if !is_dir {
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|error| format!("Cannot read EPUB entry: {error}"))?;
            }
            (name, is_dir, bytes)
        };
        let options = if name == "mimetype" {
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
        } else {
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
        };
        if is_dir {
            writer
                .add_directory(name, options)
                .map_err(|error| format!("Cannot write EPUB entry: {error}"))?;
            continue;
        }
        if let Some(label) = labels.get(name.as_str()) {
            let html = String::from_utf8_lossy(&bytes).to_string();
            let updated = retitle_chapter_xhtml(&html, label);
            if updated != html {
                bytes = updated.into_bytes();
                changed += 1;
            }
        } else if name.ends_with("nav.xhtml") {
            let html = String::from_utf8_lossy(&bytes).to_string();
            let base = parent_dir(&name);
            let updated = retitle_nav(&html, &labels, &base);
            if updated != html {
                bytes = updated.into_bytes();
                changed += 1;
            }
        } else if name.ends_with(".ncx") {
            let ncx = String::from_utf8_lossy(&bytes).to_string();
            let base = parent_dir(&name);
            let updated = retitle_ncx(&ncx, &labels, &base);
            if updated != ncx {
                bytes = updated.into_bytes();
                changed += 1;
            }
        }
        writer
            .start_file(name, options)
            .map_err(|error| format!("Cannot write EPUB entry: {error}"))?;
        writer
            .write_all(&bytes)
            .map_err(|error| format!("Cannot write EPUB entry: {error}"))?;
    }
    writer
        .finish()
        .map_err(|error| format!("Cannot finish EPUB: {error}"))?;

    // Nothing actually differed (for example a re-export of a folder that was
    // already retitled): keep the original bytes.
    if changed == 0 {
        let _ = std::fs::remove_file(&temporary);
        return Ok(0);
    }

    // Never swap in a file that no longer audits as an EPUB.
    if let Err(error) = audit_epub(&temporary) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    crate::fsutil::replace_file(&temporary, path)
        .map_err(|error| format!("Cannot replace EPUB: {error}"))?;
    Ok(changed)
}

fn retitle_chapter_xhtml(html: &str, label: &str) -> String {
    static TITLE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let title = TITLE
        .get_or_init(|| Regex::new(r"(?is)<title(\s[^>]*)?>.*?</title>").expect("title regex"));
    static HEADING: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let heading =
        HEADING.get_or_init(|| Regex::new(r"(?is)<h1(\s[^>]*)?>.*?</h1>").expect("h1 regex"));

    let updated = replace_first_tag(title, html, "title", label);
    replace_first_tag(heading, &updated, "h1", label)
}

/// Replace the first `<tag …>…</tag>`, keeping any attributes on the tag.
fn replace_first_tag(pattern: &Regex, html: &str, tag: &str, label: &str) -> String {
    let escaped = escape_html(label);
    pattern
        .replace(html, |captures: &regex::Captures| {
            let attributes = captures.get(1).map(|value| value.as_str()).unwrap_or("");
            format!("<{tag}{attributes}>{escaped}</{tag}>")
        })
        .to_string()
}

/// `EPUB/nav.xhtml` links to chapters relative to its own folder.
fn retitle_nav(html: &str, labels: &HashMap<&str, &str>, base: &str) -> String {
    static ANCHOR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let anchor = ANCHOR.get_or_init(|| {
        Regex::new(r#"(?is)<a\b([^>]*?)href\s*=\s*"([^"]*)"([^>]*)>.*?</a>"#)
            .expect("valid nav anchor regex")
    });
    anchor
        .replace_all(html, |captures: &regex::Captures| {
            let whole = captures.get(0).map(|value| value.as_str()).unwrap_or("");
            let prefix = captures.get(1).map(|value| value.as_str()).unwrap_or("");
            let href = captures.get(2).map(|value| value.as_str()).unwrap_or("");
            let suffix = captures.get(3).map(|value| value.as_str()).unwrap_or("");
            let Some(label) = labels.get(join_archive(base, href).as_str()) else {
                return whole.to_string();
            };
            format!(
                "<a{prefix}href=\"{href}\"{suffix}>{}</a>",
                escape_html(label)
            )
        })
        .to_string()
}

/// lncrawl writes `<navLabel><text>…</text></navLabel><content src="…"/>`; match
/// that pair directly so nested volume navPoints cannot swallow a chapter.
fn retitle_ncx(ncx: &str, labels: &HashMap<&str, &str>, base: &str) -> String {
    static POINT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let point = POINT.get_or_init(|| {
        Regex::new(
            r#"(?is)(<navLabel>\s*<text>).*?(</text>\s*</navLabel>\s*<content\s+src\s*=\s*"([^"]+)")"#,
        )
        .expect("valid navPoint regex")
    });
    point
        .replace_all(ncx, |captures: &regex::Captures| {
            let whole = captures.get(0).map(|value| value.as_str()).unwrap_or("");
            let open = captures.get(1).map(|value| value.as_str()).unwrap_or("");
            let close = captures.get(2).map(|value| value.as_str()).unwrap_or("");
            let src = captures.get(3).map(|value| value.as_str()).unwrap_or("");
            let Some(label) = labels.get(join_archive(base, src).as_str()) else {
                return whole.to_string();
            };
            format!("{open}{}{close}", escape_html(label))
        })
        .to_string()
}

pub fn export_book_epub(
    epub_path: &Path,
    destination: &Path,
    fallback_title: &str,
) -> Result<ExportMetadata, String> {
    let audit = audit_epub(epub_path)?;
    let metadata = NovelMetadata {
        title: audit
            .title
            .clone()
            .unwrap_or_else(|| fallback_title.to_string()),
        author: audit.author.clone(),
        authors: audit.author.iter().cloned().collect(),
        synopsis: audit.description.clone(),
        tags: Vec::new(),
        status: None,
        language: audit.language.clone(),
        chapter_count: Some(audit.chapters.len() as u32),
        source_url: String::new(),
        source: "book".to_string(),
        cover: None,
        aliases: Vec::new(),
    };
    let plan = plan_audit(&audit, None, None, None);
    let verification = verification_from_plan(&plan, audit.has_cover);
    export_epub(
        epub_path,
        destination,
        &metadata,
        None,
        None,
        &verification,
        false,
    )
}

/// Adopt a loose EPUB into the structured library without touching it.
///
/// The EPUB is only read: chapter XHTML files, the cover and `metadata.json`
/// are written beside it. Calling this on an already-adopted folder returns the
/// stored metadata unchanged, so a double click or retry is idempotent.
pub fn adopt_epub(epub_path: &Path, folder: &Path) -> Result<ExportMetadata, String> {
    if folder.join("metadata.json").is_file() {
        if let Some(existing) = read_export_metadata(folder) {
            if existing.provenance.as_deref() == Some("migrated") {
                return Ok(existing);
            }
            return Err("This folder is already a structured NovelForge download.".to_string());
        }
        return Err(
            "This folder has an unreadable metadata.json; repair or move it aside first."
                .to_string(),
        );
    }
    if !epub_path.is_file() {
        return Err("The archive file no longer exists.".to_string());
    }
    if !epub_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
    {
        return Err("Only EPUB archives can be adopted into the structured library.".to_string());
    }
    let chapters = folder.join("chapters");
    if chapters.exists() {
        let has_files = std::fs::read_dir(&chapters)
            .map(|entries| entries.filter_map(Result::ok).next().is_some())
            .unwrap_or(false);
        if has_files {
            return Err(
                "A chapters folder already exists; move it aside before adopting.".to_string(),
            );
        }
    }

    let audit = audit_epub(epub_path)?;
    if audit.chapters.is_empty() {
        return Err("The EPUB contains no readable chapters.".to_string());
    }
    let plan = plan_audit(&audit, None, None, None);
    if !plan.ok {
        return Err(format!(
            "The EPUB did not pass chapter audit: {}",
            plan.detail
        ));
    }
    let verification = verification_from_plan(&plan, audit.has_cover);
    let source_name = epub_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "local EPUB".to_string());
    let metadata = NovelMetadata {
        title: audit
            .title
            .clone()
            .or_else(|| {
                epub_path
                    .file_stem()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "Adopted EPUB".to_string()),
        author: audit.author.clone(),
        authors: audit.author.iter().cloned().collect(),
        synopsis: audit.description.clone(),
        tags: Vec::new(),
        status: None,
        language: audit.language.clone(),
        chapter_count: Some(audit.chapters.len() as u32),
        source_url: String::new(),
        source: "migrated".to_string(),
        cover: None,
        aliases: Vec::new(),
    };

    let mut export = export_epub(
        epub_path,
        folder,
        &metadata,
        None,
        None,
        &verification,
        false,
    )?;
    export.provenance = Some("migrated".to_string());
    export.source = Some("migrated".to_string());
    export.index_detail = Some(format!(
        "Adopted from {source_name}; chapter order follows the EPUB spine."
    ));
    write_metadata(folder, &export)?;
    Ok(export)
}

fn write_metadata(destination: &Path, metadata: &ExportMetadata) -> Result<(), String> {
    let path = destination.join("metadata.json");
    let mut value = serde_json::to_value(metadata)
        .map_err(|error| format!("Cannot serialise metadata.json: {error}"))?;
    // The struct deliberately has no schema field: older readers ignore unknown
    // keys, and a future reader can use this to migrate the folder in place.
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "schemaVersion".to_string(),
            serde_json::json!(crate::model::SCHEMA_VERSION),
        );
    }
    let json = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Cannot serialise metadata.json: {error}"))?;
    crate::fsutil::write_text_atomic(&path, &json)
        .map_err(|error| format!("Cannot write metadata.json: {error}"))
}

/// NF-041: store what the crawl actually cost. Called only for webnovel runs
/// that wrote chapters, so the Library shows a measured pace or nothing at all,
/// never an estimate derived from the current network.
pub fn record_crawl_pace(
    destination: &Path,
    chapters: u32,
    crawl: std::time::Duration,
) -> Option<crate::model::CrawlPace> {
    let seconds = crawl.as_secs_f64();
    if chapters == 0 || !seconds.is_finite() || seconds < 1.0 {
        return None;
    }
    let mut metadata = read_export_metadata(destination)?;
    let pace = crate::model::CrawlPace {
        chapters,
        seconds: (seconds * 10.0).round() / 10.0,
        seconds_per_chapter: ((seconds / f64::from(chapters)) * 100.0).round() / 100.0,
        chapters_per_minute: ((f64::from(chapters) / seconds) * 6_000.0).round() / 100.0,
        measured_at: crate::settings::now_secs(),
    };
    metadata.crawl = Some(pace.clone());
    write_metadata(destination, &metadata).ok()?;
    Some(pace)
}

/// Write the two chapter-index files beside `metadata.json`:
///
/// - `index.json` — compact machine-readable list.
/// - `chapters.html` — self-contained searchable page for readers.
///
/// The full source index is preserved even when only part of it was
/// downloaded, so a partial folder can always explain what is missing.
fn write_index_files(destination: &Path, export: &ExportMetadata) -> Result<(), String> {
    let index = serde_json::json!({
        "title": export.title,
        "author": export.author,
        "sourceUrl": export.source_url,
        "source": export.source,
        "chapterCount": export.chapter_count,
        "downloadedChapters": export.downloaded_chapters,
        "indexComplete": export.index_complete,
        "indexDetail": export.index_detail,
        "generatedAt": export.generated_at,
        "chapters": export.chapters,
    });
    let mut value = serde_json::to_value(index)
        .map_err(|error| format!("Cannot serialise index.json: {error}"))?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "schemaVersion".to_string(),
            serde_json::json!(crate::model::SCHEMA_VERSION),
        );
    }
    let json = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Cannot serialise index.json: {error}"))?;
    crate::fsutil::write_text_atomic(&destination.join("index.json"), &json)
        .map_err(|error| format!("Cannot write index.json: {error}"))?;
    crate::fsutil::write_text_atomic(&destination.join("chapters.html"), &index_page(export))
        .map_err(|error| format!("Cannot write chapters.html: {error}"))
}

fn index_page(export: &ExportMetadata) -> String {
    let title = escape_html(&export.title);
    let author = export
        .author
        .as_deref()
        .map(escape_html)
        .unwrap_or_else(|| "Unknown author".to_string());
    let source = export
        .source_url
        .as_deref()
        .map(|url| format!(r#" · <a href="{href}">source</a>"#, href = escape_html(url)))
        .unwrap_or_default();
    let missing = export
        .chapter_count
        .saturating_sub(export.downloaded_chapters);
    let completeness = if export.index_complete {
        "complete source index"
    } else if export.chapters.is_empty() {
        "no source index"
    } else {
        "partial source index"
    };
    let index_detail = export
        .index_detail
        .as_deref()
        .map(|detail| {
            format!(
                r#"<p class="detail">{detail}</p>"#,
                detail = escape_html(detail)
            )
        })
        .unwrap_or_default();

    let mut rows = String::new();
    for chapter in &export.chapters {
        let number = chapter.number;
        let chapter_title = escape_html(&chapter.title);
        let volume = chapter
            .volume
            .map(|volume| format!(r#"<span class="volume">Vol. {volume}</span> "#))
            .unwrap_or_default();
        let body = if chapter.downloaded {
            let href = chapter.file.as_deref().map(encode_href).unwrap_or_default();
            let link = if href.is_empty() {
                format!("<span class=\"title\">{chapter_title}</span>")
            } else {
                format!(r#"<a class="title" href="{href}">{chapter_title}</a>"#)
            };
            format!(
                r#"<li class="downloaded" data-status="downloaded"><span class="num">{number}</span><span class="body">{volume}{link}</span><span class="badge">Downloaded</span></li>"#
            )
        } else {
            format!(
                r#"<li class="missing" data-status="missing"><span class="num">{number}</span><span class="body">{volume}<span class="title">{chapter_title}</span></span><span class="badge">Not downloaded</span></li>"#
            )
        };
        rows.push_str(&body);
    }
    if rows.is_empty() {
        rows.push_str(r#"<li class="empty">No chapters were listed for this source.</li>"#);
    }

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Chapter index</title>
<style>
:root {{ color-scheme: light dark; }}
* {{ box-sizing: border-box; }}
body {{ margin: 0 auto; max-width: 60rem; padding: 2rem 1.25rem 4rem; font: 16px/1.5 system-ui, sans-serif; }}
h1 {{ margin: 0 0 .35rem; font-size: 1.7rem; }}
p {{ margin: .25rem 0; }}
.meta, .detail {{ color: #6b7280; font-size: .95rem; }}
.controls {{ display: flex; flex-wrap: wrap; gap: .75rem; align-items: center; margin: 1.5rem 0 1rem; }}
input[type="search"] {{ flex: 1 1 18rem; padding: .55rem .75rem; border: 1px solid #9ca3af; border-radius: .5rem; font: inherit; background: transparent; color: inherit; }}
label {{ display: flex; gap: .4rem; align-items: center; white-space: nowrap; }}
ol {{ list-style: none; margin: 0; padding: 0; }}
li {{ display: grid; grid-template-columns: 3.5rem minmax(0, 1fr) auto; gap: .6rem; align-items: baseline; padding: .45rem .5rem; border-bottom: 1px solid #e5e7eb33; }}
li.hidden {{ display: none; }}
.num {{ text-align: right; font-variant-numeric: tabular-nums; color: #6b7280; }}
.body {{ min-width: 0; }}
.title {{ color: inherit; text-decoration: none; overflow-wrap: anywhere; }}
a.title:hover {{ text-decoration: underline; }}
.volume {{ color: #6b7280; font-size: .85rem; margin-right: .35rem; }}
.badge {{ justify-self: end; font-size: .72rem; text-transform: uppercase; letter-spacing: .04em; border-radius: 999px; padding: .15rem .5rem; white-space: nowrap; }}
@media (max-width: 34rem) {{
  li {{ grid-template-columns: 3rem minmax(0, 1fr); }}
  .badge {{ grid-column: 2; justify-self: start; }}
}}
.downloaded .badge {{ background: #16a34a22; color: #15803d; }}
.missing .badge {{ background: #f59e0b22; color: #b45309; }}
.missing .title {{ color: #6b7280; }}
.empty {{ color: #6b7280; }}
@media (prefers-color-scheme: dark) {{
  .downloaded .badge {{ background: #22c55e22; color: #4ade80; }}
  .missing .badge {{ background: #f59e0b22; color: #fbbf24; }}
  li {{ border-bottom-color: #ffffff1a; }}
}}
</style>
</head>
<body>
<h1>{title}</h1>
<p class="meta">by {author}{source}</p>
<p class="meta">{downloaded} of {total} chapters downloaded · {missing} missing · {completeness}</p>
{index_detail}
<div class="controls">
<input id="filter" type="search" placeholder="Filter by chapter number or title…" autocomplete="off">
<label><input id="missing" type="checkbox"> Missing only</label>
</div>
<ol id="chapters">
{rows}
</ol>
<script>
(function () {{
  var filter = document.getElementById('filter');
  var missing = document.getElementById('missing');
  var rows = Array.prototype.slice.call(document.querySelectorAll('#chapters li[data-status]'));
  function apply() {{
    var needle = filter.value.trim().toLowerCase();
    var onlyMissing = missing.checked;
    rows.forEach(function (row) {{
      var matchesText = !needle || row.textContent.toLowerCase().indexOf(needle) !== -1;
      var matchesStatus = !onlyMissing || row.getAttribute('data-status') === 'missing';
      row.classList.toggle('hidden', !(matchesText && matchesStatus));
    }});
  }}
  filter.addEventListener('input', apply);
  missing.addEventListener('change', apply);
}})();
</script>
</body>
</html>
"#,
        downloaded = export.downloaded_chapters,
        total = export.chapter_count.max(export.chapters.len() as u32),
    )
}

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

/// Percent-encode a relative file path for use in an `href`, keeping `/`.
fn encode_href(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    escape_html(&out)
}

/// Canonical identity for duplicate detection. It ignores fragments, tracking
/// query parameters, a leading `www.`, the `http`/`https` distinction and
/// trailing slashes, but keeps meaningful queries such as WordPress `?p=123`.
pub fn normalize_source_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let Ok(mut url) = Url::parse(trimmed) else {
        return trimmed.trim_end_matches('/').to_ascii_lowercase();
    };
    url.set_fragment(None);
    if url.scheme() == "http" {
        let _ = url.set_scheme("https");
    }
    if let Some(host) = url.host_str() {
        let host = host.to_ascii_lowercase();
        let host = host.strip_prefix("www.").unwrap_or(&host);
        if !host.is_empty() {
            let _ = url.set_host(Some(host));
        }
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });

    let mut pairs = url
        .query_pairs()
        .filter(|(key, _)| !is_tracking_query_key(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    pairs.sort();
    url.set_query(None);
    if !pairs.is_empty() {
        let mut query = url.query_pairs_mut();
        for (key, value) in &pairs {
            query.append_pair(key, value);
        }
    }
    url.to_string()
}

fn is_tracking_query_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("utm_")
        || matches!(
            key.as_str(),
            "fbclid" | "gclid" | "mc_cid" | "mc_eid" | "igshid" | "spm" | "ref" | "ref_src" | "_ga"
        )
}

pub fn read_export_metadata(folder: &Path) -> Option<ExportMetadata> {
    let raw = std::fs::read_to_string(folder.join("metadata.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn export_is_complete(metadata: &ExportMetadata) -> bool {
    metadata.complete
        || (metadata.verified.ok
            && (metadata.index_complete
                || metadata.epub.is_none()
                || metadata.source.as_deref() == Some("book")))
}

/// True for structured webnovel exports that can be retried/updated through
/// lncrawl. Book exports have `source = "book"` and no source URL; manga
/// exports have no EPUB.
pub fn is_webnovel_export(metadata: &ExportMetadata) -> bool {
    metadata.epub.is_some()
        && metadata.source.as_deref() != Some("book")
        && metadata.source_url.as_deref().is_some_and(|url| {
            Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
        })
}

/// Merge the chapters of a previous export into this run. Only runs when both
/// manifests describe the same source. Returns the number of preserved or
/// adopted chapters.
pub fn merge_previous_export(destination: &Path, export: &mut ExportMetadata) -> usize {
    let Some(previous) = read_export_metadata(destination) else {
        return 0;
    };
    let same_source = match (previous.source_url.as_deref(), export.source_url.as_deref()) {
        (Some(left), Some(right)) => normalize_source_url(left) == normalize_source_url(right),
        _ => false,
    };
    if !same_source {
        return 0;
    }

    let mut preserved = 0;
    let new_numbers = export
        .chapters
        .iter()
        .map(|chapter| chapter.number)
        .collect::<HashSet<_>>();
    // Normalise every title once. The fallback below runs for every preserved
    // chapter, so normalising inside the scan made a full-library update
    // quadratic in regex compiles (NF-038).
    let normalized_titles = export
        .chapters
        .iter()
        .map(|chapter| normalize_title(&chapter.title))
        .collect::<Vec<_>>();

    for old in &previous.chapters {
        if !old.downloaded {
            continue;
        }
        let Some(file) = old.file.as_deref() else {
            continue;
        };
        if file.contains("..") || !destination.join(file).is_file() {
            continue;
        }
        let by_number = export
            .chapters
            .iter()
            .position(|chapter| chapter.number == old.number);
        let position = by_number.or_else(|| {
            let title = normalize_title(&old.title);
            (0..normalized_titles.len()).find(|&offset| {
                !export.chapters[offset].downloaded && normalized_titles[offset] == title
            })
        });
        if let Some(index) = position {
            let chapter = &mut export.chapters[index];
            if !chapter.downloaded {
                chapter.downloaded = true;
                chapter.file = Some(file.to_string());
                preserved += 1;
            }
        } else {
            export.chapters.push(old.clone());
            preserved += 1;
        }
    }

    // A re-download can change a chapter's filename (older builds used the
    // EPUB position). Remove the old file once nothing references it.
    let referenced = export
        .chapters
        .iter()
        .filter_map(|chapter| chapter.file.as_deref())
        .collect::<HashSet<_>>();
    for old in &previous.chapters {
        let Some(file) = old.file.as_deref() else {
            continue;
        };
        if referenced.contains(file) || !new_numbers.contains(&old.number) || file.contains("..") {
            continue;
        }
        let path = destination.join(file);
        if path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }

    export.chapters.sort_by_key(|chapter| chapter.number);
    export.downloaded_chapters = export
        .chapters
        .iter()
        .filter(|chapter| chapter.downloaded)
        .count() as u32;
    export.chapter_count = export.chapter_count.max(export.chapters.len() as u32);

    if export.cover.is_none()
        && let Some(cover) = previous.cover.as_deref()
        && destination.join(cover).is_file()
    {
        export.cover = Some(cover.to_string());
    }

    preserved
}

pub fn verification_from_plan(plan: &AuditPlan, has_cover: bool) -> ExportVerification {
    ExportVerification {
        ok: plan.ok,
        chapters_expected: plan.expected,
        chapters_found: plan.found,
        empty_chapters: plan.empty_positions.clone(),
        missing_chapters: plan.missing_serials.clone(),
        has_cover,
        detail: plan.detail.clone(),
    }
}

fn write_cover(
    destination: &Path,
    epub_cover: Option<&[u8]>,
    metadata_cover: Option<&str>,
) -> Option<String> {
    let bytes = epub_cover
        .map(|value| value.to_vec())
        .or_else(|| metadata_cover.and_then(decode_data_url))?;
    let extension = image_extension(&bytes)?;
    let name = format!("cover.{extension}");
    std::fs::write(destination.join(&name), &bytes).ok()?;
    Some(name)
}

fn decode_data_url(value: &str) -> Option<Vec<u8>> {
    let (_, payload) = value.split_once(";base64,")?;
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
        Some("png")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Manga / CBZ
// ---------------------------------------------------------------------------

/// Packs the image tree that gallery-dl downloads into one CBZ/ZIP with one
/// folder per chapter. gallery-dl owns extraction; NovelForge owns the
/// container shape that `audit_cbz` and `export_cbz` already understand.
///
/// `source_root` is normally `<work dir>/gallery-dl`. Images directly under it
/// become “Chapter 1”; nested folders keep their name and natural order.
pub fn package_manga_directory(
    source_root: &Path,
    destination: &Path,
    format: &str,
    title: &str,
) -> Result<PathBuf, String> {
    if !source_root.is_dir() {
        return Err(format!(
            "gallery-dl left no download tree at {}",
            source_root.display()
        ));
    }

    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in WalkDir::new(source_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.into_path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !is_image(name) {
            continue;
        }
        let parent = path.parent().unwrap_or(source_root);
        let key = parent
            .strip_prefix(source_root)
            .ok()
            .map(|value| {
                value
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        groups.entry(key).or_default().push(path);
    }

    if groups.is_empty() {
        return Err("gallery-dl produced no images for this URL".to_string());
    }

    let mut keys = groups.keys().cloned().collect::<Vec<_>>();
    keys.sort_by_key(|left| natural_key(left));

    // A chapter is often uploaded by several scan groups. gallery-dl extracts
    // every version that passes `--child-filter`; keep the richest one per
    // (language, volume, chapter) so the selection still means one folder per
    // chapter.
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut chosen: Vec<String> = Vec::new();
    for key in keys {
        let identity =
            gallery_chapter_identity(source_root, &key).unwrap_or_else(|| key.to_ascii_lowercase());
        let pages = groups.get(&key).map(Vec::len).unwrap_or(0);
        match seen.get(&identity).copied() {
            Some(position) => {
                let current = groups.get(&chosen[position]).map(Vec::len).unwrap_or(0);
                if pages > current {
                    chosen[position] = key;
                }
            }
            None => {
                seen.insert(identity, chosen.len());
                chosen.push(key);
            }
        }
    }
    chosen.sort_by_key(|left| natural_key(left));

    let extension = if format.eq_ignore_ascii_case("zip") {
        "zip"
    } else {
        "cbz"
    };
    let stem = sanitize_filename(title, 80);
    let stem = if stem.is_empty() {
        "manga".to_string()
    } else {
        stem
    };
    let archive_path = destination.join(format!("{stem}.{extension}"));
    let file = File::create(&archive_path)
        .map_err(|error| format!("Cannot create {}: {error}", archive_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = if extension == "zip" {
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
    } else {
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
    };

    let mut used_folders = HashSet::new();
    for (index, key) in chosen.iter().enumerate() {
        let mut images = groups.remove(key).unwrap_or_default();
        images.sort_by(|left, right| {
            natural_key(&left.file_name().unwrap_or_default().to_string_lossy()).cmp(&natural_key(
                &right.file_name().unwrap_or_default().to_string_lossy(),
            ))
        });
        if images.is_empty() {
            continue;
        }

        let label = key
            .rsplit('/')
            .find(|part| !part.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Chapter {}", index + 1));
        let mut folder = sanitize_filename(&label, 80);
        if folder.is_empty() {
            folder = format!("Chapter {}", index + 1);
        }
        let mut unique = folder.clone();
        let mut suffix = 2;
        while !used_folders.insert(unique.clone()) {
            unique = format!("{folder} ({suffix})");
            suffix += 1;
        }
        let folder_name = format!("{:04} - {unique}", index + 1);

        for (page, path) in images.iter().enumerate() {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
            if bytes.is_empty() {
                continue;
            }
            let page_extension = image_extension(&bytes).unwrap_or_else(|| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("jpg")
            });
            zip.start_file(
                format!("{folder_name}/{:03}.{page_extension}", page + 1),
                options,
            )
            .map_err(|error| format!("Cannot add manga page to archive: {error}"))?;
            zip.write_all(&bytes)
                .map_err(|error| format!("Cannot write manga page: {error}"))?;
        }
    }

    zip.finish()
        .map_err(|error| format!("Cannot finish manga archive: {error}"))?;
    Ok(archive_path)
}

/// Reads the `info.json` that gallery-dl writes next to each chapter and
/// returns `lang|volume|chapter`. Alternate uploads of one chapter share this
/// identity even when their folder titles differ.
fn gallery_chapter_identity(root: &Path, key: &str) -> Option<String> {
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join(key).join("info.json")).ok()?).ok()?;
    let chapter = json_scalar(data.get("chapter"))?;
    let volume = json_scalar(data.get("volume")).unwrap_or_default();
    let language = data
        .get("lang")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Some(format!("{language}|{volume}|{chapter}"))
}

fn json_scalar(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
        _ => None,
    }
}

pub fn audit_cbz(path: &Path) -> Result<CbzAudit, String> {
    let mut archive = open_archive(path)?;
    let mut names = archive_names(&archive);
    names.sort();

    let mut grouped: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();
    for name in names {
        if !is_image(&name) || name.starts_with("__MACOSX") {
            continue;
        }
        let bytes = read_entry(&mut archive, &name).unwrap_or_default();
        if bytes.is_empty() {
            continue;
        }
        let group = chapter_group(&name);
        grouped.entry(group).or_default().push((name, bytes));
    }

    let mut group_names = grouped.keys().cloned().collect::<Vec<_>>();
    group_names.sort_by_key(|left| natural_key(left));

    let mut chapters = Vec::new();
    let mut image_count = 0;
    let mut cover_name = None;
    let mut cover_bytes = None;
    for (index, group) in group_names.into_iter().enumerate() {
        let images = grouped.remove(&group).unwrap_or_default();
        image_count += images.len();
        if cover_bytes.is_none()
            && let Some((name, bytes)) = images.first()
        {
            cover_name = Some(name.clone());
            cover_bytes = Some(bytes.clone());
        }
        chapters.push(CbzChapter {
            position: (index + 1) as u32,
            title: if group.is_empty() {
                format!("Chapter {}", index + 1)
            } else {
                group
            },
            images,
        });
    }

    Ok(CbzAudit {
        chapters,
        image_count,
        cover_name,
        cover_bytes,
    })
}

pub fn export_cbz(
    cbz_path: &Path,
    destination: &Path,
    metadata: &NovelMetadata,
    selection: Option<&str>,
    verification: &ExportVerification,
) -> Result<ExportMetadata, String> {
    let audit = audit_cbz(cbz_path)?;
    let chapters_dir = destination.join("chapters");
    std::fs::create_dir_all(&chapters_dir)
        .map_err(|error| format!("Cannot create chapters folder: {error}"))?;

    let mut chapters = Vec::new();
    for chapter in &audit.chapters {
        // `package_manga_directory` already prefixes archive folders with
        // `0001 - `; keep that ordering in the archive but not in the
        // chapter label written beside it.
        let label = strip_archive_position(&chapter.title);
        let folder_name = sanitize_filename(label, 80);
        let folder_rel = format!("chapters/{:04} - {folder_name}", chapter.position);
        let folder = destination.join(&folder_rel);
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("Cannot create manga chapter folder: {error}"))?;
        for (index, (_, bytes)) in chapter.images.iter().enumerate() {
            let extension = image_extension(bytes).unwrap_or("jpg");
            let file = folder.join(format!("{:03}.{extension}", index + 1));
            std::fs::write(&file, bytes)
                .map_err(|error| format!("Cannot write manga page: {error}"))?;
        }
        chapters.push(NovelChapter {
            number: chapter.position,
            title: label.to_string(),
            url: metadata.source_url.clone(),
            volume: None,
            downloaded: true,
            file: Some(folder_rel),
        });
    }

    let cover = audit
        .cover_bytes
        .as_deref()
        .and_then(|bytes| {
            let extension = image_extension(bytes)?;
            let name = format!("cover.{extension}");
            std::fs::write(destination.join(&name), bytes).ok()?;
            Some(name)
        })
        .or_else(|| write_cover(destination, None, metadata.cover.as_deref()));

    let archive_name = cbz_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "manga.cbz".to_string());

    let expected = selection
        .and_then(parse_selection_serials)
        .map(|values| values.len() as u32);
    let export = ExportMetadata {
        title: non_empty(&metadata.title)
            .unwrap_or_else(|| fallback_title(cbz_path))
            .to_string(),
        author: metadata.author.clone(),
        authors: metadata.authors.clone(),
        synopsis: metadata.synopsis.clone(),
        tags: metadata.tags.clone(),
        status: metadata.status.clone(),
        language: metadata.language.clone(),
        source_url: non_empty(&metadata.source_url),
        source: non_empty(&metadata.source),
        cover,
        epub: None,
        chapter_count: expected
            .unwrap_or(audit.chapters.len() as u32)
            .max(audit.chapters.len() as u32),
        downloaded_chapters: audit.chapters.len() as u32,
        selection: selection.map(str::to_string),
        index_complete: false,
        index_detail: None,
        complete: verification.ok,
        provenance: None,
        crawl: None,
        chapters,
        generated_at: crate::settings::now_secs(),
        verified: verification.clone(),
    };
    write_metadata(destination, &export)?;
    write_index_files(destination, &export)?;
    let _ = archive_name;
    Ok(export)
}

// ---------------------------------------------------------------------------
// Verification report integration
// ---------------------------------------------------------------------------

pub fn verify_report(
    path: &Path,
    report: &mut crate::model::VerificationReport,
    expected: Option<u32>,
) {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("epub"))
    {
        if let Ok(audit) = audit_epub(path) {
            report.has_cover = audit.has_cover;
            report.chapters_found = audit.chapters.len() as u32;
            let plan = plan_audit(&audit, None, expected, None);
            report.chapters_expected = plan.expected;
            report.empty_chapters = plan.empty_positions.clone();
            report.missing_chapters = plan.missing_serials.clone();
            let cover = if audit.has_cover {
                "cover present"
            } else {
                "cover missing"
            };
            report.detail = format!(
                "{} · {} chapter file(s) · {cover}",
                report.detail,
                audit.chapters.len()
            );
        }
    } else if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "cbz" | "zip"))
        && let Ok(audit) = audit_cbz(path)
    {
        report.has_cover = audit.cover_bytes.is_some();
        report.chapters_found = audit.chapters.len() as u32;
        report.chapters_expected = expected;
        report.detail = format!(
            "{} · {} chapter folder(s) · {} page(s)",
            report.detail,
            audit.chapters.len(),
            audit.image_count
        );
    }
}

// ---------------------------------------------------------------------------
// EPUB internals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ManifestItem {
    id: String,
    href: String,
    properties: String,
}

#[derive(Debug, Clone, Default)]
struct Package {
    title: Option<String>,
    author: Option<String>,
    description: Option<String>,
    language: Option<String>,
    manifest: Vec<ManifestItem>,
    spine_ids: Vec<String>,
    cover_id: Option<String>,
}

impl Package {
    fn spine_items(&self) -> Vec<ManifestItem> {
        self.spine_ids
            .iter()
            .filter_map(|id| self.manifest.iter().find(|item| &item.id == id).cloned())
            .filter(|item| is_xhtml(&item.href))
            .collect()
    }

    fn cover_candidates(&self) -> Vec<String> {
        let mut values = Vec::new();
        if let Some(id) = &self.cover_id
            && let Some(item) = self.manifest.iter().find(|item| &item.id == id)
        {
            values.push(item.href.clone());
        }
        for item in &self.manifest {
            if item.properties.contains("cover-image") {
                values.push(item.href.clone());
            }
        }
        for item in &self.manifest {
            let lower = item.href.to_ascii_lowercase();
            if is_image(&lower) && lower.contains("cover") {
                values.push(item.href.clone());
            }
        }
        values.sort();
        values.dedup();
        values
    }
}

fn parse_container_opf(xml: &str) -> Option<String> {
    let document = roxmltree::Document::parse(xml).ok()?;
    document
        .descendants()
        .find(|node| node.has_tag_name("rootfile"))
        .and_then(|node| node.attribute("full-path"))
        .map(str::to_string)
}

fn parse_package(xml: &str) -> Result<Package, String> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| format!("Invalid EPUB package document: {error}"))?;
    let mut package = Package::default();

    for node in document.descendants() {
        match node.tag_name().name() {
            "title" => {
                if package.title.is_none() {
                    package.title = child_text(node);
                }
            }
            "creator" => {
                if package.author.is_none() {
                    package.author = child_text(node);
                }
            }
            "description" => {
                if package.description.is_none() {
                    package.description = child_text(node);
                }
            }
            "language" => {
                if package.language.is_none() {
                    package.language = child_text(node);
                }
            }
            "item" => {
                if let (Some(id), Some(href)) = (node.attribute("id"), node.attribute("href")) {
                    package.manifest.push(ManifestItem {
                        id: id.to_string(),
                        href: href.to_string(),
                        properties: node.attribute("properties").unwrap_or_default().to_string(),
                    });
                }
            }
            "itemref" => {
                if let Some(idref) = node.attribute("idref") {
                    package.spine_ids.push(idref.to_string());
                }
            }
            "meta" if node.attribute("name") == Some("cover") => {
                package.cover_id = node.attribute("content").map(str::to_string);
            }
            _ => {}
        }
    }
    Ok(package)
}

fn child_text(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let text = node
        .descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() { None } else { Some(text) }
}

fn is_chapter_entry(href: &str, _html: &str, text_chars: usize) -> bool {
    let lower = href.to_ascii_lowercase();
    if is_non_content(&lower) {
        return false;
    }
    lower.contains("chapter") || text_chars >= MIN_CHAPTER_CHARS
}

fn is_non_content(href: &str) -> bool {
    [
        "cover",
        "nav",
        "toc",
        "front",
        "intro",
        "volume_",
        "titlepage",
        "title-page",
        "copyright",
        "acknowledg",
    ]
    .iter()
    .any(|marker| href.contains(marker))
}

fn visible_text_len(html: &str) -> usize {
    static NOISE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let noise = NOISE.get_or_init(|| {
        Regex::new(
            r"(?is)<script\b[^>]*>.*?</script>|<style\b[^>]*>.*?</style>|<head\b[^>]*>.*?</head>",
        )
        .expect("valid noise regex")
    });
    let cleaned = noise.replace_all(html, " ");
    let document = Html::parse_document(&cleaned);
    let selector = Selector::parse("body").expect("valid body selector");
    let text = document
        .select(&selector)
        .next()
        .map(|body| body.text().collect::<String>())
        .unwrap_or_default();
    text.split_whitespace().map(str::len).sum()
}

fn chapter_title(html: &str, archive_path: &str) -> String {
    let document = Html::parse_document(html);
    for selector in ["h1", "h2", "title"] {
        if let Ok(selector) = Selector::parse(selector)
            && let Some(element) = document.select(&selector).next()
        {
            let text = element.text().collect::<String>();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() && text.len() < 200 {
                return text;
            }
        }
    }
    archive_path
        .rsplit('/')
        .next()
        .unwrap_or("Chapter")
        .trim_end_matches(".xhtml")
        .trim_end_matches(".html")
        .to_string()
}

fn open_archive(path: &Path) -> Result<ZipArchive<BufReader<File>>, String> {
    let file = File::open(path).map_err(|error| format!("Cannot open archive: {error}"))?;
    ZipArchive::new(BufReader::new(file)).map_err(|error| format!("Invalid ZIP archive: {error}"))
}

fn archive_names(archive: &ZipArchive<BufReader<File>>) -> Vec<String> {
    archive
        .file_names()
        .filter_map(Result::ok)
        .map(|name| name.into_owned())
        .collect()
}

fn read_entry(archive: &mut ZipArchive<BufReader<File>>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Normalise an archive entry name into a relative path, rejecting anything a
/// ZIP-slip payload would need (absolute paths, drive prefixes, `..`).
pub(crate) fn safe_entry_path(name: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in std::path::Path::new(name).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn join_archive(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href);
    if base.is_empty() {
        return normalize_archive_path(href);
    }
    normalize_archive_path(&format!("{base}/{href}"))
}

fn normalize_archive_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    parts.join("/")
}

fn is_xhtml(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm")
}

fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn is_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".css", ".jpg", ".jpeg", ".png", ".webp", ".gif", ".svg", ".ttf", ".otf", ".woff", ".woff2",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn chapter_group(name: &str) -> String {
    let mut parts = name.split('/');
    let first = parts.next().unwrap_or_default();
    if parts.next().is_some() && !is_image(first) {
        first.to_string()
    } else {
        "Chapter 1".to_string()
    }
}

/// Drops the `0001 - ` archive prefix that `package_manga_directory` adds.
fn strip_archive_position(title: &str) -> &str {
    let trimmed = title.trim_start_matches(|value: char| value.is_ascii_digit());
    trimmed.strip_prefix(" - ").unwrap_or(title)
}

fn natural_key(value: &str) -> String {
    static DIGITS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let regex = DIGITS.get_or_init(|| Regex::new(r"(\d+)").expect("valid digit regex"));
    let mut key = String::new();
    let mut last = 0;
    for captures in regex.captures_iter(value) {
        let Some(matched) = captures.get(1) else {
            continue;
        };
        key.push_str(&value[last..matched.start()].to_ascii_lowercase());
        key.push_str(&format!("{:0>10}", matched.as_str()));
        last = matched.end();
    }
    key.push_str(&value[last..].to_ascii_lowercase());
    key
}

fn sanitize_filename(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for character in value.chars() {
        let allowed = character.is_alphanumeric()
            || matches!(character, '-' | '_' | '(' | ')' | '.' | ',' | '\'' | '’');
        if allowed {
            out.push(character);
            last_space = false;
        } else if character.is_whitespace() && !last_space {
            out.push(' ');
            last_space = true;
        }
        if out.chars().count() >= max_chars {
            break;
        }
    }
    let out = out.trim().trim_matches('.').trim().to_string();
    if out.is_empty() {
        "chapter".to_string()
    } else {
        out
    }
}

pub(crate) fn chapter_file_name(position: u32, title: &str) -> String {
    let safe = sanitize_filename(title, 90);
    format!("{position:04} - {safe}.xhtml")
}

fn first_non_empty<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> &'a str {
    values
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("")
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn fallback_title(path: &Path) -> String {
    path.file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "Download".to_string())
}

fn normalize_title(value: &str) -> String {
    // Called once per chapter per comparison in the title-matching passes. A
    // full-length novel reaches 5.4 M calls, so the regex must be compiled once
    // rather than per call (NF-038: the compile alone cost minutes of export).
    static PREFIX: OnceLock<Regex> = OnceLock::new();
    let prefix = PREFIX.get_or_init(|| {
        Regex::new(r"(?i)\bchapter\s*#?\s*\d+\s*[-:–—]?\s*").expect("valid title prefix regex")
    });
    let lower = value.to_lowercase();
    let stripped = prefix.replace(&lower, "");
    let normalized = stripped
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect::<String>();
    if normalized.is_empty() {
        // Bare titles such as "Chapter 1" must stay distinct; an empty key
        // would make every chapter match the first one.
        lower
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect()
    } else {
        normalized
    }
}

fn chapter_number_from_title(value: &str) -> Option<u32> {
    static CHAPTER: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    CHAPTER
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:chapter|ch\.?)\s*#?\s*(\d+)").expect("valid chapter regex")
        })
        .captures(value)
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
}

pub fn parse_selection_serials(spec: &str) -> Option<Vec<u32>> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return None;
    }
    let mut values = Vec::new();
    for token in spec.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((start, end)) = token.split_once('-') {
            let start = start.trim().parse::<u32>().ok()?;
            let end = end.trim().parse::<u32>().ok()?;
            let (low, high) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            if high - low > 50_000 {
                return None;
            }
            values.extend(low..=high);
        } else {
            values.push(token.parse::<u32>().ok()?);
        }
    }
    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Read a structured folder for the Home shelf / Library page.
/// Rebuilds the packed archive from the expanded chapter tree beside it.
///
/// `export_cbz` writes both the archive and `chapters/<NNNN - label>/`. If the
/// archive is gone afterwards — an over-eager antivirus, a sync tool, or a
/// half-copied folder — the download would ship as loose images, which is not
/// what the user asked for. The page tree holds the same bytes, so packing it
/// again restores the deliverable without touching the source.
pub fn repack_manga_chapters(
    destination: &Path,
    format: &str,
    title: &str,
) -> Result<PathBuf, String> {
    let chapters_dir = destination.join("chapters");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&chapters_dir)
        .map_err(|error| format!("Cannot read {}: {error}", chapters_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders
        .sort_by_key(|path| natural_key(&path.file_name().unwrap_or_default().to_string_lossy()));
    if folders.is_empty() {
        return Err("The chapter folder is empty, so no archive could be rebuilt".to_string());
    }

    let extension = if format.eq_ignore_ascii_case("zip") {
        "zip"
    } else {
        "cbz"
    };
    let stem = sanitize_filename(title, 80);
    let stem = if stem.is_empty() {
        "manga".to_string()
    } else {
        stem
    };
    let archive_path = destination.join(format!("{stem}.{extension}"));
    let file = File::create(&archive_path)
        .map_err(|error| format!("Cannot create {}: {error}", archive_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = if extension == "zip" {
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
    } else {
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
    };

    for folder in &folders {
        let name = folder
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "Chapter".to_string());
        let mut pages: Vec<PathBuf> = std::fs::read_dir(folder)
            .map_err(|error| format!("Cannot read {}: {error}", folder.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(is_image)
            })
            .collect();
        pages.sort_by_key(|path| {
            natural_key(&path.file_name().unwrap_or_default().to_string_lossy())
        });
        if pages.is_empty() {
            continue;
        }
        for page in &pages {
            let page_name = page
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default();
            let bytes = std::fs::read(page)
                .map_err(|error| format!("Cannot read {}: {error}", page.display()))?;
            zip.start_file(format!("{name}/{page_name}"), options)
                .map_err(|error| format!("Cannot add manga page to archive: {error}"))?;
            zip.write_all(&bytes)
                .map_err(|error| format!("Cannot write manga page: {error}"))?;
        }
    }

    zip.finish()
        .map_err(|error| format!("Cannot finish manga archive: {error}"))?;
    Ok(archive_path)
}

/// The packed archive that lives directly inside a structured folder.
///
/// Novels use `metadata.epub`; manga has no EPUB, so the export records
/// `epub: None` and the packed CBZ/ZIP is the only reader-ready file. Pick the
/// archive named after the title when there are several, otherwise the largest
/// one (packed archives are orders of magnitude bigger than a stray download).
fn library_archive(folder: &Path, metadata: &ExportMetadata) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    matches!(extension.to_ascii_lowercase().as_str(), "cbz" | "zip")
                })
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let wanted = metadata.title.trim();
    candidates.sort_by_key(|path| {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let size = std::fs::metadata(path)
            .map(|value| value.len())
            .unwrap_or_default();
        (stem != wanted, u64::MAX - size)
    });
    candidates.into_iter().next()
}

pub fn library_novel(folder: &Path) -> Option<crate::model::LibraryNovel> {
    let metadata = read_export_metadata(folder)?;
    // Old structured folders predate the chapter index files. Backfill them
    // best-effort the next time the library scans, so every structured folder
    // gains the readable index without a re-download.
    if !folder.join("index.json").is_file() || !folder.join("chapters.html").is_file() {
        let _ = write_index_files(folder, &metadata);
    }
    let cover = metadata.cover.as_ref().and_then(|name| {
        let path = folder.join(name);
        let bytes = std::fs::read(path).ok()?;
        metadata::data_url_for(&bytes)
    });
    let complete = export_is_complete(&metadata);
    let retryable = is_webnovel_export(&metadata);
    let modified = std::fs::metadata(folder.join("metadata.json"))
        .ok()
        .and_then(|value| value.modified().ok())
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or_default();
    let epub_path = metadata.epub.as_ref().and_then(|name| {
        let path = folder.join(name);
        path.is_file().then(|| path.to_string_lossy().to_string())
    });
    // Manga folders hold the packed CBZ/ZIP beside the expanded chapter tree.
    // Surfacing it is what makes "download a comic" land as an archive: the
    // folder alone reads as a pile of images.
    let archive_path =
        library_archive(folder, &metadata).map(|path| path.to_string_lossy().to_string());
    let source_host = metadata
        .source_url
        .as_deref()
        .and_then(|value| Url::parse(value).ok())
        .and_then(|url| url.host_str().map(str::to_string));
    let source_rank = source_host
        .as_deref()
        .map(crate::sources::rank_for_host)
        .unwrap_or(u8::MAX);
    Some(crate::model::LibraryNovel {
        folder: folder.to_string_lossy().to_string(),
        title: metadata.title,
        author: metadata.author,
        synopsis: metadata.synopsis,
        tags: metadata.tags,
        cover,
        chapter_count: metadata.chapter_count,
        downloaded_chapters: metadata.downloaded_chapters,
        complete,
        source_url: metadata.source_url,
        modified,
        epub_path,
        archive_path,
        retryable,
        provenance: metadata.provenance,
        disk_bytes: crate::storage::directory_bytes(folder),
        source_host,
        source_rank,
        crawl: metadata.crawl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn fixture(path: &Path, chapter_body: &str, chapters: usize) {
        fixture_markup(
            path,
            &(1..=chapters)
                .map(|index| format!(r#"<h1>Chapter {index}</h1><p>{chapter_body}</p>"#))
                .collect::<Vec<_>>(),
        );
    }

    /// One-chapter EPUB whose body markup is supplied verbatim. Used by the
    /// chapter-cleaning regression tests.
    fn fixture_markup(path: &Path, bodies: &[String]) {
        let file = File::create(path).expect("create fixture");
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(br#"<container><rootfiles><rootfile full-path="EPUB/content.opf"/></rootfiles></container>"#).unwrap();
        let mut manifest = String::new();
        let mut spine = String::new();
        for (offset, body) in bodies.iter().enumerate() {
            let index = offset + 1;
            manifest.push_str(&format!(
                r#"<item id="c{index}" href="chapter_{index:05}.xhtml" media-type="application/xhtml+xml"/>"#
            ));
            spine.push_str(&format!(r#"<itemref idref="c{index}"/>"#));
            zip.start_file(format!("EPUB/chapter_{index:05}.xhtml"), deflated)
                .unwrap();
            zip.write_all(format!(r#"<html><body>{body}</body></html>"#).as_bytes())
                .unwrap();
        }
        zip.start_file("EPUB/content.opf", deflated).unwrap();
        zip.write_all(format!(
            r#"<package><metadata><title>Fixture</title><creator>Author</creator></metadata><manifest><item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>{manifest}</manifest><spine>{spine}</spine></package>"#
        ).as_bytes()).unwrap();
        zip.start_file("EPUB/cover.jpg", deflated).unwrap();
        zip.write_all(&[0xff, 0xd8, 0xff, 0x00]).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn audits_and_exports_split_chapters() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-export-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture(&epub, &"word ".repeat(80), 3);

        let audit = audit_epub(&epub).expect("audit");
        assert_eq!(audit.chapters.len(), 3);
        assert!(audit.has_cover);
        assert_eq!(audit.title.as_deref(), Some("Fixture"));

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 3,
            ..Default::default()
        };
        let export =
            export_epub(&epub, &destination, &metadata, None, None, &report, false).unwrap();
        assert_eq!(export.downloaded_chapters, 3);
        assert!(destination.join("chapters").is_dir());
        assert_eq!(
            std::fs::read_dir(destination.join("chapters"))
                .unwrap()
                .count(),
            3
        );
        assert!(destination.join("metadata.json").is_file());
        assert!(destination.join("index.json").is_file());
        assert!(destination.join("chapters.html").is_file());
        let index: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(destination.join("index.json")).unwrap())
                .unwrap();
        assert_eq!(index["chapters"].as_array().unwrap().len(), 3);
        assert_eq!(index["downloadedChapters"], 3);
        assert!(
            index["chapters"]
                .as_array()
                .unwrap()
                .iter()
                .all(|chapter| chapter["downloaded"] == true)
        );
        let page = std::fs::read_to_string(destination.join("chapters.html")).unwrap();
        assert!(page.contains("3 of 3 chapters downloaded"));
        assert!(page.contains("Downloaded"));
        assert!(page.contains("chapters/0001"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn library_novel_backfills_missing_index_files() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-index-backfill-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture(&epub, &"word ".repeat(80), 2);
        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Old folder".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 2,
            ..Default::default()
        };
        export_epub(&epub, &destination, &metadata, None, None, &report, false).unwrap();
        std::fs::remove_file(destination.join("index.json")).unwrap();
        std::fs::remove_file(destination.join("chapters.html")).unwrap();

        assert!(library_novel(&destination).is_some());
        assert!(destination.join("index.json").is_file());
        assert!(destination.join("chapters.html").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chapter_index_marks_downloaded_and_missing_chapters() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-index-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture(&epub, &"word ".repeat(80), 2);

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture & <Index>".to_string(),
            source_url: "https://example.com/book?a=1&b=2".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 2,
            ..Default::default()
        };
        let index = NovelChapterIndex {
            source_url: metadata.source_url.clone(),
            source: "example.com".to_string(),
            title: Some(metadata.title.clone()),
            total: 3,
            complete: true,
            detail: None,
            chapters: vec![
                NovelChapter {
                    number: 1,
                    title: "Chapter 1".to_string(),
                    url: "https://example.com/1".to_string(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
                NovelChapter {
                    number: 2,
                    title: "Chapter 2 & more".to_string(),
                    url: "https://example.com/2".to_string(),
                    volume: Some(1),
                    downloaded: false,
                    file: None,
                },
                NovelChapter {
                    number: 3,
                    title: "Chapter 3".to_string(),
                    url: "https://example.com/3".to_string(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
            ],
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        };
        let export = export_epub(
            &epub,
            &destination,
            &metadata,
            Some(&index),
            None,
            &report,
            true,
        )
        .unwrap();
        assert_eq!(export.chapters.len(), 3);
        assert_eq!(export.downloaded_chapters, 2);

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(destination.join("index.json")).unwrap())
                .unwrap();
        let chapters = written["chapters"].as_array().unwrap();
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0]["downloaded"], true);
        assert!(
            chapters[0]["file"]
                .as_str()
                .unwrap()
                .starts_with("chapters/")
        );
        assert_eq!(chapters[2]["downloaded"], false);
        assert!(chapters[2]["file"].is_null());

        let page = std::fs::read_to_string(destination.join("chapters.html")).unwrap();
        assert!(page.contains("2 of 3 chapters downloaded"));
        assert!(page.contains("1 missing"));
        assert!(page.contains("Not downloaded"));
        assert!(page.contains("Chapter 2 &amp; more"));
        assert!(page.contains("https://example.com/book?a=1&amp;b=2"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn export_cleans_duplicate_titles_and_site_chrome() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-clean-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture_markup(
            &epub,
            &[r#"<h4 aria-hidden="true">#1</h4><h1>Chapter 1</h1>
<div class="chapter-nav"><a href="/prev">Previous</a><a href="/next">Next</a></div>
<p><strong>Chapter 1 - 1: The Heart Of A Demon</strong></p>
<p>The first real story sentence, long enough to count as a chapter.</p>
<p>The second real story sentence, also part of the story.</p>"#
                .to_string()],
        );

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture".to_string(),
            source: "novelfire.net".to_string(),
            ..Default::default()
        };
        let index = NovelChapterIndex {
            source_url: String::new(),
            source: "novelfire.net".to_string(),
            title: Some("Fixture".to_string()),
            total: 1,
            complete: true,
            detail: None,
            chapters: vec![NovelChapter {
                number: 1,
                title: "Chapter 1".to_string(),
                url: String::new(),
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
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 1,
            ..Default::default()
        };
        export_epub(
            &epub,
            &destination,
            &metadata,
            Some(&index),
            None,
            &report,
            true,
        )
        .unwrap();

        let chapter = std::fs::read_to_string(
            destination.join("chapters").join(
                std::fs::read_dir(destination.join("chapters"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .file_name(),
            ),
        )
        .unwrap();
        assert!(!chapter.contains("aria-hidden"));
        assert!(!chapter.contains("chapter-nav"));
        assert!(!chapter.contains("<strong>Chapter 1 - 1: The Heart Of A Demon</strong>"));
        assert!(chapter.contains("<h1>Chapter 1</h1>"));
        assert!(chapter.contains("The first real story sentence"));
        assert!(chapter.contains("The second real story sentence"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn adopted_epub_preserves_original_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-adopt-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let folder = root.join("loose-folder");
        std::fs::create_dir_all(&folder).unwrap();
        let epub = folder.join("Old Book.epub");
        fixture(&epub, &"word ".repeat(80), 2);
        let original = std::fs::read(&epub).unwrap();

        let export = adopt_epub(&epub, &folder).expect("adopt");
        assert_eq!(export.provenance.as_deref(), Some("migrated"));
        assert_eq!(export.source.as_deref(), Some("migrated"));
        assert_eq!(export.source_url, None);
        assert_eq!(export.downloaded_chapters, 2);
        assert!(export.complete);
        assert!(folder.join("metadata.json").is_file());
        assert_eq!(
            std::fs::read_dir(folder.join("chapters")).unwrap().count(),
            2
        );
        assert!(folder.join("cover.jpg").is_file());
        // The loose EPUB is the source of truth and stays byte-identical.
        assert_eq!(std::fs::read(&epub).unwrap(), original);

        let novel = library_novel(&folder).expect("library card");
        assert!(novel.complete);
        assert!(!novel.retryable);
        assert_eq!(novel.provenance.as_deref(), Some("migrated"));

        let stored = std::fs::read(folder.join("metadata.json")).unwrap();
        let again = adopt_epub(&epub, &folder).expect("idempotent adopt");
        assert_eq!(again.title, export.title);
        assert_eq!(std::fs::read(folder.join("metadata.json")).unwrap(), stored);
        assert_eq!(std::fs::read(&epub).unwrap(), original);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn adopt_refuses_structured_and_non_epub_folders() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-adopt-refuse-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let folder = root.join("not-an-epub");
        std::fs::create_dir_all(&folder).unwrap();
        let text = folder.join("notes.txt");
        std::fs::write(&text, "not a book").unwrap();
        assert!(adopt_epub(&text, &folder).is_err());

        let epub = folder.join("Book.epub");
        fixture(&epub, &"word ".repeat(80), 1);
        adopt_epub(&epub, &folder).expect("first adopt");
        let metadata = ExportMetadata {
            title: "Native".to_string(),
            ..Default::default()
        };
        std::fs::write(
            folder.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        assert!(adopt_epub(&epub, &folder).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn incomplete_index_is_written_and_blocks_completion() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-export-index-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture(&epub, &"word ".repeat(80), 3);

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 3,
            ..Default::default()
        };
        let index = NovelChapterIndex {
            source_url: "https://example.com/book".to_string(),
            source: "example.com".to_string(),
            title: None,
            total: 2,
            complete: false,
            detail: Some("listed 2 of 3 advertised chapters".to_string()),
            chapters: Vec::new(),
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        };
        let export = export_epub(
            &epub,
            &destination,
            &metadata,
            Some(&index),
            None,
            &report,
            true,
        )
        .unwrap();
        assert!(!export.index_complete);
        assert!(!export.complete);
        assert_eq!(
            export.index_detail.as_deref(),
            Some("listed 2 of 3 advertised chapters")
        );
        let written = std::fs::read_to_string(destination.join("metadata.json")).unwrap();
        assert!(written.contains("\"indexDetail\": \"listed 2 of 3 advertised chapters\""));
        assert!(written.contains("\"indexComplete\": false"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn library_novel_infers_completeness() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-library-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let good = root.join("good");
        std::fs::create_dir_all(&good).unwrap();
        let verified = ExportVerification {
            ok: true,
            chapters_found: 3,
            detail: "3/3 chapters present and non-empty".to_string(),
            ..Default::default()
        };
        let metadata = ExportMetadata {
            title: "Fixture".to_string(),
            source: Some("novelfire.net".to_string()),
            epub: Some("Fixture.epub".to_string()),
            chapter_count: 3,
            downloaded_chapters: 3,
            index_complete: true,
            verified: verified.clone(),
            ..Default::default()
        };
        std::fs::write(
            good.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        assert!(library_novel(&good).expect("novel").complete);

        let partial = root.join("partial");
        std::fs::create_dir_all(&partial).unwrap();
        let metadata = ExportMetadata {
            title: "Fixture".to_string(),
            source: Some("novelfire.net".to_string()),
            epub: Some("Fixture.epub".to_string()),
            index_complete: false,
            verified: ExportVerification {
                ok: false,
                detail: "empty chapter(s): 3".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        std::fs::write(
            partial.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        assert!(!library_novel(&partial).expect("novel").complete);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chapter_only_titles_do_not_collide() {
        assert_ne!(normalize_title("Chapter 1"), normalize_title("Chapter 2"));
        assert_eq!(
            normalize_title("Chapter 1: The Beginning"),
            normalize_title("Chapter 1 - The Beginning")
        );
    }

    /// EPUB with a nav document and NCX, so the retitling pass has something to
    /// rewrite. Each chapter heading is the given title, as lncrawl writes it.
    fn fixture_with_toc(path: &Path, titles: &[&str]) {
        let file = File::create(path).expect("create fixture");
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(br#"<container><rootfiles><rootfile full-path="EPUB/content.opf"/></rootfiles></container>"#).unwrap();

        let mut manifest = String::new();
        let mut spine = String::new();
        let mut nav = String::new();
        let mut ncx = String::new();
        for (offset, title) in titles.iter().enumerate() {
            let index = offset + 1;
            manifest.push_str(&format!(
                r#"<item id="c{index}" href="chapter_{index:05}.xhtml" media-type="application/xhtml+xml"/>"#
            ));
            spine.push_str(&format!(r#"<itemref idref="c{index}"/>"#));
            zip.start_file(format!("EPUB/chapter_{index:05}.xhtml"), deflated)
                .unwrap();
            zip.write_all(
                format!(
                    r#"<html><head><title>{title}</title></head><body><h1>{title}</h1><p>{}</p></body></html>"#,
                    "word ".repeat(80)
                )
                .as_bytes(),
            )
            .unwrap();
            nav.push_str(&format!(
                r#"<li><a href="chapter_{index:05}.xhtml">{title}</a></li>"#
            ));
            ncx.push_str(&format!(
                r#"<navPoint id="n{index}"><navLabel><text>{title}</text></navLabel><content src="chapter_{index:05}.xhtml"/></navPoint>"#
            ));
        }
        zip.start_file("EPUB/content.opf", deflated).unwrap();
        zip.write_all(format!(
            r#"<package><metadata><title>Fixture</title><creator>Author</creator></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>{manifest}</manifest><spine toc="ncx">{spine}</spine></package>"#
        ).as_bytes()).unwrap();
        zip.start_file("EPUB/nav.xhtml", deflated).unwrap();
        zip.write_all(
            format!(r#"<html><body><nav epub:type="toc"><ol>{nav}</ol></nav></body></html>"#)
                .as_bytes(),
        )
        .unwrap();
        zip.start_file("EPUB/toc.ncx", deflated).unwrap();
        zip.write_all(format!(r#"<ncx><navMap>{ncx}</navMap></ncx>"#).as_bytes())
            .unwrap();
        zip.finish().unwrap();
    }

    fn read_epub_entry(path: &Path, name: &str) -> String {
        let file = File::open(path).unwrap();
        let mut archive = ZipArchive::new(BufReader::new(file)).unwrap();
        let mut entry = archive.by_name(name).unwrap();
        let mut value = String::new();
        entry.read_to_string(&mut value).unwrap();
        value
    }

    #[test]
    fn borrowed_chapter_names_are_written_into_the_bundled_epub() {
        assert!(!carries_name("Chapter 12"));
        assert!(!carries_name("12"));
        assert!(carries_name("Chapter 12 - A name"));
        assert!(carries_name("A name"));
        assert_eq!(
            epub_heading(1, "The heart of a demon never has regret even in death"),
            "Chapter 1 - The heart of a demon never has regret even in death"
        );
        assert_eq!(
            epub_heading(348, "Chapter 348 - Chrysanthemum Akita Dog"),
            "Chapter 348 - Chrysanthemum Akita Dog"
        );
        assert_eq!(
            epub_heading(0, "There are three ways to survive in a ruined world"),
            "Chapter 0 - There are three ways to survive in a ruined world"
        );
        assert_eq!(
            epub_heading(551, "Epilogue 5 – The Eternity and Epilogue (Complete)"),
            "Chapter 551 - Epilogue 5 – The Eternity and Epilogue (Complete)"
        );

        let root = std::env::temp_dir().join(format!(
            "novelforge-export-retitle-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture_with_toc(&epub, &["Chapter 1", "Chapter 2"]);

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture".to_string(),
            source: "novelphoenix.net".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 2,
            ..Default::default()
        };
        let index = NovelChapterIndex {
            source_url: "https://novelphoenix.net/book".to_string(),
            source: "novelphoenix.net".to_string(),
            title: Some("Fixture".to_string()),
            total: 2,
            complete: true,
            detail: None,
            chapters: vec![
                NovelChapter {
                    number: 1,
                    title: "The heart of a demon never has regret even in death".to_string(),
                    url: "https://novelphoenix.net/book/chapter-1.html".to_string(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
                NovelChapter {
                    number: 2,
                    title: "Chapter 2".to_string(),
                    url: "https://novelphoenix.net/book/chapter-2.html".to_string(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
            ],
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: Some("novelfire.net".to_string()),
            title_lookup_attempted: true,
            title_recovery_attempted: true,
        };
        export_epub(
            &epub,
            &destination,
            &metadata,
            Some(&index),
            None,
            &report,
            true,
        )
        .unwrap();

        let heading = "Chapter 1 - The heart of a demon never has regret even in death";
        let chapter = read_epub_entry(&epub, "EPUB/chapter_00001.xhtml");
        assert!(
            chapter.contains(&format!("<title>{heading}</title>")),
            "{chapter}"
        );
        assert!(
            chapter.contains(&format!("<h1>{heading}</h1>")),
            "{chapter}"
        );
        let nav = read_epub_entry(&epub, "EPUB/nav.xhtml");
        assert!(nav.contains(&format!(">{heading}</a>")), "{nav}");
        let ncx = read_epub_entry(&epub, "EPUB/toc.ncx");
        assert!(ncx.contains(&format!("<text>{heading}</text>")), "{ncx}");
        // A row the source already named keeps lncrawl's heading.
        let second = read_epub_entry(&epub, "EPUB/chapter_00002.xhtml");
        assert!(second.contains("<h1>Chapter 2</h1>"), "{second}");
        // The rewritten archive is still a valid EPUB.
        assert_eq!(audit_epub(&epub).unwrap().chapters.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bundled_epub_is_untouched_when_every_row_was_named() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-export-noretitle-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture_with_toc(&epub, &["Chapter 1 - Crimson"]);
        let before = std::fs::read(&epub).unwrap();

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 1,
            ..Default::default()
        };
        let index = NovelChapterIndex {
            source_url: "https://novelphoenix.net/book".to_string(),
            source: "novelphoenix.net".to_string(),
            title: Some("Fixture".to_string()),
            total: 1,
            complete: true,
            detail: None,
            chapters: vec![NovelChapter {
                number: 1,
                title: "Chapter 1 - Crimson".to_string(),
                url: "https://novelphoenix.net/book/chapter-1.html".to_string(),
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
        };
        export_epub(
            &epub,
            &destination,
            &metadata,
            Some(&index),
            None,
            &report,
            true,
        )
        .unwrap();
        assert_eq!(std::fs::read(&epub).unwrap(), before);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn partial_selection_names_chapters_by_serial() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-export-serial-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture(&epub, &"word ".repeat(80), 2);

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let metadata = NovelMetadata {
            title: "Fixture".to_string(),
            ..Default::default()
        };
        let report = ExportVerification {
            ok: true,
            chapters_found: 2,
            ..Default::default()
        };
        let export = export_epub(
            &epub,
            &destination,
            &metadata,
            None,
            Some("3-4"),
            &report,
            false,
        )
        .unwrap();
        assert_eq!(export.downloaded_chapters, 2);
        assert!(
            destination
                .join("chapters/0003 - Chapter 1.xhtml")
                .is_file()
        );
        assert!(
            destination
                .join("chapters/0004 - Chapter 2.xhtml")
                .is_file()
        );
        assert!(!destination.join("chapters/0001 - Chapter 1.xhtml").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn audit_rejects_a_selection_that_fetched_the_neighbouring_chapter() {
        let audit_with = |titles: &[&str]| EpubAudit {
            title: None,
            author: None,
            description: None,
            language: None,
            chapters: titles
                .iter()
                .enumerate()
                .map(|(offset, title)| EpubChapter {
                    position: offset as u32 + 1,
                    title: title.to_string(),
                    archive_path: format!("OEBPS/ch{}.xhtml", offset + 1),
                    text_chars: 500,
                })
                .collect(),
            has_cover: false,
            cover_name: None,
            cover_bytes: None,
            opf_path: "OEBPS/content.opf".to_string(),
            opf_dir: "OEBPS".to_string(),
        };

        // ORV visible 2-3 must arrive as headings 2 and 3. A serial-based spec
        // fetched 1 and 2 while the count-only audit still passed.
        let shifted = plan_audit(
            &audit_with(&["Chapter 1", "Chapter 2"]),
            Some("2-3"),
            None,
            None,
        );
        assert!(!shifted.ok);
        assert_eq!(shifted.misnumbered_positions, vec![1, 2]);
        assert!(
            shifted
                .detail
                .contains("position 1 is Chapter 1, expected Chapter 2")
        );

        let correct = plan_audit(
            &audit_with(&["Chapter 2", "Chapter 3"]),
            Some("2-3"),
            None,
            None,
        );
        assert!(correct.ok);

        // Unnamed headings and volume-local restarts are not evidence of a
        // shift, so they keep the count-only behaviour.
        let unnamed = plan_audit(
            &audit_with(&["Prologue", "Chapter 1"]),
            Some("1-2"),
            None,
            None,
        );
        assert!(unnamed.ok);
        let restarted = plan_audit(
            &audit_with(&["Chapter 1", "Chapter 2", "Chapter 1", "Chapter 2"]),
            Some("1-4"),
            None,
            None,
        );
        assert!(restarted.ok);
    }

    #[test]
    fn merge_preserves_previous_chapters_and_removes_superseded_files() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-export-merge-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let destination = root.join("out");
        std::fs::create_dir_all(destination.join("chapters")).unwrap();
        std::fs::write(
            destination.join("chapters/0001 - Chapter 1.xhtml"),
            "<html>new one</html>",
        )
        .unwrap();
        std::fs::write(
            destination.join("chapters/0001 - Old name.xhtml"),
            "<html>old one</html>",
        )
        .unwrap();
        std::fs::write(
            destination.join("chapters/0002 - Chapter 2.xhtml"),
            "<html>old two</html>",
        )
        .unwrap();

        let previous = ExportMetadata {
            title: "Fixture".to_string(),
            source_url: Some("https://example.com/book".to_string()),
            source: Some("example.com".to_string()),
            chapter_count: 4,
            downloaded_chapters: 2,
            complete: true,
            chapters: vec![
                NovelChapter {
                    number: 1,
                    title: "Chapter 1".to_string(),
                    url: "https://example.com/book".to_string(),
                    volume: None,
                    downloaded: true,
                    file: Some("chapters/0001 - Old name.xhtml".to_string()),
                },
                NovelChapter {
                    number: 2,
                    title: "Chapter 2".to_string(),
                    url: "https://example.com/book".to_string(),
                    volume: None,
                    downloaded: true,
                    file: Some("chapters/0002 - Chapter 2.xhtml".to_string()),
                },
            ],
            ..Default::default()
        };
        std::fs::write(
            destination.join("metadata.json"),
            serde_json::to_vec(&previous).unwrap(),
        )
        .unwrap();

        let mut export = ExportMetadata {
            title: "Fixture".to_string(),
            source_url: Some("https://example.com/book/?utm_source=repeat".to_string()),
            source: Some("example.com".to_string()),
            chapter_count: 4,
            downloaded_chapters: 1,
            complete: true,
            chapters: vec![
                NovelChapter {
                    number: 1,
                    title: "Chapter 1".to_string(),
                    url: "https://example.com/book".to_string(),
                    volume: None,
                    downloaded: true,
                    file: Some("chapters/0001 - Chapter 1.xhtml".to_string()),
                },
                NovelChapter {
                    number: 2,
                    title: "Chapter 2".to_string(),
                    url: "https://example.com/book".to_string(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
                NovelChapter {
                    number: 3,
                    title: "Chapter 3".to_string(),
                    url: "https://example.com/book".to_string(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
            ],
            ..Default::default()
        };

        assert_eq!(merge_previous_export(&destination, &mut export), 1);
        assert_eq!(export.downloaded_chapters, 2);
        let chapter_two = export
            .chapters
            .iter()
            .find(|chapter| chapter.number == 2)
            .expect("chapter two");
        assert!(chapter_two.downloaded);
        assert_eq!(
            chapter_two.file.as_deref(),
            Some("chapters/0002 - Chapter 2.xhtml")
        );
        assert!(
            destination
                .join("chapters/0002 - Chapter 2.xhtml")
                .is_file()
        );
        assert!(!destination.join("chapters/0001 - Old name.xhtml").exists());
        assert!(
            destination
                .join("chapters/0001 - Chapter 1.xhtml")
                .is_file()
        );

        // A different source must not inherit chapters from the old manifest.
        let mut other = export.clone();
        other.source_url = Some("https://example.com/other".to_string());
        assert_eq!(merge_previous_export(&destination, &mut other), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manga_packaging_prefers_one_upload_per_chapter() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-manga-dedupe-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
        for (name, pages, group) in [("v01 c001 Short", 2, "A"), ("v01 c001 Longer", 4, "B")] {
            let dir = root.join("gallery-dl/mangadex/Example").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("info.json"),
                format!(r#"{{"chapter": 1, "volume": 1, "lang": "en", "group": ["{group}"]}}"#),
            )
            .unwrap();
            for page in 1..=pages {
                std::fs::write(dir.join(format!("{page:03}.png")), png).unwrap();
            }
        }

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let archive =
            package_manga_directory(&root.join("gallery-dl"), &destination, "cbz", "Example")
                .expect("package");
        let audit = audit_cbz(&archive).expect("audit");
        assert_eq!(audit.chapters.len(), 1);
        assert_eq!(audit.chapters[0].title, "0001 - v01 c001 Longer");
        assert_eq!(audit.chapters[0].images.len(), 4);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_empty_chapters() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-empty-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let epub = root.join("book.epub");
        fixture(&epub, "", 1);
        let audit = audit_epub(&epub).expect("audit");
        let plan = plan_audit(&audit, None, Some(1), None);
        assert!(!plan.ok);
        assert_eq!(plan.empty_positions, vec![1]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rebuilds_a_missing_archive_from_the_page_tree() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-manga-repack-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let destination = root.join("comic-0123abcd");
        let png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
        for (chapter, pages) in [("0002 - Chapter 2", 2), ("0001 - Chapter 1", 3)] {
            let folder = destination.join("chapters").join(chapter);
            std::fs::create_dir_all(&folder).unwrap();
            for page in 1..=pages {
                std::fs::write(folder.join(format!("{page:03}.png")), png).unwrap();
            }
        }

        let archive = repack_manga_chapters(&destination, "cbz", "Comic").expect("rebuild archive");
        assert_eq!(archive.file_name().unwrap(), "Comic.cbz");
        let audit = audit_cbz(&archive).expect("audit");
        assert_eq!(audit.chapters.len(), 2);
        // Natural order, not the read_dir order.
        assert!(audit.chapters[0].title.ends_with("Chapter 1"));
        assert_eq!(audit.chapters[0].images.len(), 3);
        assert_eq!(audit.chapters[1].images.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn library_prefers_the_archive_named_after_the_title() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-manga-archive-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let folder = root.join("example-0123abcd");
        std::fs::create_dir_all(folder.join("chapters/0001 - Chapter 1")).unwrap();
        std::fs::write(
            folder.join("chapters/0001 - Chapter 1/001.jpg"),
            [0xff, 0xd8, 0xff],
        )
        .unwrap();
        // A stray, larger archive must not win over the packed one.
        std::fs::write(folder.join("unrelated.zip"), vec![0u8; 4096]).unwrap();
        std::fs::write(folder.join("Example.cbz"), b"packed").unwrap();
        std::fs::write(folder.join("cover.jpg"), [0xff, 0xd8, 0xff]).unwrap();

        let metadata = ExportMetadata {
            title: "Example".to_string(),
            ..Default::default()
        };
        let archive = library_archive(&folder, &metadata).expect("archive");
        assert_eq!(archive.file_name().unwrap(), "Example.cbz");

        // No archive at all is not an error; the card simply has no CBZ action.
        std::fs::remove_file(folder.join("Example.cbz")).unwrap();
        std::fs::remove_file(folder.join("unrelated.zip")).unwrap();
        assert!(library_archive(&folder, &metadata).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_selection_specs() {
        assert_eq!(parse_selection_serials("all"), None);
        assert_eq!(parse_selection_serials("1-3,7"), Some(vec![1, 2, 3, 7]));
        assert_eq!(parse_selection_serials("5-1"), Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn packages_gallery_dl_tree_into_one_cbz() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-manga-pack-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let source = root.join("gallery-dl/mangadex/Example/v01 c001");
        std::fs::create_dir_all(&source).unwrap();
        let png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
        for page in 1..=3 {
            std::fs::write(source.join(format!("00{page}.png")), png).unwrap();
        }
        let second = root.join("gallery-dl/mangadex/Example/v01 c002");
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("001.png"), png).unwrap();

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();
        let archive =
            package_manga_directory(&root.join("gallery-dl"), &destination, "cbz", "Example")
                .expect("package");
        assert!(archive.ends_with("Example.cbz"));

        let audit = audit_cbz(&archive).expect("audit");
        assert_eq!(audit.chapters.len(), 2);
        assert_eq!(audit.image_count, 4);
        assert_eq!(audit.chapters[0].title, "0001 - v01 c001");
        assert_eq!(audit.chapters[0].images[0].0, "0001 - v01 c001/001.png");

        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(package_manga_directory(&empty, &destination, "cbz", "Empty").is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn measured_crawl_pace_is_persisted_and_exposed_to_the_library() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-pace-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let metadata = ExportMetadata {
            title: "Paced".to_string(),
            source_url: Some("https://novelfire.net/book/paced".to_string()),
            source: Some("novelfire".to_string()),
            ..Default::default()
        };
        write_metadata(&root, &metadata).unwrap();

        let pace = record_crawl_pace(&root, 20, std::time::Duration::from_secs(120))
            .expect("measure a two-minute crawl");
        assert_eq!(pace.chapters, 20);
        assert!((pace.chapters_per_minute - 10.0).abs() < 0.01, "{pace:?}");
        assert!((pace.seconds_per_chapter - 6.0).abs() < 0.01, "{pace:?}");

        let stored = read_export_metadata(&root).expect("metadata round trip");
        assert!(stored.crawl.is_some(), "crawl pace was not written");

        let novel = library_novel(&root).expect("library entry");
        assert_eq!(novel.source_host.as_deref(), Some("novelfire.net"));
        assert!(novel.source_rank > 0, "novelfire is measured slower");
        assert_eq!(
            novel.crawl.as_ref().map(|value| value.chapters),
            Some(20),
            "library did not expose the measured pace"
        );

        // A run without chapters cannot claim a pace.
        assert!(record_crawl_pace(&root, 0, std::time::Duration::from_secs(10)).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
