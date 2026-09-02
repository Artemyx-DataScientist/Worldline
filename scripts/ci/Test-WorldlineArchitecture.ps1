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
        'scripts/ci/Initialize-WorldlineCi.ps1',
        'scripts/ci/Get-WorldlineLoc.ps1',
        'scripts/ci/Invoke-WorldlineCi.ps1',
        'scripts/ci/Test-WorldlineArchitecture.ps1',
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

Write-Host 'Checking external plugin boundary dependency direction.'
$kernelManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-kernel/Cargo.toml') -Raw
foreach ($forbiddenToken in @(
        'wasmtime',
        'wit-bindgen',
        'worldline-plugin-protocol',
        'worldline-native-host',
        'worldline-wasm-host',
        'worldline-reference-external'
    )) {
    Assert-Condition (-not $kernelManifest.Contains($forbiddenToken)) "worldline-kernel manifest must not mention ${forbiddenToken}."
}
$kernelSourceFiles = @(Get-ChildItem -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-kernel/src') -Filter '*.rs' -File -Recurse)
foreach ($sourceFile in $kernelSourceFiles) {
    $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
    foreach ($forbiddenToken in @('wasmtime', 'wit_bindgen', 'serde_json')) {
        Assert-Condition (-not $sourceText.Contains($forbiddenToken)) "Kernel source '$($sourceFile.Name)' must not mention ${forbiddenToken}."
    }
}

foreach ($adapterCrate in @('worldline-native-host', 'worldline-wasm-host')) {
    $adapterManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath "crates/${adapterCrate}/Cargo.toml") -Raw
    Assert-Condition ($adapterManifest.Contains('worldline-plugin-protocol')) "${adapterCrate} must translate the shared protocol vocabulary."
    Assert-Condition (-not $adapterManifest.Contains('worldline-kernel')) "${adapterCrate} must stay transport-only and must not depend on worldline-kernel."
}

$referenceExternalManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-reference-external/Cargo.toml') -Raw
Assert-Condition ($referenceExternalManifest.Contains('worldline-kernel')) 'worldline-reference-external must bridge adapters to kernel contracts.'

foreach ($consumerContractCrate in @('worldline-kernel', 'worldline-reference-external')) {
    $consumerSourceRoot = Get-RepositoryPath -RelativePath "crates/${consumerContractCrate}/src"
    $consumerSourceFiles = @(Get-ChildItem -LiteralPath $consumerSourceRoot -Filter '*.rs' -File -Recurse)
    foreach ($sourceFile in $consumerSourceFiles) {
        $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
        foreach ($forbiddenToken in @('wasmtime::', 'StoreLimits', 'ComponentLinker')) {
            Assert-Condition (-not $sourceText.Contains($forbiddenToken)) "Consumer contract source '$($sourceFile.Name)' must not reference WASM runtime types."
        }
    }
}

Write-Host 'Checking browser contract and engine anti-corruption boundaries.'
[void](Require-RepositoryPath -RelativePath 'docs/adr/ADR-BROWSER-ENGINE-V1.md' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-contract/Cargo.toml' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-spike/Cargo.toml' -Kind File)

$kernelManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-kernel/Cargo.toml') -Raw
foreach ($forbiddenToken in @('worldline-browser-contract', 'worldline-browser-spike', 'cef', 'chromium', 'webview')) {
    Assert-Condition (-not $kernelManifest.Contains($forbiddenToken)) "worldline-kernel manifest must not mention browser/engine token '${forbiddenToken}'."
}

