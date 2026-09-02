# FILE: scripts/ci/Initialize-WorldlineCef.ps1
# VERSION: 1.0.0
# START_MODULE_CONTRACT
#   PURPOSE: Verify and stage the repository-pinned Windows CEF runtime for real S3B.
#   SCOPE: CEF archive identity, extracted runtime, Ninja discovery, and native provider client staging.
#   DEPENDS: M-CI-BASELINE, M-BROWSER-ENGINE-PROVIDER-PROCESS
#   LINKS: M-CI-BASELINE, M-BROWSER-ENGINE-PROVIDER-PROCESS, M-BROWSER-SERVICE-PLUGINS
#   ROLE: SCRIPT
#   MAP_MODE: LOCALS
# END_MODULE_CONTRACT

[CmdletBinding()]
param(
    [Parameter()]
    [string]$CacheRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($env:OS -ne 'Windows_NT') {
    throw 'The pinned CEF runtime is supported only on the hosted Windows target.'
}

$scriptRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path (Join-Path $scriptRoot '..') '..')).Path
$manifestPath = Join-Path $repoRoot 'crates\worldline-browser-cef\cef-runtime.manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json

if ($manifest.target -ne 'x86_64-pc-windows-msvc') {
    throw "CEF manifest target '$($manifest.target)' is not the supported Windows target."
}
if (-not $manifest.runtime_policy.sandbox_required -or -not $manifest.runtime_policy.headful_required) {
    throw 'CEF manifest must require both sandboxing and headful execution.'
}
if ($manifest.runtime_policy.allow_ambient_installation -or $manifest.runtime_policy.no_sandbox) {
    throw 'CEF manifest permits an ambient installation or sandbox bypass.'
}

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:WORLDLINE_CEF_CACHE_ROOT)) {
        $CacheRoot = $env:WORLDLINE_CEF_CACHE_ROOT
    } elseif (-not [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
        $CacheRoot = Join-Path $env:RUNNER_TEMP 'worldline-cef-runtime'
    } else {
        $CacheRoot = Join-Path $repoRoot 'target\cef-runtime-cache'
    }
}

$cacheRootFull = [IO.Path]::GetFullPath($CacheRoot)
New-Item -ItemType Directory -Force -Path $cacheRootFull | Out-Null
$archivePath = Join-Path $cacheRootFull $manifest.distribution.archive
$stagedRoot = Join-Path $cacheRootFull 'cef-staged-windows64'
$archiveRecordPath = Join-Path $stagedRoot 'archive.json'

if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
    Write-Host "Downloading the exact CEF archive declared by the manifest..."
    Invoke-WebRequest -Uri $manifest.distribution.url -OutFile $archivePath
}

$actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
$expectedHash = ([string]$manifest.distribution.sha256).ToLowerInvariant()
if ($actualHash -ne $expectedHash) {
    throw "CEF archive SHA-256 mismatch. Expected '$expectedHash', got '$actualHash'."
}

function Test-StagedRuntime {
    if (-not (Test-Path -LiteralPath $stagedRoot -PathType Container)) {
        return $false
    }
    if (-not (Test-Path -LiteralPath $archiveRecordPath -PathType Leaf)) {
        return $false
    }
    $record = Get-Content -LiteralPath $archiveRecordPath -Raw | ConvertFrom-Json
    if ($record.PSObject.Properties.Name -notcontains 'sha256') {
        return $false
    }
    if ($record.sha256 -ne $actualHash -or $record.archive -ne $manifest.distribution.archive) {
        return $false
    }
    foreach ($required in @(
        'bootstrapc.exe',
        'bootstrap.exe',
        'libcef.dll',
        'icudtl.dat',
        'resources.pak',
        'chrome_100_percent.pak',
        'chrome_200_percent.pak',
        'v8_context_snapshot.bin'
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $stagedRoot $required) -PathType Leaf)) {
            return $false
        }
    }
    return $true
}

