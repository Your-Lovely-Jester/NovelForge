//! Live chapter structuring while a crawl is still running (NF-039).
//!
//! lncrawl writes one zstd frame per chapter into the job's private work tree
//! as soon as it extracts the body:
//!
//! ```text
//! <work>/novels/<id>/chapters/000123.zst
//! ```
//!
//! Without this module nothing outside the work tree exists until the crawl
//! ends, so a two-hour download is unreadable for two hours and a crash loses
//! every structured file even though all bodies are safely on disk.
//!
//! The poller decompresses each new frame, applies the same
//! [`ChapterCleaner`] the export uses, and writes
//! `destination/chapters/<serial> - <title>.xhtml` atomically. The
//! authoritative export later rewrites that folder; [`sweep`] removes only the
//! provisional names that a real export file replaced, so a crash keeps
//! whatever was already structured.
//!
//! Titles come from the chapter body (the NovelPhoenix adapter emits a bold
//! `Chapter N – …` line and no heading). They are a display convenience only:
//! the serial, not the title, is what the final export matches on.

use crate::chapter_clean::ChapterCleaner;
use crate::export;
use crate::model::ExportMetadata;
use regex::Regex;
use ruzstd::decoding::StreamingDecoder;
use scraper::{Html, Selector};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

/// How often the work tree is checked for new chapter files.
const POLL: Duration = Duration::from_millis(500);
/// One decompressed chapter should never approach this; the cap only stops a
/// malformed frame from exhausting memory.
const MAX_CHAPTER_BYTES: u64 = 8 * 1024 * 1024;
/// Emit a progress line about this often so the dock shows live movement.
const PROGRESS_EVERY: usize = 250;

pub type Logger = Box<dyn Fn(String) + Send + 'static>;

/// Start the poller on a blocking thread. It exits within [`POLL`] of `stop`
/// being set, and writes nothing after that, so the caller can run the final
/// export without racing the sweep.
pub fn spawn(
    destination: PathBuf,
    work_dir: PathBuf,
    source: String,
    stop: Arc<AtomicBool>,
    log: Option<Logger>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        Structurer::new(destination, work_dir, source, log).run(&stop)
    })
}

