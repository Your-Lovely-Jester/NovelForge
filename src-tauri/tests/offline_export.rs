//! Offline integration test for the structured-export pipeline.
//!
//! No network and no external tools: this builds a real EPUB ZIP in a temp
//! directory and drives `audit_epub` + `export_epub` exactly as the download
//! job does after a crawler run. It verifies the artefacts a user relies on:
//! cleaned chapter XHTML, cover, metadata, `index.json`, `chapters.html` and the
//! completeness flags.

use novelforge_lib::export::{audit_epub, export_epub};
use novelforge_lib::model::{ExportVerification, NovelChapter, NovelChapterIndex, NovelMetadata};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "novelforge-offline-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    root
}

fn chapter(number: u32) -> String {
    format!(
        r#"<?xml version='1.0' encoding='utf-8'?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter {number}</title></head>
<body>
<div id="chapter">
<h4 aria-hidden="true">#{number}</h4>
<h1>Chapter {number}</h1>
<div class="chapter-nav"><a href="/prev">Previous</a><a href="/next">Next</a></div>
<p>Story paragraph one for chapter {number}. This sentence exists so the audit sees real text, not an empty document.</p>
<p>Story paragraph two for chapter {number}. It must survive the cleaner byte-for-byte in spirit and in text.</p>
</div>
</body></html>"#
    )
}

fn fixture_epub(path: &Path) {
    let file = std::fs::File::create(path).expect("create fixture");
    let mut zip = ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();
    zip.start_file("META-INF/container.xml", deflated).unwrap();
    zip.write_all(br#"<container><rootfiles><rootfile full-path="EPUB/content.opf"/></rootfiles></container>"#)
        .unwrap();

    zip.start_file("EPUB/content.opf", deflated).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf">
  <metadata>
    <title>Offline Fixture</title>
    <creator>Fixture Author</creator>
    <language>en</language>
    <meta name="cover" content="cover-image"/>
  </metadata>
  <manifest>
    <item id="cover-image" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
    <item id="c1" href="chapter_00001.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="chapter_00002.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>"#,
    )
    .unwrap();

    zip.start_file("EPUB/cover.jpg", deflated).unwrap();
    zip.write_all(&[0xff, 0xd8, 0xff, 0x00]).unwrap();

    for number in 1..=2 {
        zip.start_file(format!("EPUB/chapter_{number:05}.xhtml"), deflated)
            .unwrap();
        zip.write_all(chapter(number).as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn index_fixture(complete: bool) -> NovelChapterIndex {
    let mut chapters = Vec::new();
    for number in 1..=3 {
        chapters.push(NovelChapter {
            number,
            title: format!("Chapter {number}"),
            url: format!("https://example.com/book/chapter-{number}"),
            volume: Some(1),
            downloaded: false,
            file: None,
        });
    }
    NovelChapterIndex {
        source_url: "https://example.com/book".to_string(),
        source: "example.com".to_string(),
        title: Some("Offline Fixture".to_string()),
        total: 3,
        complete,
        detail: Some(if complete {
            "3 of 3 advertised chapters listed".to_string()
        } else {
            "2 of 3 advertised chapters listed".to_string()
        }),
        chapters,
        cached: false,
        cache_age_seconds: None,
        transport: None,
        title_source: None,
        title_lookup_attempted: false,
        title_recovery_attempted: false,
    }
}

fn metadata() -> NovelMetadata {
    NovelMetadata {
        title: "Offline Fixture".to_string(),
        author: Some("Fixture Author".to_string()),
        authors: vec!["Fixture Author".to_string()],
        synopsis: Some("A generated fixture used by the offline integration test.".to_string()),
        tags: vec!["Fantasy".to_string()],
        status: Some("Ongoing".to_string()),
        language: Some("en".to_string()),
        chapter_count: Some(3),
        source_url: "https://example.com/book".to_string(),
        source: "example.com".to_string(),
        cover: None,
        aliases: Vec::new(),
    }
}

fn verification() -> ExportVerification {
    ExportVerification {
        ok: true,
        chapters_expected: Some(2),
        chapters_found: 2,
        empty_chapters: Vec::new(),
        missing_chapters: Vec::new(),
        has_cover: true,
        detail: "2/2 chapters present and non-empty".to_string(),
    }
}

#[test]
fn offline_export_writes_cleaned_chapters_metadata_and_index() {
    let root = temp_root("export");
    let epub = root.join("book.epub");
    fixture_epub(&epub);

    let audit = audit_epub(&epub).expect("audit fixture");
    assert_eq!(audit.chapters.len(), 2);
    assert!(audit.has_cover);

    let destination = root.join("out");
    let index = index_fixture(true);
    let export = export_epub(
        &epub,
        &destination,
        &metadata(),
        Some(&index),
        None,
        &verification(),
        true,
    )
    .expect("export fixture");

    assert!(
        export.complete,
        "complete index + verified run must complete"
    );
    assert!(export.index_complete);
    assert_eq!(export.chapter_count, 3);
    assert_eq!(export.downloaded_chapters, 2);
    assert_eq!(
        export.index_detail.as_deref(),
        Some("3 of 3 advertised chapters listed")
    );

    // Chapter files exist, are cleaned, and keep story text.
    let chapter_one = std::fs::read_dir(destination.join("chapters"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("0001"))
        })
        .expect("chapter 1 file");
    let html = std::fs::read_to_string(chapter_one).unwrap();
    assert!(!html.contains("aria-hidden"));
    assert!(!html.contains("chapter-nav"));
    assert!(html.contains("<h1>Chapter 1</h1>"));
    assert!(html.contains("Story paragraph one for chapter 1"));

    // Sidecars and cover.
    for relative in ["metadata.json", "index.json", "chapters.html", "cover.jpg"] {
        assert!(
            destination.join(relative).is_file(),
            "{relative} must exist"
        );
    }

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(destination.join("metadata.json")).unwrap())
            .expect("metadata json");
    assert_eq!(written["title"], "Offline Fixture");
    assert_eq!(written["author"], "Fixture Author");
    assert_eq!(written["downloadedChapters"], 2);
    assert_eq!(written["complete"], true);

    let index_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(destination.join("index.json")).unwrap())
            .expect("index json");
    assert_eq!(index_json["chapters"].as_array().unwrap().len(), 3);
    let downloaded = index_json["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|chapter| chapter["downloaded"] == true)
        .count();
    assert_eq!(downloaded, 2);
    assert_eq!(index_json["indexComplete"], true);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn offline_export_blocks_completion_when_index_is_incomplete() {
    let root = temp_root("incomplete");
    let epub = root.join("book.epub");
    fixture_epub(&epub);
    let destination = root.join("out");
    let index = index_fixture(false);
    let export = export_epub(
        &epub,
        &destination,
        &metadata(),
        Some(&index),
        None,
        &verification(),
        true,
    )
    .expect("export fixture");

    assert!(!export.index_complete);
    assert!(
        !export.complete,
        "an incomplete source index must block completion"
    );
    assert_eq!(
        export.index_detail.as_deref(),
        Some("2 of 3 advertised chapters listed")
    );

    // The files are still written so a partial download can be inspected.
    assert!(destination.join("metadata.json").is_file());
    assert!(destination.join("index.json").is_file());
    assert!(destination.join("chapters.html").is_file());

    let _ = std::fs::remove_dir_all(&root);
}
