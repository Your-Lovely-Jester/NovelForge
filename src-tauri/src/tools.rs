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
    /// Directory scanned for installer shims. Surfaced in the UI and the
    /// diagnostics report so "not found" always says where the app looked.
    pub bin_dir: Option<String>,
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
                "~/.local/share/novelforge/tools/lncrawl-venv/bin/lncrawl",
            ])
        })
        .or_else(|| installed_bin("lncrawl"))?;

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
        })
        .or_else(|| installed_bin("gallery-dl"))?;

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
        })
        .or_else(|| installed_bin("aget"))?;

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

/// Launchers written by `scripts/install-tools.ps1` on Windows:
/// `%LOCALAPPDATA%\novelforge\bin`. `data_local_dir` is the same directory, so
/// tool discovery works without editing PATH or rebooting. On Linux the bash
/// installer writes to `~/.local/bin`, which `find_binary` already reaches via
/// PATH; looking here as well is a harmless fallback.
fn installed_bin(name: &str) -> Option<PathBuf> {
    installed_bin_in(&tool_root()?, name)
}

/// `%LOCALAPPDATA%\novelforge` on Windows, `~/.local/share/novelforge` on
/// Linux. Both installers write their launchers below it.
fn tool_root() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("novelforge"))
}

/// The `bin` directory the installers populate. Exposed so Settings and the
/// diagnostics report can name the exact folder that was searched.
pub fn tool_bin_dir() -> Option<PathBuf> {
    Some(tool_root()?.join("bin"))
}

/// Resolve `name` inside an installer bin directory. Split from
/// [`installed_bin`] so the lookup order can be tested without touching the
/// real `%LOCALAPPDATA%`.
fn installed_bin_in(root: &Path, name: &str) -> Option<PathBuf> {
    ["", ".exe", ".cmd"]
        .into_iter()
        .map(|extension| root.join("bin").join(format!("{name}{extension}")))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        // Windows has no executable bit; a regular file is callable.
        metadata.is_file()
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
        bin_dir: tool_bin_dir().map(|path| path.to_string_lossy().to_string()),
    }
}

/// A configured path that no longer exists is the most common reason a tool
/// "disappears" after it was found once. Report it explicitly instead of
/// leaving the row at a silent "not found".
fn configured_path_problem(path: Option<&str>, label: &str) -> Option<String> {
    path.filter(|path| !Path::new(path).is_file())
        .map(|path| format!("Configured {label} path is not a file: {path}"))
}

fn join_detail(base: String, problem: Option<String>) -> String {
    match problem {
        Some(problem) => format!("{base} · {problem}"),
        None => base,
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
        detail: configured_path_problem(settings.lncrawl_path.as_deref(), "lncrawl"),
        hint: install_hint("webnovel"),
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
        detail: Some(join_detail(
            "387 sites · 26 manga extractors · CBZ / ZIP".to_string(),
            configured_path_problem(settings.gallery_dl_path.as_deref(), "gallery-dl"),
        )),
        hint: install_hint("manga"),
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
        detail: Some(join_detail(
            if chromium {
                "Playwright Chromium ready".to_string()
            } else {
                "Chromium missing — Anna's Archive/Ocean will be skipped; LibGen and WeLib still work"
                    .to_string()
            },
            configured_path_problem(settings.aget_path.as_deref(), "aget"),
        )),
        hint: install_hint("books"),
    }
}

pub fn engine_for_tool_id(id: &str) -> Option<Engine> {
    match id {
        "lncrawl" => Some(Engine::Webnovel),
        "gallery_dl" => Some(Engine::Manga),
        "aget" => Some(Engine::Books),
        _ => None,
    }
}

fn component_for_engine(engine: Engine) -> &'static str {
    match engine {
        Engine::Webnovel => "webnovel",
        Engine::Manga => "manga",
        Engine::Books => "books",
    }
}

/// Probe one user-entered path without saving settings. The Settings page uses
/// this for its per-field "Check" button so typing a path has an immediate,
/// visible result instead of waiting for a full save + re-check.
pub async fn probe_tool_path(id: &str, path: &str) -> Option<ToolStatus> {
    let engine = engine_for_tool_id(id)?;
    let expanded = expand_tilde(path.trim());
    let trimmed = expanded.as_str();
    let missing = if trimmed.is_empty() || !Path::new(trimmed).is_file() {
        Some(if trimmed.is_empty() {
            "No path entered".to_string()
        } else {
            format!("No executable file at {trimmed}")
        })
    } else {
        None
    };

    if let Some(detail) = missing {
        return Some(ToolStatus {
            id: id.to_string(),
            label: match engine {
                Engine::Webnovel => "Lightnovel Crawler",
                Engine::Manga => "gallery-dl",
                Engine::Books => "Book sources (aget)",
            }
            .to_string(),
            found: false,
            path: None,
            version: None,
            detail: Some(detail),
            hint: install_hint(component_for_engine(engine)),
        });
    }

    let mut settings = Settings::default();
    match engine {
        Engine::Webnovel => settings.lncrawl_path = Some(trimmed.to_string()),
        Engine::Manga => settings.gallery_dl_path = Some(trimmed.to_string()),
        Engine::Books => settings.aget_path = Some(trimmed.to_string()),
    }

    Some(match engine {
        Engine::Webnovel => probe_lncrawl(&settings).await,
        Engine::Manga => probe_gallery_dl(&settings).await,
        Engine::Books => probe_aget(&settings).await,
    })
}