if (-not (Test-StagedRuntime)) {
    if (Test-Path -LiteralPath $stagedRoot) {
        Remove-Item -LiteralPath $stagedRoot -Recurse -Force
    }
    $extractRoot = Join-Path $cacheRootFull 'cef-extract'
    if (Test-Path -LiteralPath $extractRoot) {
        Remove-Item -LiteralPath $extractRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
    if ($null -eq $tar) {
        throw 'tar.exe is required to extract the pinned CEF archive.'
    }
    & $tar.Source -xjf $archivePath -C $extractRoot
    if ($LASTEXITCODE -ne 0) {
        throw "CEF archive extraction failed with exit code $LASTEXITCODE."
    }
    $bootstrap = Get-ChildItem -LiteralPath $extractRoot -Filter 'bootstrapc.exe' -Recurse -File |
        Select-Object -First 1
    if ($null -eq $bootstrap) {
        throw 'The pinned CEF archive does not contain bootstrapc.exe.'
    }
    $runtimeSource = $bootstrap.Directory.Parent.FullName
    New-Item -ItemType Directory -Force -Path $stagedRoot | Out-Null
    $releaseRoot = Join-Path $runtimeSource 'Release'
    $resourcesRoot = Join-Path $runtimeSource 'Resources'
    $includeRoot = Join-Path $runtimeSource 'include'
    foreach ($requiredRoot in @($releaseRoot, $resourcesRoot, $includeRoot)) {
        if (-not (Test-Path -LiteralPath $requiredRoot -PathType Container)) {
            throw "The pinned CEF archive is missing '$requiredRoot'."
        }
    }
    Get-ChildItem -LiteralPath $releaseRoot -Force |
        Copy-Item -Destination $stagedRoot -Recurse -Force
    Get-ChildItem -LiteralPath $resourcesRoot -Force |
        Copy-Item -Destination $stagedRoot -Recurse -Force
    Copy-Item -LiteralPath $includeRoot -Destination $stagedRoot -Recurse -Force
    $record = [ordered]@{
        archive = $manifest.distribution.archive
        sha256 = $actualHash
        cef_version = $manifest.cef_version
        target = $manifest.target
    }
    $record | ConvertTo-Json | Set-Content -LiteralPath $archiveRecordPath -Encoding utf8
}

if (-not (Test-StagedRuntime)) {
    throw 'The staged CEF runtime failed its required-file or identity checks.'
}

$ninja = Get-Command ninja.exe -ErrorAction SilentlyContinue
if ($null -eq $ninja) {
    throw 'Ninja is required by the pinned cef-rs build and was not found in PATH.'
}

$env:CEF_PATH = $stagedRoot
$env:WORLDLINE_CEF_RUNTIME_ROOT = $stagedRoot
$env:Path = "$($ninja.Source | Split-Path -Parent);$env:Path"

Write-Host 'Building the native provider client against the verified CEF runtime...'
& cargo build -p worldline-browser-provider-process --lib
if ($LASTEXITCODE -ne 0) {
    throw "worldline-browser-provider-process client build failed with exit code $LASTEXITCODE."
}

$metadataJson = & cargo metadata --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) {
    throw 'cargo metadata failed while locating the native provider client.'
}
$metadata = $metadataJson | ConvertFrom-Json
$targetDirectory = [string]$metadata.target_directory
$clientPath = Join-Path $targetDirectory 'debug\worldline_browser_provider_client.dll'
if (-not (Test-Path -LiteralPath $clientPath -PathType Leaf)) {
    throw "Native provider client was not produced at '$clientPath'."
}
$stagedClientPath = Join-Path $stagedRoot 'worldline_browser_provider_client.dll'
Copy-Item -LiteralPath $clientPath -Destination $stagedClientPath -Force

$bootstrapPath = Join-Path $stagedRoot 'bootstrapc.exe'
if (-not (Test-Path -LiteralPath $bootstrapPath -PathType Leaf)) {
    throw 'The staged CEF bootstrap executable is missing after client staging.'
}
$env:WORLDLINE_BROWSER_PROVIDER_BOOTSTRAP = $bootstrapPath
$env:WORLDLINE_BROWSER_PROVIDER_CLIENT = $stagedClientPath

if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
    @(
        "CEF_PATH=$stagedRoot"
        "WORLDLINE_CEF_RUNTIME_ROOT=$stagedRoot"
        "WORLDLINE_BROWSER_PROVIDER_BOOTSTRAP=$bootstrapPath"
        "WORLDLINE_BROWSER_PROVIDER_CLIENT=$stagedClientPath"
    ) | Add-Content -LiteralPath $env:GITHUB_ENV -Encoding utf8
}
if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_PATH)) {
    $stagedRoot | Add-Content -LiteralPath $env:GITHUB_PATH -Encoding utf8
}

Write-Host "Verified CEF $($manifest.cef_version) from SHA-256 $actualHash."
Write-Host "Staged runtime: $stagedRoot"
Write-Host "Staged provider client: $stagedClientPath"
