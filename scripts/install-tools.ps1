# NovelForge Windows toolchain installer
# ---------------------------------------------------------------------------
# Installs the three download engines NovelForge drives on native Windows:
#
#   lncrawl    webnovels  -> EPUB
#   gallery-dl manga      -> CBZ / ZIP
#   aget       e-books    -> EPUB / PDF (Anna's Archive, Playwright)
#
# Everything lands under %LOCALAPPDATA%\novelforge, no administrator rights:
#   %LOCALAPPDATA%\novelforge\bin\lncrawl.cmd
#   %LOCALAPPDATA%\novelforge\bin\gallery-dl.cmd
#   %LOCALAPPDATA%\novelforge\bin\aget.cmd
#   %LOCALAPPDATA%\novelforge\tools\lncrawl-venv
#   %LOCALAPPDATA%\novelforge\tools\gallery-dl-venv
#   %LOCALAPPDATA%\novelforge\tools\aget-venv
#   %LOCALAPPDATA%\novelforge\tools\lightnovel-crawler
#   %LOCALAPPDATA%\novelforge\tools\annas-archive-downloader
#
# NovelForge searches that bin directory directly, so no PATH edit and no
# reboot are required. Re-running is safe.
#
# Usage (from a clone of the repository):
#   powershell -ExecutionPolicy Bypass -File scripts\install-tools.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\install-tools.ps1 webnovel
#   powershell -ExecutionPolicy Bypass -File scripts\install-tools.ps1 manga books
#   powershell -ExecutionPolicy Bypass -File scripts\install-tools.ps1 all -SkipBrowser
#
# Requirements: Python 3.10+ and Git. Both are available from winget:
#   winget install -e --id Python.Python.3.12
#   winget install -e --id Git.Git
# ---------------------------------------------------------------------------
[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$Components,

    # Skip the ~170 MB Playwright Chromium download. LibGen and WeLib still
    # work; Anna's Archive and OceanOfPDF are skipped until it is installed.
    [switch]$SkipBrowser
)

$ErrorActionPreference = "Stop"

# Pinned versions: an upstream release must not silently change crawl output.
# Bump these together with a live smoke run (see docs/QA_MATRIX.md).
$LncrawlRepo = "https://github.com/lncrawl/lightnovel-crawler.git"
$LncrawlTag = "v4.14.0"
$GalleryDlVersion = "1.32.20"
$AgetRepo = "https://github.com/ALBEDO-TABAI/annas-archive-downloader.git"
$AgetClick = "8.5.1"
$AgetNodriver = "0.50.99"
$AgetPlaywright = "1.65.0"
$AgetRich = "15.0.0"

$Root = Join-Path $env:LOCALAPPDATA "novelforge"
$BinDir = Join-Path $Root "bin"
$ToolsDir = Join-Path $Root "tools"
$LncrawlDir = Join-Path $ToolsDir "lightnovel-crawler"
$LncrawlVenv = Join-Path $ToolsDir "lncrawl-venv"
$GalleryVenv = Join-Path $ToolsDir "gallery-dl-venv"
$AgetDir = Join-Path $ToolsDir "annas-archive-downloader"
$AgetVenv = Join-Path $ToolsDir "aget-venv"

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Write-Warn {
    param([string]$Message)
    Write-Host "[!] $Message" -ForegroundColor Yellow
}

function Fail {
    param([string]$Message)
    Write-Host "[x] $Message" -ForegroundColor Red
    exit 1
}

function Invoke-Native {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [string]$Description = ""
    )
    & $FilePath @ArgumentList
    if ($LASTEXITCODE -ne 0) {
        if ($Description) {
            Fail "$Description failed (exit $LASTEXITCODE)"
        }
        Fail "$FilePath $($ArgumentList -join ' ') failed (exit $LASTEXITCODE)"
    }
}

function Write-CmdShim {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Target,
        [string[]]$TargetArgs = @()
    )
    $quotedTarget = '"' + $Target + '"'
    $quotedArgs = ($TargetArgs | ForEach-Object { '"' + $_ + '"' }) -join " "
    $command = ($quotedTarget + " " + $quotedArgs).Trim() + " %*"
    $path = Join-Path $BinDir ($Name + ".cmd")
    $lines = @("@echo off", $command)
    Set-Content -LiteralPath $path -Value $lines -Encoding ASCII
}

