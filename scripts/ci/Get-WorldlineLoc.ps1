# FILE: scripts/ci/Get-WorldlineLoc.ps1
# VERSION: 1.0.0
# START_MODULE_CONTRACT
#   PURPOSE: Calculate and report repository lines-of-code (LOC) statistics across crates, tests, contracts, and documentation.
#   SCOPE: Categorized line metrics, console table formatting, and GitHub Step Summary export.
#   DEPENDS: M-CI-BASELINE
#   LINKS: M-CI-BASELINE
#   ROLE: SCRIPT
#   MAP_MODE: LOCALS
# END_MODULE_CONTRACT

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path (Join-Path $scriptRoot '..') '..')).Path

function Measure-FileLines {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,

        [Parameter(Mandatory = $true)]
        [string]$Category
    )

    $extension = [System.IO.Path]::GetExtension($FilePath).ToLowerInvariant()
    $lines = [System.IO.File]::ReadAllLines($FilePath)
    $total = $lines.Count
    $blank = 0
    $comment = 0
    $code = 0

    $inBlockComment = $false

    foreach ($rawLine in $lines) {
        $trimmed = $rawLine.Trim()
        if ($trimmed.Length -eq 0) {
            $blank++
            continue
        }

        if ($extension -eq '.rs' -or $extension -eq '.wit') {
            if ($inBlockComment) {
                $comment++
                if ($trimmed.Contains('*/')) {
                    $inBlockComment = $false
                }
                continue
            }
            if ($trimmed.StartsWith('/*')) {
                $comment++
                if (-not $trimmed.Contains('*/')) {
                    $inBlockComment = $true
                }
                continue
            }
            if ($trimmed.StartsWith('//')) {
                $comment++
                continue
            }
            $code++
        } elseif ($extension -eq '.ps1' -or $extension -eq '.toml' -or $extension -eq '.yml' -or $extension -eq '.yaml') {
            if ($trimmed.StartsWith('#')) {
                $comment++
                continue
            }
            $code++
        } elseif ($extension -eq '.xml' -or $extension -eq '.html') {
            if ($inBlockComment) {
                $comment++
                if ($trimmed.Contains('-->')) {
                    $inBlockComment = $false
                }
                continue
            }
            if ($trimmed.StartsWith('<!--')) {
                $comment++
                if (-not $trimmed.Contains('-->')) {
                    $inBlockComment = $true
                }
                continue
            }
            $code++
        } else {
            # Default (e.g. Markdown, JSON, plain text)
            $code++
        }
    }

    [PSCustomObject]@{
        Category = $Category
        FilePath = [System.IO.Path]::GetRelativePath($repoRoot, $FilePath)
        Total    = $total
        Code     = $code
        Comment  = $comment
        Blank    = $blank
    }
}

$categories = [ordered]@{
    'Rust Production (Kernel/Storage/Hosts/Demo)' = @{
        Root    = 'crates'
        Include = @('*/src/**/*.rs')
        Exclude = @('*/src/bin/**')
    }
    'Rust Acceptance & Hardening Tests'           = @{
        Root    = 'crates'
        Include = @('*/tests/**/*.rs', '*/src/bin/**/*.rs', '*/tests/*.rs')
        Exclude = @()
    }
    'Protocol Vocabulary, WIT & Fixtures'         = @{
        Root    = 'crates/worldline-plugin-protocol'
        Include = @('wit/**/*.wit', 'fixtures/**/*.json')
        Exclude = @()
    }
    'CI Scripts & Automation'                     = @{
        Root    = 'scripts'
        Include = @('**/*.ps1')
        Exclude = @()
    }
    'GitHub Workflows'                            = @{
        Root    = '.github'
        Include = @('**/*.yml', '**/*.md')
        Exclude = @()
    }
    'GRACE 4 Engineering Projections'             = @{
        Root    = '.grace'
        Include = @('**/*.xml')
        Exclude = @()
    }
    'Architecture & Living Documentation'         = @{
        Root    = 'docs'
        Include = @('**/*.md', '**/*.xml')
        Exclude = @()
    }
}

$results = [System.Collections.Generic.List[PSObject]]::new()

