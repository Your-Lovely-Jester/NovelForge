use crate::model::Engine;
use crate::settings::{Settings, expand_tilde};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub id: String,
    pub label: String,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub detail: Option<String>,
    pub hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsReport {
    pub lncrawl: ToolStatus,
    pub gallery_dl: ToolStatus,
    pub aget: ToolStatus,
    pub all_ready: bool,
    pub checked_at: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedTool {
    pub program: PathBuf,
    pub prefix: Vec<String>,
    pub label: &'static str,
    /// Short, shell-free description used in the terminal log.
    pub display: String,
}

impl ResolvedTool {
    pub fn command_line(&self, args: &[String]) -> String {
        let mut parts = vec![self.program.to_string_lossy().to_string()];
        parts.extend(self.prefix.iter().cloned());
        parts.extend(args.iter().cloned());
        parts
            .iter()
            .map(|part| {
                if part.contains(' ') {
                    format!("\"{part}\"")
                } else {
                    part.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn resolve_tool(settings: &Settings, engine: Engine) -> Option<ResolvedTool> {
    match engine {
        Engine::Webnovel => resolve_lncrawl(settings),
        Engine::Manga => resolve_gallery_dl(settings),
        Engine::Books => resolve_aget(settings),
    }
}

fn resolve_lncrawl(settings: &Settings) -> Option<ResolvedTool> {
    let program = settings
        .lncrawl_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            find_binary(&[
                "lncrawl",
                "~/.local/bin/lncrawl",
                "/usr/local/bin/lncrawl",
                "/usr/bin/lncrawl",
            ])
        })?;

    Some(ResolvedTool {
        display: program.to_string_lossy().to_string(),
        program,
        prefix: Vec::new(),
        label: "lncrawl",
    })
}

fn resolve_gallery_dl(settings: &Settings) -> Option<ResolvedTool> {
    let program = settings
        .gallery_dl_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            find_binary(&[
                "gallery-dl",
                "~/.local/bin/gallery-dl",
                "/usr/local/bin/gallery-dl",
                "/usr/bin/gallery-dl",
                "~/.local/share/novelforge/tools/gallery-dl-venv/bin/gallery-dl",
            ])
        })?;

    Some(ResolvedTool {
        display: program.to_string_lossy().to_string(),
        program,
        prefix: Vec::new(),
        label: "gallery-dl",
    })
}

fn resolve_aget(settings: &Settings) -> Option<ResolvedTool> {
    let path = settings
        .aget_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            find_binary(&[
                "aget",
                "~/.local/bin/aget",
                "~/.local/share/novelforge/tools/annas-archive-downloader/aget",
            ])
        })?;

    if path.extension().is_some_and(|ext| ext == "py") {
        let python = find_binary(&["python3", "/usr/bin/python3"])?;
        return Some(ResolvedTool {
            display: format!("{} {}", python.display(), path.display()),
            program: python,
            prefix: vec![path.to_string_lossy().to_string()],
            label: "aget",
        });
    }

    Some(ResolvedTool {
        display: path.to_string_lossy().to_string(),
        program: path,
        prefix: Vec::new(),
        label: "aget",
    })
}

fn find_binary(candidates: &[&str]) -> Option<PathBuf> {
    for candidate in candidates {
        let expanded = expand_tilde(candidate);
        let path = PathBuf::from(expanded);
        if path.components().count() > 1 {
            if is_executable(&path) {
                return Some(path);
            }
            continue;
        }
        if let Some(paths) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&paths) {
                let candidate = directory.join(candidate);
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub async fn check_tools(settings: &Settings) -> ToolsReport {
    let (lncrawl, gallery_dl, aget) = tokio::join!(
        probe_lncrawl(settings),
        probe_gallery_dl(settings),
        probe_aget(settings),
    );

    let all_ready = lncrawl.found && gallery_dl.found && aget.found;
    ToolsReport {
        lncrawl,
        gallery_dl,
        aget,
        all_ready,
        checked_at: crate::settings::now_ms(),
    }
}

async fn probe_lncrawl(settings: &Settings) -> ToolStatus {
    let resolved = resolve_lncrawl(settings);
    let version = match &resolved {
        Some(tool) => probe(&tool.program, &tool.prefix, &["version"]).await,
        None => None,
    };

    ToolStatus {
        id: "lncrawl".to_string(),
        label: "Lightnovel Crawler".to_string(),
        found: resolved.is_some(),
        path: resolved
            .as_ref()
            .map(|tool| tool.program.to_string_lossy().to_string()),
        version,
        detail: None,
        hint: "curl -fsSL https://github.com/lncrawl/lightnovel-crawler/releases/latest/download/lncrawl-linux -o ~/.local/bin/lncrawl && chmod +x ~/.local/bin/lncrawl".to_string(),
    }
}

async fn probe_gallery_dl(settings: &Settings) -> ToolStatus {
    let resolved = resolve_gallery_dl(settings);
    let version = match &resolved {
        Some(tool) => probe(&tool.program, &tool.prefix, &["--version"]).await,
        None => None,
    };

    ToolStatus {
        id: "gallery_dl".to_string(),
        label: "gallery-dl".to_string(),
        found: resolved.is_some(),
        path: resolved.as_ref().map(|tool| tool.display.clone()),
        version,
        detail: Some("387 sites · 26 manga extractors · CBZ / ZIP".to_string()),
        hint: "python3 -m pip install --user -U gallery-dl".to_string(),
    }
}

async fn probe_aget(settings: &Settings) -> ToolStatus {
    let resolved = resolve_aget(settings);
    // aget has no `--version`; its `--help` starts with a usage line, which is
    // not a version and should not be shown as one in Settings or diagnostics.
    let version = None;
    let chromium = playwright_chromium_installed();

    ToolStatus {
        id: "aget".to_string(),
        label: "Book sources (aget)".to_string(),
        // Chromium is only needed for Anna's Archive/Ocean. LibGen and WeLib are
        // pure HTTP, so a missing browser must not mark the whole engine absent.
        found: resolved.is_some(),
        path: resolved.as_ref().map(|tool| tool.display.clone()),
        version,
        detail: Some(if chromium {
            "Playwright Chromium ready".to_string()
        } else {
            "Chromium missing — Anna's Archive/Ocean will be skipped; LibGen and WeLib still work"
                .to_string()
        }),
        hint: "Run scripts/install-tools.sh to provision aget, Playwright and Chromium".to_string(),
    }
}

async fn probe(program: &Path, prefix: &[String], args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(prefix).args(args).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;

    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let raw = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .trim_matches('|')
        .trim()
        .to_string();

    let cleaned = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.chars().take(120).collect())
    }
}

fn playwright_chromium_installed() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let cache = home.join(".cache/ms-playwright");
    let Ok(entries) = std::fs::read_dir(cache) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().starts_with("chromium"))
}