fn install_hint(component: &str) -> String {
    #[cfg(windows)]
    {
        format!("Run scripts\\install-tools.ps1 {component} (PowerShell)")
    }
    #[cfg(not(windows))]
    {
        format!("Run scripts/install-tools.sh {component}")
    }
}

async fn probe(program: &Path, prefix: &[String], args: &[&str]) -> Option<String> {
    let mut command = Command::new(crate::spawn::native_path(program));
    command.args(prefix).args(args).kill_on_drop(true);
    crate::spawn::hide_console(&mut command);
    // A cold `lncrawl version` imports the whole crawler package. On Windows
    // with a mechanical disk that can take well over five seconds; timing out
    // is not fatal (the binary is still reported as found) but it hides the
    // version line, so allow a realistic cold start.
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
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
    playwright_chromium_path().is_some()
}

/// Path to the Chromium binary Playwright downloaded for aget.
///
/// Playwright uses the platform cache directory (`~/.cache/ms-playwright` on
/// Linux, `%LOCALAPPDATA%\ms-playwright` on Windows). aget's OceanOfPDF driver
/// needs an explicit executable because nodriver cannot discover Playwright's
/// private browser.
pub fn playwright_chromium_path() -> Option<PathBuf> {
    let cache = dirs::cache_dir()?.join("ms-playwright");
    let mut installs: Vec<PathBuf> = std::fs::read_dir(cache)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("chromium"))
        .map(|entry| entry.path())
        .collect();
    // Several Playwright versions can be cached; newest first.
    installs.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    installs
        .iter()
        .find_map(|directory| chromium_binary(directory))
}

fn chromium_binary(directory: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let candidates = [
        directory.join("chrome-win64").join("chrome.exe"),
        directory.join("chrome-win").join("chrome.exe"),
    ];
    #[cfg(not(windows))]
    let candidates = [
        directory.join("chrome-linux64").join("chrome"),
        directory.join("chrome-linux").join("chrome"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("novelforge-tools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        root
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[test]
    fn installed_bin_finds_installer_shims() {
        let root = scratch("installed-bin");
        let shim = root.join("bin").join("lncrawl");
        std::fs::write(&shim, "#!/bin/sh\n").unwrap();
        make_executable(&shim);

        assert_eq!(installed_bin_in(&root, "lncrawl"), Some(shim));
        assert_eq!(installed_bin_in(&root, "gallery-dl"), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn configured_path_problem_names_missing_files() {
        let root = scratch("configured-path");
        let present = root.join("bin").join("lncrawl.cmd");
        std::fs::write(&present, "").unwrap();
        assert_eq!(
            configured_path_problem(Some(&present.to_string_lossy()), "lncrawl"),
            None
        );

        let missing = root.join("bin").join("gone.cmd");
        let detail = configured_path_problem(Some(&missing.to_string_lossy()), "lncrawl")
            .expect("missing path should be reported");
        assert!(detail.contains("not a file"), "{detail}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tool_ids_map_to_engines() {
        assert_eq!(engine_for_tool_id("lncrawl"), Some(Engine::Webnovel));
        assert_eq!(engine_for_tool_id("gallery_dl"), Some(Engine::Manga));
        assert_eq!(engine_for_tool_id("aget"), Some(Engine::Books));
        assert_eq!(engine_for_tool_id("unknown"), None);
    }

    #[test]
    fn chromium_binary_matches_the_playwright_layout() {
        let root = scratch("chromium");
        #[cfg(windows)]
        let downloaded = root.join("chrome-win64").join("chrome.exe");
        #[cfg(not(windows))]
        let downloaded = root.join("chrome-linux64").join("chrome");

        std::fs::create_dir_all(downloaded.parent().unwrap()).unwrap();
        std::fs::write(&downloaded, "").unwrap();
        assert_eq!(chromium_binary(&root), Some(downloaded));

        let _ = std::fs::remove_dir_all(&root);
    }
}