function Resolve-Python {
    # Prefer 3.12/3.11/3.13: lncrawl v4.14.0 dependencies are best covered by
    # those wheels. Fall back to whatever "py -3" or python resolves to.
    $candidates = @(
        @{ Exe = "py"; Prefix = @("-3.12") },
        @{ Exe = "py"; Prefix = @("-3.11") },
        @{ Exe = "py"; Prefix = @("-3.13") },
        @{ Exe = "py"; Prefix = @("-3") },
        @{ Exe = "python"; Prefix = @() },
        @{ Exe = "python3"; Prefix = @() }
    )
    $probe = "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)"
    foreach ($candidate in $candidates) {
        if (-not (Get-Command $candidate.Exe -ErrorAction SilentlyContinue)) {
            continue
        }
        $checkArgs = $candidate.Prefix + @("-c", $probe)
        & $candidate.Exe @checkArgs 2>$null
        if ($LASTEXITCODE -ne 0) {
            continue
        }
        $versionArgs = $candidate.Prefix + @("-c", "import sys; print(sys.version.split()[0])")
        $version = (& $candidate.Exe @versionArgs 2>$null) -join ""
        Write-Host "    Python $version via $($candidate.Exe) $($candidate.Prefix -join ' ')" -ForegroundColor DarkGray
        return $candidate
    }
    Fail "Python 3.10+ not found. Install it with: winget install -e --id Python.Python.3.12"
}

# ---------------------------------------------------------------------------
# Component selection
# ---------------------------------------------------------------------------
$wantWebnovel = $false
$wantManga = $false
$wantBooks = $false

if (-not $Components -or $Components.Count -eq 0) {
    $Components = @("all")
}
foreach ($component in $Components) {
    switch ($component.ToLowerInvariant()) {
        "webnovel" { $wantWebnovel = $true }
        "lncrawl" { $wantWebnovel = $true }
        "manga" { $wantManga = $true }
        "gallery-dl" { $wantManga = $true }
        "books" { $wantBooks = $true }
        "aget" { $wantBooks = $true }
        "all" {
            $wantWebnovel = $true
            $wantManga = $true
            $wantBooks = $true
        }
        default { Fail "Unknown component: $component (expected webnovel, manga, books or all)" }
    }
}

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
Write-Step "Checking prerequisites"
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Fail "Git not found. Install it with: winget install -e --id Git.Git"
}
$Python = Resolve-Python

foreach ($directory in @($Root, $BinDir, $ToolsDir)) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
}

function New-Venv {
    param([string]$Venv)
    $pythonExe = Join-Path $Venv "Scripts\python.exe"
    if (-not (Test-Path -LiteralPath $pythonExe)) {
        Write-Step "Creating Python virtualenv at $Venv"
        $venvArgs = $Python.Prefix + @("-m", "venv", $Venv)
        Invoke-Native -FilePath $Python.Exe -ArgumentList $venvArgs -Description "python -m venv"
    }
    if (-not (Test-Path -LiteralPath $pythonExe)) {
        Fail "Virtualenv did not create Scripts\python.exe under $Venv"
    }
    return $pythonExe
}