$kernelSourceFiles = @(Get-ChildItem -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-kernel/src') -Filter '*.rs' -File -Recurse)
foreach ($sourceFile in $kernelSourceFiles) {
    $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
    foreach ($forbiddenToken in @('worldline_browser_contract', 'worldline_browser_spike', 'cef', 'chromium')) {
        Assert-Condition (-not $sourceText.Contains($forbiddenToken)) "Kernel source '$($sourceFile.Name)' must not mention '${forbiddenToken}'."
    }
}

$browserContractManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-contract/Cargo.toml') -Raw
foreach ($forbiddenToken in @('cef', 'chromium', 'webview2', 'wpe', 'webkit', 'gecko', 'servo')) {
    Assert-Condition (-not $browserContractManifest.Contains($forbiddenToken)) "worldline-browser-contract manifest must not depend on engine token '${forbiddenToken}'."
}

$browserContractSourceFiles = @(Get-ChildItem -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-contract/src') -Filter '*.rs' -File -Recurse)
foreach ($sourceFile in $browserContractSourceFiles) {
    $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
    foreach ($forbiddenToken in @('cef', 'chromium', 'webview2', 'wpewebkit')) {
        Assert-Condition (-not $sourceText.ToLowerInvariant().Contains($forbiddenToken)) "Browser contract source '$($sourceFile.Name)' must not reference engine '${forbiddenToken}'."
    }
}

$browserSpikeManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-spike/Cargo.toml') -Raw
Assert-Condition ($browserSpikeManifest.Contains('worldline-browser-contract')) 'worldline-browser-spike must depend on worldline-browser-contract.'
Assert-Condition ($browserSpikeManifest.Contains('worldline-kernel')) 'worldline-browser-spike must depend on worldline-kernel.'

Write-Host 'Checking browser services anti-corruption and dependency isolation boundaries.'
[void](Require-RepositoryPath -RelativePath 'docs/adr/ADR-BROWSER-SERVICE-PLUGINS-V1.md' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-services-contract/Cargo.toml' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-tabs/Cargo.toml' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-history/Cargo.toml' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-downloads/Cargo.toml' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-cookies/Cargo.toml' -Kind File)

foreach ($forbiddenToken in @('worldline-browser-services-contract', 'worldline-browser-tabs', 'worldline-browser-history', 'worldline-browser-downloads', 'worldline-browser-cookies', 'TabId', 'HistoryEntryId', 'DownloadRecordId', 'CookieMetadata')) {
    Assert-Condition (-not $kernelManifest.Contains($forbiddenToken)) "worldline-kernel manifest must not mention service token '${forbiddenToken}'."
}

$servicesContractManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-services-contract/Cargo.toml') -Raw
foreach ($forbiddenToken in @('worldline-browser-cef', 'cef', 'chromium', 'cdp', 'wgpu')) {
    Assert-Condition (-not $servicesContractManifest.Contains($forbiddenToken)) "worldline-browser-services-contract must not depend on '${forbiddenToken}'."
}

$tabsManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-tabs/Cargo.toml') -Raw
Assert-Condition (-not $tabsManifest.Contains('worldline-browser-cef')) 'worldline-browser-tabs must not depend on worldline-browser-cef.'

$historyManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-history/Cargo.toml') -Raw
Assert-Condition (-not $historyManifest.Contains('worldline-browser-cef')) 'worldline-browser-history must not depend on worldline-browser-cef.'

$downloadsManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-downloads/Cargo.toml') -Raw
Assert-Condition (-not $downloadsManifest.Contains('worldline-browser-cef')) 'worldline-browser-downloads must not depend on worldline-browser-cef.'

$cookiesManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-cookies/Cargo.toml') -Raw
Assert-Condition (-not $cookiesManifest.Contains('worldline-browser-cef')) 'worldline-browser-cookies must not depend on worldline-browser-cef.'

$cefManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-cef/Cargo.toml') -Raw
foreach ($forbiddenToken in @('worldline-browser-tabs', 'worldline-browser-history', 'worldline-browser-downloads', 'worldline-browser-cookies', 'TabId', 'HistoryEntryId', 'DownloadRecordId', 'CookieMetadata')) {
    Assert-Condition (-not $cefManifest.Contains($forbiddenToken)) "worldline-browser-cef manifest must not depend on service token '${forbiddenToken}'."
}

Write-Host 'Checking browser request-policy interception boundaries.'
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-adblock/Cargo.toml' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-browser-adblock/src/lib.rs' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-reference/src/request_policy.rs' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-reference/src/s3c.rs' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-reference/tests/s3c_acceptance.rs' -Kind File)
[void](Require-RepositoryPath -RelativePath 'crates/worldline-reference/tests/s3c_real_acceptance.rs' -Kind File)
[void](Require-RepositoryPath -RelativePath 'docs/adr/ADR-BROWSER-REQUEST-POLICY-INTERCEPTION-V1.md' -Kind File)

$policyContractManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-contract/Cargo.toml') -Raw
foreach ($forbiddenToken in @(
        'worldline-browser-adblock',
        'worldline-browser-provider',
        'worldline-browser-services-contract',
        'cef',
        'chromium',
        'webview2',
        'wpewebkit',
        'gecko',
        'servo'
    )) {
    Assert-Condition (-not $policyContractManifest.ToLowerInvariant().Contains($forbiddenToken)) "worldline-browser-contract must remain engine/provider/profile neutral and must not depend on '${forbiddenToken}'."
}

$providerManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-provider/Cargo.toml') -Raw
Assert-Condition ($providerManifest.Contains('worldline-browser-contract')) 'worldline-browser-provider must depend on the neutral browser contract.'
foreach ($forbiddenToken in @('worldline-browser-cef', 'worldline-browser-adblock', 'worldline-browser-tabs', 'worldline-browser-history', 'worldline-browser-downloads', 'worldline-browser-cookies')) {
    Assert-Condition (-not $providerManifest.Contains($forbiddenToken)) "worldline-browser-provider must not depend on '${forbiddenToken}'."
}

$adblockManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-adblock/Cargo.toml') -Raw
Assert-Condition ($adblockManifest.Contains('worldline-browser-contract')) 'worldline-browser-adblock must depend on the neutral browser contract.'
Assert-Condition ($adblockManifest.Contains('worldline-browser-provider')) 'worldline-browser-adblock must implement the provider-owned evaluator boundary.'
foreach ($forbiddenToken in @('worldline-browser-cef', 'worldline-kernel', 'worldline-browser-tabs', 'worldline-browser-history', 'worldline-browser-downloads', 'worldline-browser-cookies')) {
    Assert-Condition (-not $adblockManifest.Contains($forbiddenToken)) "worldline-browser-adblock must not depend on '${forbiddenToken}'."
}
$adblockSource = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-adblock/src/lib.rs') -Raw
foreach ($forbiddenToken in @('worldline_browser_cef', 'worldline_kernel', 'worldline_browser_tabs', 'worldline_browser_history', 'worldline_browser_downloads', 'worldline_browser_cookies')) {
    Assert-Condition (-not $adblockSource.Contains($forbiddenToken)) "worldline-browser-adblock source must not reference '${forbiddenToken}'."
}
foreach ($requiredToken in @('RequestPolicyEvaluator', 'MAX_RULES', 'fail_open_registration')) {
    Assert-Condition ($adblockSource.Contains($requiredToken)) "worldline-browser-adblock source must contain '${requiredToken}'."
}

$cefManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-cef/Cargo.toml') -Raw
Assert-Condition ($cefManifest.Contains('worldline-browser-provider')) 'worldline-browser-cef must depend on the provider boundary.'
Assert-Condition ($cefManifest.Contains('worldline-browser-contract')) 'worldline-browser-cef must depend on the neutral browser contract.'
foreach ($forbiddenToken in @('worldline-browser-adblock', 'worldline-kernel', 'worldline-browser-services-contract')) {
    Assert-Condition (-not $cefManifest.Contains($forbiddenToken)) "worldline-browser-cef must not depend on '${forbiddenToken}'."
}
$cefSourceRoot = Get-RepositoryPath -RelativePath 'crates/worldline-browser-cef/src'
$cefSourceFiles = @(Get-ChildItem -LiteralPath $cefSourceRoot -Filter '*.rs' -File -Recurse)
foreach ($sourceFile in $cefSourceFiles) {
    $sourceText = Get-Content -LiteralPath $sourceFile.FullName -Raw
    foreach ($forbiddenToken in @('AdblockRule', 'FilterList', 'worldline_browser_adblock', 'worldline_browser_provider::RequestPolicyEvaluator')) {
        Assert-Condition (-not $sourceText.Contains($forbiddenToken)) "CEF adapter source '$($sourceFile.Name)' must not contain or import '${forbiddenToken}'."
    }
}

foreach ($serviceCrate in @('worldline-browser-tabs', 'worldline-browser-history', 'worldline-browser-downloads', 'worldline-browser-cookies')) {
    $serviceManifest = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath "crates/${serviceCrate}/Cargo.toml") -Raw
    foreach ($forbiddenToken in @('worldline-browser-adblock', 'worldline-browser-provider', 'worldline-browser-cef')) {
        Assert-Condition (-not $serviceManifest.Contains($forbiddenToken)) "${serviceCrate} must not depend on request-policy implementation/engine token '${forbiddenToken}'."
    }
}

$providerProcessSource = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-provider-process/src/lib.rs') -Raw
foreach ($requiredToken in @('REQUEST_POLICY_INTERFACE', 'REQUEST_POLICY_MAX_IN_FLIGHT', 'RequestPolicyFailureMode', 'fail-open', 'fail-closed')) {
    Assert-Condition ($providerProcessSource.Contains($requiredToken)) "Provider-process source must declare '${requiredToken}' for bounded configurable request-policy semantics."
}
Assert-Condition ($providerProcessSource.Contains('const PROVIDER_COMMAND_QUEUE_CAPACITY')) 'Provider process must retain an explicit bounded command queue.'

$transportSource = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'crates/worldline-browser-provider-process/src/request_policy_transport.rs') -Raw
foreach ($requiredToken in @('max_in_flight', 'max_frame_bytes', 'DeadlineExceeded', 'Cancellation', 'retired', 'unknown correlation')) {
    Assert-Condition ($transportSource.Contains($requiredToken)) "Request-policy transport must contain bounded/cancellation guard '${requiredToken}'."
}

$policyAdr = Get-Content -LiteralPath (Get-RepositoryPath -RelativePath 'docs/adr/ADR-BROWSER-REQUEST-POLICY-INTERCEPTION-V1.md') -Raw
foreach ($requiredToken in @('T004-real-20260902-local-01', 'FailOpen', 'EVENT BUS IS NOT RPC', 'real CEF', 'stop/replan')) {
    Assert-Condition ($policyAdr.Contains($requiredToken)) "Request-policy ADR must preserve '${requiredToken}' evidence or decision text."
}

Write-Host 'Architecture guard passed: GRACE layout, dependency direction, browser contracts/services, and request-policy interception boundaries are valid.'
