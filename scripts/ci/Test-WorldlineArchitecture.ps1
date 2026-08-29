# FILE: scripts/ci/Test-WorldlineArchitecture.ps1
# VERSION: 1.0.0
# START_MODULE_CONTRACT
#   PURPOSE: Validate repository architecture direction and the canonical GRACE 4 layout.
#   SCOPE: Cargo metadata dependency direction, GRACE files, XML versions, and routed targets.
#   DEPENDS: M-CI-BASELINE, M-GRACE-CONTROL-LAYER, M-KERNEL-CAPABILITY-RUNTIME
#   LINKS: M-CI-BASELINE, M-GRACE-CONTROL-LAYER, M-KERNEL-CAPABILITY-RUNTIME
#   ROLE: SCRIPT
#   MAP_MODE: LOCALS
# END_MODULE_CONTRACT

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path (Join-Path $scriptRoot '..') '..')).Path
$graceRoot = Join-Path $repoRoot '.grace'

function Assert-Condition {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Get-RepositoryPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RelativePath
    )

    return Join-Path $repoRoot $RelativePath
}

function Require-RepositoryPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RelativePath,

        [Parameter(Mandatory = $true)]
        [ValidateSet('Any', 'File', 'Directory')]
        [string]$Kind
    )

    $path = Get-RepositoryPath -RelativePath $RelativePath
    $pathType = switch ($Kind) {
        'File' { 'Leaf' }
        'Directory' { 'Container' }
        default { $null }
    }

    $exists = if ($null -eq $pathType) {
        Test-Path -LiteralPath $path
    } else {
        Test-Path -LiteralPath $path -PathType $pathType
    }

    Assert-Condition $exists "Required $($Kind.ToLowerInvariant()) is missing: ${RelativePath}"
    return $path
}

function Read-RepositoryXml {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RelativePath
    )

    $path = Require-RepositoryPath -RelativePath $RelativePath -Kind File
    try {
        return [xml](Get-Content -LiteralPath $path -Raw)
    } catch {
        throw "Invalid XML in ${RelativePath}: $($_.Exception.Message)"
    }
}

function Assert-GraceVersion {
    param(
        [Parameter(Mandatory = $true)]
        [xml]$Document,

        [Parameter(Mandatory = $true)]
        [string]$RelativePath
    )

    $version = $Document.DocumentElement.GetAttribute('graceVersion')
    Assert-Condition ($version -eq '4.0') "${RelativePath} must declare graceVersion=4.0."
}

function Assert-RoutedXmlIndex {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RelativePath,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedRoot,

        [Parameter(Mandatory = $true)]
        [string]$RouteRoot
    )

    $document = Read-RepositoryXml -RelativePath $RelativePath
    Assert-Condition ($document.DocumentElement.LocalName -eq $ExpectedRoot) "${RelativePath} has unexpected root element."
    Assert-GraceVersion -Document $document -RelativePath $RelativePath

    $routeNodes = @($document.SelectNodes('//Path'))
    Assert-Condition ($routeNodes.Count -gt 0) "${RelativePath} must contain at least one routed Path."

    foreach ($routeNode in $routeNodes) {
        $route = $routeNode.InnerText.Trim()
        Assert-Condition ($route.Length -gt 0) "${RelativePath} contains an empty route."
        $target = Join-Path $RouteRoot $route
        Assert-Condition (Test-Path -LiteralPath $target -PathType Leaf) "${RelativePath} routes to a missing file: ${route}"
    }
}

function Get-CargoMetadata {
    Push-Location -LiteralPath $repoRoot
    try {
        $metadataJson = & cargo metadata --format-version 1 --no-deps --locked
        $metadataExitCode = $LASTEXITCODE
    } catch {
        throw "cargo metadata could not be started: $($_.Exception.Message)"
    } finally {
        Pop-Location
    }

    Assert-Condition ($metadataExitCode -eq 0) "cargo metadata failed with exit code ${metadataExitCode}."

    try {
        return (($metadataJson -join [Environment]::NewLine) | ConvertFrom-Json)
    } catch {
        throw "cargo metadata returned invalid JSON: $($_.Exception.Message)"
    }
}

Write-Host 'Validating canonical GRACE 4 layout.'
foreach ($directory in @('.grace', '.grace/context', '.grace/graph', '.grace/verification')) {
    [void](Require-RepositoryPath -RelativePath $directory -Kind Directory)
}

foreach ($file in @(
        'AGENTS.md',
        'rust-toolchain.toml',
        'docs/grace/README.md',
        '.grace/graph/index.xml',
        '.grace/graph/main.xml',
        '.grace/verification/index.xml',
        '.grace/verification/main.xml'
    )) {
    [void](Require-RepositoryPath -RelativePath $file -Kind File)
}