/// Remove provisional chapter files that a real export file replaced.
///
/// Only files named `<serial>[ - …].xhtml` whose serial is present in the
/// export are considered. A chapter the export could not produce (`file` is
/// `None`) keeps its provisional file, so a partial run can never delete
/// chapters recovered from a crashed crawl. Returns the number removed.
///
/// `serial_to_visible` is the catalogue's serial → displayed-number map
/// (NF-044). The crawler names a frame by serial, but the export names the
/// chapter by the number the source shows. On ORV serial 4 is visible
/// Chapter 3, so the provisional `0004` must be compared with the export's
/// Chapter 3 file, not with an unselected Chapter 4.
pub fn sweep(
    destination: &Path,
    export: &ExportMetadata,
    serial_to_visible: &HashMap<u32, u32>,
) -> usize {
    let chapters = destination.join("chapters");
    let Ok(entries) = fs::read_dir(&chapters) else {
        return 0;
    };
    // Every file the authoritative export wrote is final. This is also what
    // separates an exported name (a visible number) from a provisional name (a
    // serial) when the two numberings differ.
    let written = export
        .chapters
        .iter()
        .filter_map(|chapter| chapter.file.as_deref())
        .collect::<HashSet<_>>();
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        // A crash can leave the structurer's dot-prefixed temp file behind. It
        // has a name the reader ignores, so remove it once the structurer has
        // stopped (this runs after the join) instead of letting it accumulate.
        if name.starts_with('.') && name.ends_with(".part") {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
            continue;
        }
        let relative = format!("chapters/{name}");
        if written.contains(relative.as_str()) {
            continue;
        }
        let Some(serial) = file_serial(name) else {
            continue;
        };
        // A provisional file is named by the crawler's serial. Remove it only
        // when the export produced the visible chapter that serial belongs to;
        // a chapter the export could not produce keeps its streamed copy.
        let number = serial_to_visible.get(&serial).copied().unwrap_or(serial);
        let produced = export
            .chapters
            .iter()
            .any(|chapter| chapter.number == number && chapter.file.is_some());
        if produced && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Serial prefix of a structured chapter file (`0001`, `0001 - Title.xhtml`).
fn file_serial(name: &str) -> Option<u32> {
    if !name.ends_with(".xhtml") {
        return None;
    }
    let head = name.split(' ').next()?;
    if head.len() < 4 || !head.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    head.parse().ok()
}

struct Structurer {
    destination: PathBuf,
    work_dir: PathBuf,
    cleaner: ChapterCleaner,
    log: Option<Logger>,
    /// Serials that already have a provisional file.
    structured: HashSet<u32>,
    /// Failed serials keyed by (size, mtime). A file that has not changed since
    /// the failure is skipped instead of decoded twice a second.
    failed: HashMap<u32, (u64, Option<SystemTime>)>,
    announced: usize,
}

impl Structurer {
    fn new(destination: PathBuf, work_dir: PathBuf, source: String, log: Option<Logger>) -> Self {
        Self {
            destination,
            work_dir,
            cleaner: ChapterCleaner::for_source(Some(source.as_str())),
            log,
            structured: HashSet::new(),
            failed: HashMap::new(),
            announced: 0,
        }
    }

    fn run(mut self, stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) {
            self.poll_once();
            std::thread::sleep(POLL);
        }
        if !self.structured.is_empty() {
            self.system(format!(
                "Live structuring · {} chapter file(s) ready before the final export",
                self.structured.len()
            ));
        }
    }

    /// One pass over every chapter store in the work tree.
    fn poll_once(&mut self) -> usize {
        let novels = self.work_dir.join("novels");
        let Ok(entries) = fs::read_dir(&novels) else {
            return 0;
        };
        let mut chapters_dirs = entries
            .flatten()
            .map(|entry| entry.path().join("chapters"))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        chapters_dirs.sort();

        let mut written = 0;
        for chapters in chapters_dirs {
            let Ok(entries) = fs::read_dir(&chapters) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(serial) = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .and_then(chapter_serial)
                else {
                    continue;
                };
                if self.structured.contains(&serial) {
                    continue;
                }
                let stamp = entry
                    .metadata()
                    .map(|meta| (meta.len(), meta.modified().ok()))
                    .unwrap_or_default();
                if self.failed.get(&serial) == Some(&stamp) {
                    continue;
                }
                match self.structure(&path, serial) {
                    Ok(Some(name)) => {
                        self.failed.remove(&serial);
                        self.structured.insert(serial);
                        written += 1;
                        let total = self.structured.len();
                        if (total == 1 || total.is_multiple_of(PROGRESS_EVERY))
                            && total > self.announced
                        {
                            self.announced = total;
                            self.system(format!(
                                    "Live structuring · {total} chapter file(s) ready while the crawl continues · latest {name}"
                                ));
                        }
                    }
                    Ok(None) | Err(_) => {
                        // A chapter file can be observed mid-write; the frame
                        // header or the trailing block then fails. Retry once
                        // the file changes.
                        self.failed.insert(serial, stamp);
                    }
                }
            }
        }
        written
    }

    /// Decode, clean and write one chapter. `Ok(None)` means the frame was
    /// readable but contained no usable body.
    fn structure(&self, path: &Path, serial: u32) -> Result<Option<String>, String> {
        let compressed = fs::read(path).map_err(|error| error.to_string())?;
        let mut raw = Vec::new();
        let decoder = StreamingDecoder::new(compressed.as_slice())
            .map_err(|error| format!("not a complete zstd frame yet: {error}"))?;
        decoder
            .take(MAX_CHAPTER_BYTES)
            .read_to_end(&mut raw)
            .map_err(|error| format!("truncated zstd frame: {error}"))?;
        if raw.len() < crate::export::MIN_CHAPTER_CHARS {
            return Ok(None);
        }
        let text = std::str::from_utf8(&raw).map_err(|error| error.to_string())?;
        let title = provisional_title(text, serial);
        // lncrawl stores a bare fragment. Wrapping it first gives the cleaner a
        // real document, so the split file keeps the XHTML namespace and a
        // `<title>` an external reader can show.
        let document = wrapped(text, &title);
        let cleaned = self.cleaner.clean(document.as_bytes(), &title);
        let bytes = cleaned.as_deref().unwrap_or(&raw);
        let name = export::chapter_file_name(serial, &title);
        write_atomic(&self.destination.join("chapters"), &name, bytes)?;
        Ok(Some(name))
    }

    fn system(&self, message: String) {
        if let Some(log) = &self.log {
            log(message);
        }
    }
}

