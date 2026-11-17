use crate::model::VerificationReport;
use crate::settings::now_ms;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use zip::CompressionMethod;

const EPUB_MIMETYPE: &str = "application/epub+zip";

/// Verifies that an EPUB is a real ZIP container whose `mimetype` entry
/// declares `application/epub+zip`. When `strict` is enabled the OCF rules are
/// enforced as well: `mimetype` must be the first entry and stored uncompressed.
pub fn verify_epub(path: &Path, strict: bool) -> VerificationReport {
    let mut report = VerificationReport {
        ok: false,
        container: false,
        mimetype: None,
        ocf_compliant: false,
        detail: String::new(),
        checked_at: now_ms(),
        chapters_expected: None,
        chapters_found: 0,
        empty_chapters: Vec::new(),
        missing_chapters: Vec::new(),
        has_cover: false,
    };

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            report.detail = format!("Cannot read file metadata: {error}");
            return report;
        }
    };
    if metadata.len() == 0 {
        report.detail = "File is empty".to_string();
        return report;
    }

    if !has_zip_magic(path) {
        report.detail = "Not a ZIP container (invalid magic bytes)".to_string();
        return report;
    }
    report.container = true;

    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            report.detail = format!("Cannot open file: {error}");
            return report;
        }
    };
    let mut archive = match zip::ZipArchive::new(BufReader::new(file)) {
        Ok(archive) => archive,
        Err(error) => {
            report.container = false;
            report.detail = format!("ZIP container is corrupt: {error}");
            return report;
        }
    };

    let index = archive.index_for_name("mimetype");
    let Some(index) = index else {
        report.detail = "ZIP container has no `mimetype` entry".to_string();
        return report;
    };

    let stored = {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(error) => {
                report.detail = format!("Cannot read `mimetype` entry: {error}");
                return report;
            }
        };

        let compression = entry.compression();
        // The spec fixes `mimetype` at 20 bytes; cap the read so a hostile
        // archive cannot inflate an arbitrarily large entry into memory.
        let mut buffer = Vec::with_capacity(64);
        if let Err(error) = entry.by_ref().take(64).read_to_end(&mut buffer).map(|_| ()) {
            report.detail = format!("Cannot decompress `mimetype` entry: {error}");
            return report;
        }
        let mut value = String::from_utf8_lossy(&buffer).to_string();
        value = value.trim_end_matches(['\0', '\r', '\n', ' ']).to_string();
        report.mimetype = Some(value.clone());
        report.ocf_compliant = index == 0 && compression == CompressionMethod::Stored;

        if value != EPUB_MIMETYPE {
            report.detail = format!(
                "Invalid EPUB mimetype: expected `{EPUB_MIMETYPE}`, found `{}`",
                truncate(&value, 80)
            );
            return report;
        }

        compression == CompressionMethod::Stored
    };

    if strict && !report.ocf_compliant {
        report.detail =
            "EPUB is missing OCF compliance: `mimetype` must be the first, uncompressed entry"
                .to_string();
        return report;
    }

    report.ok = true;
    report.detail = if report.ocf_compliant {
        format!("Verified EPUB container · mimetype {EPUB_MIMETYPE}")
    } else if stored {
        format!(
            "Verified EPUB mimetype {EPUB_MIMETYPE} · warning: `mimetype` is not the first entry"
        )
    } else {
        format!("Verified EPUB mimetype {EPUB_MIMETYPE} · warning: `mimetype` is compressed")
    };
    report
}

/// CBZ files are ZIP containers of images. We only need to guarantee the
/// container is structurally readable before surfacing a success state.
pub fn verify_cbz(path: &Path) -> VerificationReport {
    let mut report = VerificationReport {
        ok: false,
        container: false,
        mimetype: None,
        ocf_compliant: false,
        detail: String::new(),
        checked_at: now_ms(),
        chapters_expected: None,
        chapters_found: 0,
        empty_chapters: Vec::new(),
        missing_chapters: Vec::new(),
        has_cover: false,
    };

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            report.detail = format!("Cannot read file metadata: {error}");
            return report;
        }
    };
    if metadata.len() == 0 {
        report.detail = "File is empty".to_string();
        return report;
    }
    if !has_zip_magic(path) {
        report.detail = "Not a ZIP container (invalid magic bytes)".to_string();
        return report;
    }
    report.container = true;

    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            report.detail = format!("Cannot open file: {error}");
            return report;
        }
    };
    match zip::ZipArchive::new(BufReader::new(file)) {
        Ok(mut archive) => {
            if archive.is_empty() {
                report.detail = "ZIP container is empty".to_string();
                return report;
            }
            if let Err(error) = archive.validate_header_consistency() {
                report.detail = format!("Invalid CBZ archive: {error}");
                return report;
            }
            report.ok = true;
            report.detail = format!("Verified CBZ container · {} pages", archive.len());
            report
        }
        Err(error) => {
            report.container = false;
            report.detail = format!("ZIP container is corrupt: {error}");
            report
        }
    }
}

pub fn verify_pdf(path: &Path) -> VerificationReport {
    let mut report = VerificationReport {
        ok: false,
        container: false,
        mimetype: None,
        ocf_compliant: false,
        detail: String::new(),
        checked_at: now_ms(),
        chapters_expected: None,
        chapters_found: 0,
        empty_chapters: Vec::new(),
        missing_chapters: Vec::new(),
        has_cover: false,
    };
    let mut header = [0_u8; 5];
    match File::open(path).and_then(|mut file| file.read_exact(&mut header)) {
        Ok(()) if &header == b"%PDF-" => {
            report.ok = true;
            report.container = true;
            report.detail = "Verified PDF signature".to_string();
        }
        Ok(()) => report.detail = "Not a PDF (invalid signature)".to_string(),
        Err(error) => report.detail = format!("Cannot read PDF: {error}"),
    }
    report
}