$contextDirectory = Get-RepositoryPath -RelativePath '.grace/context'
$contextFiles = @(Get-ChildItem -LiteralPath $contextDirectory -Filter '*.xml' -File)
Assert-Condition ($contextFiles.Count -gt 0) 'The GRACE context directory must contain at least one XML artifact.'
foreach ($contextFile in $contextFiles) {
    $contextRelativePath = [IO.Path]::GetRelativePath($repoRoot, $contextFile.FullName)
    $contextDocument = Read-RepositoryXml -RelativePath $contextRelativePath
    Assert-GraceVersion -Document $contextDocument -RelativePath $contextRelativePath
}

Assert-RoutedXmlIndex -RelativePath '.grace/graph/index.xml' -ExpectedRoot 'GraceGraphIndex' -RouteRoot $graceRoot
Assert-RoutedXmlIndex -RelativePath '.grace/verification/index.xml' -ExpectedRoot 'GraceVerificationIndex' -RouteRoot $graceRoot

$graphDocument = Read-RepositoryXml -RelativePath '.grace/graph/main.xml'
Assert-Condition ($graphDocument.DocumentElement.LocalName -eq 'GraceGraphDocument') '.grace/graph/main.xml has unexpected root element.'
Assert-GraceVersion -Document $graphDocument -RelativePath '.grace/graph/main.xml'

$verificationDocument = Read-RepositoryXml -RelativePath '.grace/verification/main.xml'
Assert-Condition ($verificationDocument.DocumentElement.LocalName -eq 'GraceVerificationDocument') '.grace/verification/main.xml has unexpected root element.'
Assert-GraceVersion -Document $verificationDocument -RelativePath '.grace/verification/main.xml'

Write-Host 'Reading Cargo metadata and checking kernel dependency direction.'
$metadata = Get-CargoMetadata
$workspaceMemberIds = @($metadata.workspace_members | ForEach-Object { [string]$_ })
$workspacePackages = @($metadata.packages | Where-Object { $workspaceMemberIds -contains ([string]$_.id) })
$kernelPackages = @($workspacePackages | Where-Object { $_.name -eq 'worldline-kernel' })

Assert-Condition ($kernelPackages.Count -eq 1) 'Cargo metadata must contain exactly one worldline-kernel workspace package.'

$kernelPackage = $kernelPackages[0]
$otherWorkspaceNames = @(
    $workspacePackages |
        Where-Object { $_.name -ne 'worldline-kernel' } |
        ForEach-Object { [string]$_.name }
)
$forbiddenDependencies = @(
    foreach ($dependency in @($kernelPackage.dependencies)) {
        $dependencyName = [string]$dependency.name
        if ($otherWorkspaceNames -contains $dependencyName) {
            $dependencyName
        }
    }
)
$forbiddenDependencies = @($forbiddenDependencies | Sort-Object -Unique)

Assert-Condition ($forbiddenDependencies.Count -eq 0) ("worldline-kernel must not depend on another Worldline workspace crate: " + ($forbiddenDependencies -join ', '))

$storagePackages = @($workspacePackages | Where-Object { $_.name -eq 'worldline-storage' })
Assert-Condition ($storagePackages.Count -eq 1) 'Cargo metadata must contain exactly one worldline-storage workspace package.'
$storagePackage = $storagePackages[0]
$storageDependencyNames = @($storagePackage.dependencies | ForEach-Object { [string]$_.name })
Assert-Condition ($storageDependencyNames -contains 'worldline-kernel') 'worldline-storage must depend on worldline-kernel contracts.'

$kernelManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-kernel/Cargo.toml') -Raw
foreach ($forbiddenToken in @('worldline-storage', 'rusqlite', 'sha2', 'SqliteStateBackend')) {
    Assert-Condition (-not $kernelManifest.Contains($forbiddenToken)) "worldline-kernel manifest must not mention ${forbiddenToken}."
}
$kernelSourceFiles = @(Get-ChildItem -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-kernel/src') -Filter '*.rs' -File -Recurse)
foreach ($sourceFile in $kernelSourceFiles) {
    $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
    foreach ($forbiddenToken in @('rusqlite', 'SqliteStateBackend', 'worldline_storage')) {
        Assert-Condition (-not $sourceText.Contains($forbiddenToken)) "Kernel source '$($sourceFile.Name)' must not mention ${forbiddenToken}."
    }
}

$storageManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-storage/Cargo.toml') -Raw
Assert-Condition ($storageManifest.Contains('test-failpoints = []')) 'Storage failpoints must be an opt-in feature.'
Assert-Condition (-not $storageManifest.Contains('default = ["test-failpoints"]')) 'Storage failpoints must not be enabled by default.'
$storageSourceFiles = @(Get-ChildItem -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-storage/src') -Filter '*.rs' -File -Recurse)
foreach ($sourceFile in $storageSourceFiles) {
    $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
    Assert-Condition (-not $sourceText.Contains('InMemoryStateBackend')) "Storage source '$($sourceFile.Name)' must not provide an in-memory fallback."
}

Write-Host 'Architecture guard passed: GRACE layout and kernel dependency direction are valid.'
