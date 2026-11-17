use crate::error::{AppError, AppResult};
use crate::model::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// A user-pinned transport limit for one source host. The built-in profiles
/// remain the starting point; an entry here is a hard ceiling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceLimit {
    pub host: String,
    pub concurrency: u8,
    pub request_gap_ms: u64,
    pub chapter_rate: f32,
}

impl Default for SourceLimit {
    fn default() -> Self {
        Self {
            host: String::new(),
            concurrency: 1,
            request_gap_ms: 900,
            chapter_rate: 2.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub download_dir: String,
    pub lncrawl_path: Option<String>,
    /// `kotatsuPath` is accepted once so an existing config keeps working
    /// after the NF-022 engine swap.
    #[serde(alias = "kotatsuPath")]
    pub gallery_dl_path: Option<String>,
    pub aget_path: Option<String>,
    pub default_engine: Engine,
    pub manga_format: String,
    pub manga_throttle: bool,
    /// BCP-47-ish language code passed to gallery-dl as `lang`. `None` lets
    /// the extractor download every available translation.
    pub manga_language: Option<String>,
    pub book_language: Option<String>,
    /// Per-host transport overrides. Empty means the built-in profiles apply.
    pub source_limits: Vec<SourceLimit>,
    /// Maximum bytes a new download may push the library past. `0` disables
    /// the quota. Existing partial folders are never deleted to enforce it.
    pub max_library_bytes: u64,
    /// Remove the per-job crawler work tree after a verified import. Disable
    /// only when debugging a source adapter; leftover trees count separately
    /// from library bytes and can be cleaned from the Library page.
    pub cleanup_work_after_import: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir().to_string_lossy().to_string(),
            lncrawl_path: None,
            gallery_dl_path: None,
            aget_path: None,
            default_engine: Engine::Webnovel,
            manga_format: "cbz".to_string(),
            manga_throttle: true,
            manga_language: Some("en".to_string()),
            book_language: None,
            source_limits: Vec::new(),
            max_library_bytes: 0,
            cleanup_work_after_import: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .map(Self::normalized)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut value = serde_json::to_value(self)
            .map_err(|error| AppError::msg(format!("Could not serialise settings: {error}")))?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "schemaVersion".to_string(),
                serde_json::json!(crate::model::SCHEMA_VERSION),
            );
        }
        let json = serde_json::to_string_pretty(&value)
            .map_err(|error| AppError::msg(format!("Could not serialise settings: {error}")))?;
        crate::fsutil::write_text_atomic(path, &json)
            .map_err(|error| AppError::msg(format!("Could not write settings: {error}")))?;
        Ok(())
    }

    /// Expands `~`, clamps numeric engine options and verifies the download root.
    pub fn normalized(mut self) -> Self {
        self.download_dir = expand_tilde(self.download_dir.trim());
        if self.download_dir.is_empty() {
            self.download_dir = default_download_dir().to_string_lossy().to_string();
        }

        self.lncrawl_path = normalise_optional_path(self.lncrawl_path);
        self.gallery_dl_path = normalise_optional_path(self.gallery_dl_path);
        self.aget_path = normalise_optional_path(self.aget_path);

        if !matches!(self.manga_format.as_str(), "cbz" | "zip") {
            // NF-022: gallery-dl always packages chapter folders into one
            // archive; the old loose-folder mode no longer has an engine.
            self.manga_format = "cbz".to_string();
        }
        self.manga_language = self
            .manga_language
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        self.book_language = self
            .book_language
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        // 100 TiB ceiling: a typo cannot ask for an impossible quota, and `0`
        // stays the explicit "unlimited" value.
        self.max_library_bytes = self.max_library_bytes.min(100 * 1024_u64.pow(4));

        let mut seen = HashSet::new();
        self.source_limits = self
            .source_limits
            .into_iter()
            .filter_map(|mut limit| {
                let host = normalize_source_host(&limit.host)?;
                limit.host = host.clone();
                limit.concurrency = limit.concurrency.clamp(1, 8);
                limit.request_gap_ms = limit.request_gap_ms.min(10_000);
                limit.chapter_rate = limit.chapter_rate.clamp(0.1, 20.0);
                if !seen.insert(host) {
                    return None;
                }
                Some(limit)
            })
            .collect();
        self
    }

    pub fn download_root(&self) -> PathBuf {
        PathBuf::from(&self.download_dir)
    }
}

pub fn prepare_download_dir(settings: &Settings) -> AppResult<PathBuf> {
    let root = settings.download_root();
    std::fs::create_dir_all(&root).map_err(|error| {
        AppError::msg(format!(
            "Could not create download directory {}: {error}",
            root.display()
        ))
    })?;
    root.canonicalize().map_err(|error| {
        AppError::msg(format!(
            "Could not resolve download directory {}: {error}",
            root.display()
        ))
    })
}

pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("NovelForge")
}

/// Accepts `host`, `host:port` or a full source URL and returns a stable
/// lowercase host key. Invalid/empty entries are dropped instead of silently
/// matching every source.
pub fn normalize_source_host(value: &str) -> Option<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(&trimmed);
    let authority = without_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    let parsed = Url::parse(&format!("http://{authority}")).ok()?;
    let host = parsed.host_str()?.trim_start_matches("www.").to_string();
    if host.is_empty() {
        return None;
    }
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

pub fn expand_tilde(value: &str) -> String {
    if value == "~" {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .to_string();
    }
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().to_string();
    }
    value.to_string()
}

fn normalise_optional_path(value: Option<String>) -> Option<String> {
    value
        .map(|path| expand_tilde(path.trim()))
        .filter(|path| !path.is_empty())
}

pub fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut last_dash = false;

    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 42 {
            break;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_host_normalisation_accepts_urls_and_ports() {
        assert_eq!(
            normalize_source_host("https://www.NovelFull.net/reverend-insanity.html"),
            Some("novelfull.net".to_string())
        );
        assert_eq!(
            normalize_source_host("example.com:8443"),
            Some("example.com:8443".to_string())
        );
        assert_eq!(normalize_source_host("  "), None);
        assert_eq!(normalize_source_host("not a host"), None);
    }

    #[test]
    fn source_limits_are_clamped_and_deduplicated() {
        let settings = Settings {
            source_limits: vec![
                SourceLimit {
                    host: "https://novelfull.net/book".to_string(),
                    concurrency: 99,
                    request_gap_ms: 99_999,
                    chapter_rate: 99.0,
                },
                SourceLimit {
                    host: "novelfull.net".to_string(),
                    concurrency: 2,
                    request_gap_ms: 500,
                    chapter_rate: 3.0,
                },
                SourceLimit {
                    host: String::new(),
                    ..SourceLimit::default()
                },
            ],
            ..Settings::default()
        }
        .normalized();

        assert_eq!(settings.source_limits.len(), 1);
        let limit = &settings.source_limits[0];
        assert_eq!(limit.host, "novelfull.net");
        assert_eq!(limit.concurrency, 8);
        assert_eq!(limit.request_gap_ms, 10_000);
        assert_eq!(limit.chapter_rate, 20.0);
    }

    #[test]
    fn quota_is_clamped_and_defaults_to_unlimited() {
        let defaults = Settings::default().normalized();
        assert_eq!(defaults.max_library_bytes, 0);
        assert!(defaults.cleanup_work_after_import);

        let settings = Settings {
            max_library_bytes: u64::MAX,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.max_library_bytes, 100 * 1024_u64.pow(4));
    }
}