/// `000123.zst` → `Some(123)`. Translations such as `000123.en.zst` and the
/// database are ignored.
fn chapter_serial(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".zst")?;
    if stem.is_empty() || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

fn provisional_title(html: &str, serial: u32) -> String {
    let document = Html::parse_document(html);
    for selector in ["h1", "h2", "h3"] {
        if let Ok(selector) = Selector::parse(selector)
            && let Some(element) = document.select(&selector).next()
        {
            let text = collapse(&element.text().collect::<String>());
            if !text.is_empty() && text.chars().count() <= 160 {
                return text;
            }
        }
    }
    // NovelPhoenix-style bodies carry no heading: the first paragraphs are the
    // bold chapter line, then the story.
    if let Ok(selector) = Selector::parse("p") {
        for paragraph in document.select(&selector).take(3) {
            let text = collapse(&paragraph.text().collect::<String>());
            if text.is_empty() || text.chars().count() > 160 {
                continue;
            }
            if chapter_prefix().is_match(&text) {
                return text;
            }
        }
    }
    format!("Chapter {serial}")
}

fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wrap lncrawl's body fragment in a full XHTML document. Adapters that already
/// return a document are passed through untouched.
fn wrapped(body: &str, title: &str) -> String {
    if body.to_ascii_lowercase().contains("<html") {
        return body.to_string();
    }
    format!(
        "<?xml version='1.0' encoding='utf-8'?><!DOCTYPE html>\
<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\" xml:lang=\"en\">\
<head><title>{}</title></head><body>{body}</body></html>",
        escape_text(title)
    )
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn chapter_prefix() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:chapter|ch\.|part|volume|vol\.|prologue|epilogue|afterword|interlude)\b",
        )
        .expect("chapter prefix regex")
    })
}