pub fn verify_any(path: &Path, engine_is_books: bool) -> VerificationReport {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "epub" => {
            let mut report = verify_epub(path, engine_is_books);
            crate::export::verify_report(path, &mut report, None);
            if report.ok && report.chapters_found == 0 {
                report.ok = false;
                report.detail = format!("{} · no chapter files found", report.detail);
            }
            if report.ok && !report.empty_chapters.is_empty() {
                report.ok = false;
                report.detail = format!(
                    "{} · empty chapter(s): {}",
                    report.detail,
                    report
                        .empty_chapters
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            report
        }
        "cbz" | "zip" => {
            let mut report = verify_cbz(path);
            crate::export::verify_report(path, &mut report, None);
            if report.ok && report.chapters_found == 0 {
                report.ok = false;
                report.detail = format!("{} · no chapter images found", report.detail);
            }
            report
        }
        "pdf" => verify_pdf(path),
        _ => {
            let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
            VerificationReport {
                ok: size > 0,
                container: false,
                mimetype: None,
                ocf_compliant: false,
                detail: if size > 0 {
                    "File exists and is non-empty".to_string()
                } else {
                    "File is empty".to_string()
                },
                checked_at: now_ms(),
                chapters_expected: None,
                chapters_found: 0,
                empty_chapters: Vec::new(),
                missing_chapters: Vec::new(),
                has_cover: false,
            }
        }
    }
}

fn has_zip_magic(path: &Path) -> bool {
    let mut header = [0_u8; 4];
    match File::open(path).and_then(|mut file| file.read_exact(&mut header)) {
        Ok(()) => {
            matches!(
                header,
                [0x50, 0x4b, 0x03, 0x04] | [0x50, 0x4b, 0x05, 0x06] | [0x50, 0x4b, 0x07, 0x08]
            )
        }
        Err(_) => false,
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "novelforge-verify-{}-{unique}-{name}",
                std::process::id()
            ));
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8], CompressionMethod)]) {
        let file = File::create(path).expect("create zip");
        let mut writer = ZipWriter::new(file);
        for (name, body, method) in entries {
            let options = SimpleFileOptions::default().compression_method(*method);
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(body).expect("write entry");
        }
        writer.finish().expect("finish zip");
    }

    #[test]
    fn accepts_ocf_compliant_epub() {
        let file = TempFile::new("valid.epub");
        write_zip(
            file.path(),
            &[
                (
                    "mimetype",
                    EPUB_MIMETYPE.as_bytes(),
                    CompressionMethod::Stored,
                ),
                (
                    "META-INF/container.xml",
                    b"<container/>",
                    CompressionMethod::Deflated,
                ),
            ],
        );

        let report = verify_epub(file.path(), true);
        assert!(report.ok, "{}", report.detail);
        assert!(report.ocf_compliant);
        assert_eq!(report.mimetype.as_deref(), Some(EPUB_MIMETYPE));
    }

    #[test]
    fn lenient_accepts_but_strict_rejects_disordered_mimetype() {
        let file = TempFile::new("disordered.epub");
        write_zip(
            file.path(),
            &[
                (
                    "META-INF/container.xml",
                    b"<container/>",
                    CompressionMethod::Deflated,
                ),
                (
                    "mimetype",
                    EPUB_MIMETYPE.as_bytes(),
                    CompressionMethod::Stored,
                ),
            ],
        );

        let lenient = verify_epub(file.path(), false);
        assert!(lenient.ok, "{}", lenient.detail);
        assert!(!lenient.ocf_compliant);

        let strict = verify_epub(file.path(), true);
        assert!(!strict.ok);
        assert!(strict.detail.contains("OCF"));
    }

    #[test]
    fn strict_rejects_compressed_mimetype() {
        let file = TempFile::new("compressed.epub");
        write_zip(
            file.path(),
            &[(
                "mimetype",
                EPUB_MIMETYPE.as_bytes(),
                CompressionMethod::Deflated,
            )],
        );

        assert!(verify_epub(file.path(), false).ok);
        assert!(!verify_epub(file.path(), true).ok);
    }

    #[test]
    fn rejects_wrong_mimetype() {
        let file = TempFile::new("wrong.epub");
        write_zip(
            file.path(),
            &[("mimetype", b"application/zip", CompressionMethod::Stored)],
        );

        let report = verify_epub(file.path(), false);
        assert!(!report.ok);
        assert!(report.container);
    }

    #[test]
    fn rejects_non_zip_payload() {
        let file = TempFile::new("fake.epub");
        std::fs::write(file.path(), b"this is not a zip archive").expect("write payload");

        let report = verify_epub(file.path(), false);
        assert!(!report.ok);
        assert!(!report.container);
    }

    #[test]
    fn pdf_signature_is_enforced() {
        let good = TempFile::new("good.pdf");
        std::fs::write(good.path(), b"%PDF-1.7\n...").expect("write pdf");
        assert!(verify_pdf(good.path()).ok);

        let bad = TempFile::new("bad.pdf");
        std::fs::write(bad.path(), b"<html>not a pdf</html>").expect("write html");
        assert!(!verify_pdf(bad.path()).ok);
    }
}
