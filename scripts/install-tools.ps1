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
# The eight patches in scripts/patches/ were generated against this commit on
# `dev`. It still reports 4.14.0 but already contains the
# `create_crawler_info` refactor the source-capabilities patch depends on; tag
# `v4.14.0` is older and the last three patches do not apply to it.
$LncrawlCommit = "59b0382d51927953aa8120c5de62dab23ce3f731"
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

# Windows PowerShell 5.1 converts a native command's stderr into an ErrorRecord
# as soon as the stream is redirected, and $ErrorActionPreference = "Stop" then
# aborts the script. Probes intentionally produce stderr and non-zero exits
# ("patch does not apply", "no suitable Python runtime"), so they run under
# Continue and keep both streams to themselves.
function Invoke-Probe {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$ArgumentList = @()
    )
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $FilePath @ArgumentList 2>&1
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 0 }
        return [pscustomobject]@{ ExitCode = $code; Output = @($output) }
    } finally {
        $ErrorActionPreference = $previous
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
        $check = Invoke-Probe -FilePath $candidate.Exe -ArgumentList $checkArgs
        if ($check.ExitCode -ne 0) {
            continue
        }
        $versionArgs = $candidate.Prefix + @("-c", "import sys; print(sys.version.split()[0])")
        $version = ((Invoke-Probe -FilePath $candidate.Exe -ArgumentList $versionArgs).Output -join "").Trim()
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
    Write-Step "Installing lncrawl $($LncrawlCommit.Substring(0, 10))"
    if (-not (Test-Path -LiteralPath (Join-Path $LncrawlDir ".git"))) {
        if (Test-Path -LiteralPath $LncrawlDir) {
            Fail "$LncrawlDir exists but is not a git checkout. Remove it and rerun."
        }
        Invoke-Native -FilePath "git" -Description "git init lncrawl" -ArgumentList @(
            "init", "-q", $LncrawlDir
        )
        Invoke-Native -FilePath "git" -Description "git remote lncrawl" -ArgumentList @(
            "-C", $LncrawlDir, "remote", "add", "origin", $LncrawlRepo
        )
        Invoke-Native -FilePath "git" -Description "git fetch lncrawl" -ArgumentList @(
            "-C", $LncrawlDir, "-c", "core.autocrlf=false",
            "fetch", "--depth", "1", "origin", $LncrawlCommit
        )
        Invoke-Native -FilePath "git" -Description "git checkout lncrawl" -ArgumentList @(
            "-C", $LncrawlDir, "checkout", "-q", "FETCH_HEAD"
        )
    } else {
        # Without this, a checkout made while autocrlf was true keeps CRLF and
        # every patch is rejected. Set the config first, then rewrite the tree.
        & git -C $LncrawlDir config core.autocrlf false
        $headResult = Invoke-Probe -FilePath "git" -ArgumentList @(
            "-C", $LncrawlDir, "rev-parse", "HEAD"
        )
        $head = ($headResult.Output -join "").Trim()
        $target = "HEAD"
        if ($head -ne $LncrawlCommit) {
            $short = $head.Substring(0, [Math]::Min(10, $head.Length))
            Write-Warn "lncrawl checkout is at $short; moving to the pinned commit."
            Invoke-Native -FilePath "git" -Description "git fetch lncrawl" -ArgumentList @(
                "-C", $LncrawlDir, "-c", "core.autocrlf=false",
                "fetch", "--depth", "1", "origin", $LncrawlCommit
            )
            $target = "FETCH_HEAD"
        }
        # The tree only ever holds upstream code plus our patches, which are
        # re-applied below; discarding local edits is safe and also normalises
        # line endings.
        Invoke-Native -FilePath "git" -Description "git reset lncrawl" -ArgumentList @(
            "-C", $LncrawlDir, "reset", "-q", "--hard", $target
        )
        Write-Host "    Checkout: $LncrawlDir" -ForegroundColor DarkGray
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
    # The installer's own commit is the first question in any patch failure:
    # an old copy of this script cannot normalise anything.
    $installerProbe = Invoke-Probe -FilePath "git" -ArgumentList @(
        "-C", $PSScriptRoot, "rev-parse", "--short", "HEAD"
    )
    $installerCommit = if ($installerProbe.ExitCode -eq 0) {
        ($installerProbe.Output -join "").Trim()
    } else {
        "unknown (not a git checkout)"
    }
    $lncrawlHead = ((Invoke-Probe -FilePath "git" -ArgumentList @(
        "-C", $LncrawlDir, "rev-parse", "HEAD"
    )).Output -join "").Trim()
    $lncrawlShort = $lncrawlHead.Substring(0, [Math]::Min(10, $lncrawlHead.Length))
    Write-Host "    installer $installerCommit | lncrawl $lncrawlShort" -ForegroundColor DarkGray

    # A Windows clone with core.autocrlf=true checks these files out with CRLF,
    # and git then rejects every hunk against the LF lncrawl checkout. Write an
    # LF copy to a scratch directory and apply that instead.
    $patchTemp = Join-Path $env:TEMP ("novelforge-patches-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $patchTemp | Out-Null
    foreach ($name in $patchNames) {
        $source = Join-Path $patchDir $name
        if (-not (Test-Path -LiteralPath $source)) {
            Fail "Patch not found: $source"
        }
        $patch = Join-Path $patchTemp $name
        $raw = [System.IO.File]::ReadAllText($source)
        $patchHasCr = $raw.Contains("`r")
        $text = $raw -replace "`r`n", "`n"
        [System.IO.File]::WriteAllText($patch, $text)

        $reverse = Invoke-Probe -FilePath "git" -ArgumentList @(
            "-C", $LncrawlDir, "apply", "--reverse", "--check", "--quiet", $patch
        )
        if ($reverse.ExitCode -eq 0) {
            Write-Host "    Already applied: $name" -ForegroundColor DarkGray
            continue
        }

        # No --quiet: the error text is the diagnostic when this fails.
        $applyFlags = @()
        $forward = Invoke-Probe -FilePath "git" -ArgumentList @(
            @("-C", $LncrawlDir, "apply", "--check") + $applyFlags + @($patch)
        )
        if ($forward.ExitCode -ne 0) {
            # Second chance: some checkouts still produce whitespace-only
            # differences (CRLF from an older clone, smart quotes in an editor).
            $relaxed = Invoke-Probe -FilePath "git" -ArgumentList @(
                "-C", $LncrawlDir, "apply", "--check", "--ignore-whitespace", $patch
            )
            if ($relaxed.ExitCode -eq 0) {
                $applyFlags = @("--ignore-whitespace")
                Write-Warn "$name needed --ignore-whitespace; the checkout's line endings are not clean."
            } else {
                $detail = ($relaxed.Output | Select-Object -Last 5) -join "`n    "
                $eol = ((Invoke-Probe -FilePath "git" -ArgumentList @(
                    "-C", $LncrawlDir, "ls-files", "--eol", "sources/en/n/novelfull.py"
                )).Output -join "").Trim()
                Fail @"
Patch does not apply cleanly: $name
    installer:    $installerCommit
    lncrawl HEAD: $lncrawlHead (pinned $LncrawlCommit)
    patch CR:     $patchHasCr
    target EOL:   $eol
    git:          $detail
"@
            }
        }
        Invoke-Native -FilePath "git" -Description "apply $name" -ArgumentList @(
            @("-C", $LncrawlDir, "apply") + $applyFlags + @($patch)
        )
        Write-Host "    Applied: $name" -ForegroundColor DarkGray
    }
    Remove-Item -LiteralPath $patchTemp -Recurse -Force -ErrorAction SilentlyContinue

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
        $pull = Invoke-Probe -FilePath "git" -ArgumentList @("-C", $AgetDir, "pull", "--ff-only")
        if ($pull.ExitCode -ne 0) {
            Write-Warn "Could not refresh aget; continuing with the existing checkout."
            Write-Host ("    " + (($pull.Output | Select-Object -Last 3) -join " ")) -ForegroundColor DarkGray
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
    $lncrawl = Invoke-Probe -FilePath (Join-Path $BinDir "lncrawl.cmd") -ArgumentList @("version")
    if ($lncrawl.ExitCode -ne 0) {
        Fail "lncrawl.cmd did not run (exit $($lncrawl.ExitCode)). Check the output above."
    }
    $version = ($lncrawl.Output | Select-Object -First 1)
    Write-Host "    lncrawl    $version" -ForegroundColor DarkGray
}
if ($wantManga) {
    $gallery = Invoke-Probe -FilePath (Join-Path $BinDir "gallery-dl.cmd") -ArgumentList @("--version")
    if ($gallery.ExitCode -ne 0) {
        Fail "gallery-dl.cmd did not run (exit $($gallery.ExitCode)). Check the output above."
    }
    $version = ($gallery.Output | Select-Object -First 1)
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