# ---------------------------------------------------------------------------
# lncrawl
# ---------------------------------------------------------------------------
if ($wantWebnovel) {
    Write-Step "Installing lncrawl $LncrawlTag"
    if (-not (Test-Path -LiteralPath (Join-Path $LncrawlDir ".git"))) {
        if (Test-Path -LiteralPath $LncrawlDir) {
            Fail "$LncrawlDir exists but is not a git checkout. Remove it and rerun."
        }
        Invoke-Native -FilePath "git" -Description "git clone lncrawl" -ArgumentList @(
            "-c", "core.autocrlf=false",
            "clone", "--depth", "1", "--branch", $LncrawlTag,
            $LncrawlRepo, $LncrawlDir
        )
    } else {
        Write-Host "    Keeping existing checkout at $LncrawlDir" -ForegroundColor DarkGray
        & git -C $LncrawlDir config core.autocrlf false
    }

    $lncrawlPython = New-Venv -Venv $LncrawlVenv
    Invoke-Native -FilePath $lncrawlPython -Description "pip upgrade" -ArgumentList @(
        "-m", "pip", "install", "--upgrade", "pip"
    )
    Invoke-Native -FilePath $lncrawlPython -Description "lncrawl install" -ArgumentList @(
        "-m", "pip", "install", "--editable", $LncrawlDir
    )

    # The patches are order-sensitive and idempotent. `git apply --reverse
    # --check` proves a patch is already present without touching the tree.
    Write-Step "Applying NovelForge lncrawl patches"
    $patchNames = @(
        "lncrawl-v4.14.0-novelfull-ajax.patch",
        "lncrawl-v4.14.0-novelfull-metadata.patch",
        "lncrawl-v4.14.0-divinedaolibrary-redesign.patch",
        "lncrawl-v4.14.0-novelping-adapter.patch",
        "lncrawl-v4.14.0-novelphoenix-redesign.patch",
        "lncrawl-v4.14.0-source-capabilities.patch",
        "lncrawl-v4.14.0-chapter-selection.patch",
        "lncrawl-v4.14.0-offline-source-index.patch"
    )
    $patchDir = Join-Path $PSScriptRoot "patches"
    foreach ($name in $patchNames) {
        $patch = Join-Path $patchDir $name
        if (-not (Test-Path -LiteralPath $patch)) {
            Fail "Patch not found: $patch"
        }
        & git -C $LncrawlDir apply --reverse --check $patch 2>$null
        if ($LASTEXITCODE -eq 0) {
            Write-Host "    Already applied: $name" -ForegroundColor DarkGray
            continue
        }
        & git -C $LncrawlDir apply --check $patch 2>$null
        if ($LASTEXITCODE -ne 0) {
            Fail "Patch does not apply cleanly: $name (lncrawl version mismatch?)"
        }
        Invoke-Native -FilePath "git" -Description "apply $name" -ArgumentList @(
            "-C", $LncrawlDir, "apply", $patch
        )
        Write-Host "    Applied: $name" -ForegroundColor DarkGray
    }

    $scraperPatch = Join-Path $PSScriptRoot "patch_lncrawl_scraper.py"
    if (-not (Test-Path -LiteralPath $scraperPatch)) {
        Fail "Missing $scraperPatch (run this script from the repository)"
    }
    Invoke-Native -FilePath $lncrawlPython -Description "lncrawl-scraper patch" -ArgumentList @($scraperPatch)
    Invoke-Native -FilePath $lncrawlPython -Description "lncrawl-scraper verification" -ArgumentList @(
        $scraperPatch, "--verify"
    )

    $lncrawlExe = Join-Path $LncrawlVenv "Scripts\lncrawl.exe"
    if (-not (Test-Path -LiteralPath $lncrawlExe)) {
        Fail "lncrawl console script missing from $LncrawlVenv. The editable install did not finish."
    }
    Write-CmdShim -Name "lncrawl" -Target $lncrawlExe
    Write-Host "    Installed: $(Join-Path $BinDir 'lncrawl.cmd')" -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------------
# gallery-dl
# ---------------------------------------------------------------------------
if ($wantManga) {
    Write-Step "Installing gallery-dl $GalleryDlVersion"
    $galleryPython = New-Venv -Venv $GalleryVenv
    Invoke-Native -FilePath $galleryPython -Description "pip upgrade" -ArgumentList @(
        "-m", "pip", "install", "--upgrade", "pip"
    )
    Invoke-Native -FilePath $galleryPython -Description "gallery-dl install" -ArgumentList @(
        "-m", "pip", "install", "gallery-dl==$GalleryDlVersion"
    )

    $galleryExe = Join-Path $GalleryVenv "Scripts\gallery-dl.exe"
    if (-not (Test-Path -LiteralPath $galleryExe)) {
        Fail "gallery-dl console script missing from $GalleryVenv"
    }
    Write-CmdShim -Name "gallery-dl" -Target $galleryExe
    Write-Host "    Installed: $(Join-Path $BinDir 'gallery-dl.cmd')" -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------------
# aget
# ---------------------------------------------------------------------------
if ($wantBooks) {
    Write-Step "Installing aget"
    if (-not (Test-Path -LiteralPath (Join-Path $AgetDir ".git"))) {
        if (Test-Path -LiteralPath $AgetDir) {
            Fail "$AgetDir exists but is not a git checkout. Remove it and rerun."
        }
        Invoke-Native -FilePath "git" -Description "git clone aget" -ArgumentList @(
            "-c", "core.autocrlf=false", "clone", "--depth", "1",
            $AgetRepo, $AgetDir
        )
    } else {
        Write-Host "    Keeping existing checkout at $AgetDir" -ForegroundColor DarkGray
        & git -C $AgetDir config core.autocrlf false
        & git -C $AgetDir pull --ff-only
        if ($LASTEXITCODE -ne 0) {
            Write-Warn "Could not refresh aget; continuing with the existing checkout."
        }
    }

    # NovelForge owns the book engine: this copy is LibGen-first, verifies
    # title/author/ISBN and keeps every browser headless. Copy after the pull so
    # an update cannot silently restore the old behaviour.
    $agetScript = Join-Path (Join-Path $AgetDir "scripts") "aget.py"
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "aget.py") -Destination $agetScript -Force

    $agetPython = New-Venv -Venv $AgetVenv
    Invoke-Native -FilePath $agetPython -Description "pip upgrade" -ArgumentList @(
        "-m", "pip", "install", "--upgrade", "pip"
    )
    Write-Step "Installing aget Python dependencies"
    Invoke-Native -FilePath $agetPython -Description "aget dependencies" -ArgumentList @(
        "-m", "pip", "install",
        "click==$AgetClick",
        "rich==$AgetRich",
        "playwright==$AgetPlaywright",
        "nodriver==$AgetNodriver"
    )

    if ($SkipBrowser) {
        Write-Warn "Skipped Playwright Chromium (-SkipBrowser). Anna's Archive and OceanOfPDF will be skipped; LibGen and WeLib still work."
    } else {
        Write-Step "Installing Playwright Chromium (aget drives a headless browser)"
        Invoke-Native -FilePath $agetPython -Description "playwright install chromium" -ArgumentList @(
            "-m", "playwright", "install", "chromium"
        )
    }

    Write-CmdShim -Name "aget" -Target $agetPython -TargetArgs @($agetScript)
    Write-Host "    Installed: $(Join-Path $BinDir 'aget.cmd')" -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------------
# Verification
# ---------------------------------------------------------------------------
Write-Step "Verifying"
if ($wantWebnovel) {
    $version = (& (Join-Path $BinDir "lncrawl.cmd") version 2>&1 | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0) {
        Fail "lncrawl.cmd did not run. Check the output above."
    }
    Write-Host "    lncrawl    $version" -ForegroundColor DarkGray
}
if ($wantManga) {
    $version = (& (Join-Path $BinDir "gallery-dl.cmd") --version 2>&1 | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0) {
        Fail "gallery-dl.cmd did not run. Check the output above."
    }
    Write-Host "    gallery-dl $version" -ForegroundColor DarkGray
}
if ($wantBooks) {
    $chrome = $null
    $cache = Join-Path $env:LOCALAPPDATA "ms-playwright"
    if (Test-Path -LiteralPath $cache) {
        $chromiumDirs = Get-ChildItem -LiteralPath $cache -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like "chromium-*" } |
            Sort-Object Name -Descending
        foreach ($directory in $chromiumDirs) {
            foreach ($relative in @("chrome-win64\chrome.exe", "chrome-win\chrome.exe")) {
                $candidate = Join-Path $directory.FullName $relative
                if (Test-Path -LiteralPath $candidate) {
                    $chrome = $candidate
                    break
                }
            }
            if ($chrome) { break }
        }
    }
    if ($chrome) {
        Write-Host "    aget       $(Join-Path $BinDir 'aget.cmd')" -ForegroundColor DarkGray
        Write-Host "    chromium   $chrome" -ForegroundColor DarkGray
    } else {
        Write-Host "    aget       $(Join-Path $BinDir 'aget.cmd') (no Chromium: Anna's/Ocean skipped)" -ForegroundColor DarkGray
    }
}

Write-Host ""
Write-Step "Toolchain ready"
Write-Host "NovelForge searches $BinDir directly; no PATH change or reboot is needed."
Write-Host "Restart NovelForge, then open Settings and run the tool check."