fn write_atomic(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let temporary = directory.join(format!(".{name}.part"));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    crate::fsutil::replace_file(&temporary, &directory.join(name)).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NovelChapter;

    const REAL_CHAPTER: &[u8] = include_bytes!("../tests/fixtures/novelphoenix/chapter_0001.zst");

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "novelforge-structurer-{label}-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    /// Work tree shaped like lncrawl's store.
    fn store(root: &Path, bytes: &[u8]) -> PathBuf {
        let chapters = root.join("work/novels/id/chapters");
        fs::create_dir_all(&chapters).unwrap();
        fs::write(chapters.join("000001.zst"), bytes).unwrap();
        chapters
    }

    fn run_once(root: &Path) -> Vec<String> {
        let mut structurer = Structurer::new(
            root.join("out"),
            root.join("work"),
            "novelphoenix.net".to_string(),
            None,
        );
        assert_eq!(structurer.poll_once(), 1);
        // A second pass must not rewrite or duplicate anything.
        assert_eq!(structurer.poll_once(), 0);
        let mut names = fs::read_dir(root.join("out/chapters"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn structures_a_real_lncrawl_frame_during_the_crawl() {
        let root = temp_root("live");
        store(&root, REAL_CHAPTER);
        let names = run_once(&root);
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(
            names[0].starts_with("0001 - Chapter 1"),
            "unexpected name: {}",
            names[0]
        );
        let body = fs::read_to_string(root.join("out/chapters").join(&names[0])).unwrap();
        assert!(body.starts_with("<?xml version='1.0' encoding='utf-8'?>"));
        assert!(body.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
        assert!(body.contains("<title>Chapter 1 – The heart of a demon"));
        assert!(body.contains("The heart of a demon never has regret even in death"));
        assert!(body.contains("quietly hand over the Spring Autumn Cicada"));
        assert!(!body.contains("<nav>"), "chrome must be cleaned");
        // The title paragraph is the only title in this fragment, so it stays.
        assert!(body.contains("<strong>Chapter 1 –"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_partial_frame_is_retried_until_the_writer_finishes() {
        let root = temp_root("partial");
        let chapters = store(&root, &REAL_CHAPTER[..REAL_CHAPTER.len() / 3]);
        let mut structurer = Structurer::new(
            root.join("out"),
            root.join("work"),
            "novelphoenix.net".to_string(),
            None,
        );
        assert_eq!(structurer.poll_once(), 0);
        assert!(!root.join("out/chapters").is_dir());

        fs::write(chapters.join("000001.zst"), REAL_CHAPTER).unwrap();
        assert_eq!(structurer.poll_once(), 1);
        let files = fs::read_dir(root.join("out/chapters")).unwrap().count();
        assert_eq!(files, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignores_translations_and_non_zstd_files() {
        let root = temp_root("names");
        let chapters = store(&root, REAL_CHAPTER);
        fs::write(chapters.join("000001.en.zst"), REAL_CHAPTER).unwrap();
        fs::write(chapters.join("notes.txt"), b"ignore").unwrap();
        let names = run_once(&root);
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("0001 - "));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_only_removes_provisional_names_the_export_replaced() {
        let root = temp_root("sweep");
        let chapters = root.join("out/chapters");
        fs::create_dir_all(&chapters).unwrap();
        fs::write(chapters.join("0001 - Provisional Old Title.xhtml"), b"old").unwrap();
        fs::write(chapters.join("0001 - Chapter 1.xhtml"), b"real").unwrap();
        fs::write(chapters.join("0002 - Not Exported Yet.xhtml"), b"keep").unwrap();
        fs::write(chapters.join("style.css"), b"css").unwrap();

        let export = ExportMetadata {
            chapters: vec![
                NovelChapter {
                    number: 1,
                    title: "Chapter 1".to_string(),
                    url: String::new(),
                    volume: None,
                    downloaded: true,
                    file: Some("chapters/0001 - Chapter 1.xhtml".to_string()),
                },
                NovelChapter {
                    number: 2,
                    title: "Chapter 2".to_string(),
                    url: String::new(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(sweep(&root.join("out"), &export, &HashMap::new()), 1);
        assert!(chapters.join("0001 - Chapter 1.xhtml").is_file());
        assert!(!chapters.join("0001 - Provisional Old Title.xhtml").exists());
        // A chapter the export could not produce keeps its streamed file, so a
        // partial run never deletes chapters recovered from a crashed crawl.
        assert!(chapters.join("0002 - Not Exported Yet.xhtml").is_file());
        assert!(chapters.join("style.css").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_maps_a_serial_to_the_visible_chapter_the_export_wrote() {
        let root = temp_root("sweep-visible");
        let chapters = root.join("out/chapters");
        fs::create_dir_all(&chapters).unwrap();
        // lncrawl serial 4 is visible Chapter 3 on a 0-based catalogue.
        fs::write(chapters.join("0004 - Chapter 4.xhtml"), b"serial four").unwrap();
        fs::write(chapters.join("0003 - Chapter 3.xhtml"), b"visible three").unwrap();
        fs::write(chapters.join("0009 - Not Selected.xhtml"), b"keep").unwrap();

        let export = ExportMetadata {
            chapters: vec![
                NovelChapter {
                    number: 3,
                    title: "Chapter 3".to_string(),
                    url: String::new(),
                    volume: None,
                    downloaded: true,
                    file: Some("chapters/0003 - Chapter 3.xhtml".to_string()),
                },
                NovelChapter {
                    number: 4,
                    title: "Chapter 4".to_string(),
                    url: String::new(),
                    volume: None,
                    downloaded: false,
                    file: None,
                },
            ],
            ..Default::default()
        };
        let map = HashMap::from([(4, 3)]);
        assert_eq!(sweep(&root.join("out"), &export, &map), 1);
        // The provisional copy of visible Chapter 3 is gone; the real file is
        // untouched and an unproduced chapter keeps its streamed copy.
        assert!(!chapters.join("0004 - Chapter 4.xhtml").exists());
        assert!(chapters.join("0003 - Chapter 3.xhtml").is_file());
        assert!(chapters.join("0009 - Not Selected.xhtml").is_file());
        let _ = fs::remove_dir_all(&root);
    }
}