foreach ($catName in $categories.Keys) {
    $def = $categories[$catName]
    $searchDir = Join-Path $repoRoot $def.Root
    if (-not (Test-Path -LiteralPath $searchDir)) {
        continue
    }

    $files = Get-ChildItem -LiteralPath $searchDir -File -Recurse
    foreach ($file in $files) {
        $rel = [System.IO.Path]::GetRelativePath($repoRoot, $file.FullName)
        
        # Check exclusion
        $excluded = $false
        if ($catName -eq 'Rust Production (Kernel/Storage/Hosts/Demo)') {
            if ($rel -replace '\\', '/' -match '/src/bin/') {
                $excluded = $true
            }
        }
        if ($catName -eq 'Rust Acceptance & Hardening Tests') {
            if (-not ($rel -replace '\\', '/' -match '/tests/' -or $rel -replace '\\', '/' -match '/src/bin/')) {
                $excluded = $true
            }
        }
        if ($excluded) {
            continue
        }

        $ext = $file.Extension.ToLowerInvariant()
        $validExt = switch ($catName) {
            'Rust Production (Kernel/Storage/Hosts/Demo)' { $ext -eq '.rs' }
            'Rust Acceptance & Hardening Tests'           { $ext -eq '.rs' }
            'Protocol Vocabulary, WIT & Fixtures'         { $ext -eq '.wit' -or $ext -eq '.json' }
            'CI Scripts & Automation'                     { $ext -eq '.ps1' }
            'GitHub Workflows'                            { $ext -eq '.yml' -or $ext -eq '.md' }
            'GRACE 4 Engineering Projections'             { $ext -eq '.xml' }
            'Architecture & Living Documentation'         { $ext -eq '.md' -or $ext -eq '.xml' }
            default { $false }
        }

        if ($validExt) {
            $results.Add((Measure-FileLines -FilePath $file.FullName -Category $catName))
        }
    }
}

# Root-level docs
foreach ($rootDoc in @('ROADMAP.md', 'AGENTS.md', 'README.md', 'rust-toolchain.toml', 'Cargo.toml')) {
    $fullPath = Join-Path $repoRoot $rootDoc
    if (Test-Path -LiteralPath $fullPath) {
        $results.Add((Measure-FileLines -FilePath $fullPath -Category 'Root Governance & Project Manifests'))
    }
}

$summary = $results | Group-Object Category | ForEach-Object {
    [PSCustomObject]@{
        Category = $_.Name
        Files    = [int]$_.Count
        Code     = [int64]($_.Group | Measure-Object -Property Code -Sum).Sum
        Comment  = [int64]($_.Group | Measure-Object -Property Comment -Sum).Sum
        Blank    = [int64]($_.Group | Measure-Object -Property Blank -Sum).Sum
        Total    = [int64]($_.Group | Measure-Object -Property Total -Sum).Sum
    }
}

$totalFiles = [int]($summary | Measure-Object -Property Files -Sum).Sum
$totalCode = [int64]($summary | Measure-Object -Property Code -Sum).Sum
$totalComment = [int64]($summary | Measure-Object -Property Comment -Sum).Sum
$totalBlank = [int64]($summary | Measure-Object -Property Blank -Sum).Sum
$totalLines = [int64]($summary | Measure-Object -Property Total -Sum).Sum

Write-Host ''
Write-Host '============================== WORLDLINE LINES OF CODE (LOC) =============================='
$summary | Format-Table -AutoSize | Out-String | Write-Host
Write-Host ('-' * 90)
Write-Host ("TOTAL FILES: {0,5} | CODE: {1,7} | COMMENTS: {2,6} | BLANKS: {3,6} | TOTAL LINES: {4,7}" -f $totalFiles, $totalCode, $totalComment, $totalBlank, $totalLines)
Write-Host '==========================================================================================='
Write-Host ''

# Write to GitHub Step Summary if running in GitHub Actions
if ($env:GITHUB_STEP_SUMMARY -and (Test-Path (Split-Path -Parent $env:GITHUB_STEP_SUMMARY))) {
    $md = [System.Text.StringBuilder]::new()
    [void]$md.AppendLine('### 📊 Worldline Lines of Code (LOC) Summary')
    [void]$md.AppendLine('')
    [void]$md.AppendLine('| Category | Files | Code Lines | Comments | Blanks | Total Lines |')
    [void]$md.AppendLine('| :--- | :---: | :---: | :---: | :---: | :---: |')
    foreach ($row in $summary) {
        [void]$md.AppendLine("| $($row.Category) | $($row.Files) | $($row.Code) | $($row.Comment) | $($row.Blank) | $($row.Total) |")
    }
    [void]$md.AppendLine("| **TOTAL** | **$totalFiles** | **$totalCode** | **$totalComment** | **$totalBlank** | **$totalLines** |")
    [void]$md.AppendLine('')
    [System.IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $md.ToString())
}
