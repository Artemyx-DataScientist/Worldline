# FILE: scripts/ci/Invoke-WorldlineCi.ps1
# VERSION: 1.1.0
# START_MODULE_CONTRACT
#   PURPOSE: Run the repository-owned Worldline CI suites with local and hosted parity.
#   SCOPE: Source, Correctness, RealChromium, ArchitectureSecurity, ProvingSlice, and All suite orchestration.
#   DEPENDS: M-CI-BASELINE
#   LINKS: M-CI-BASELINE
#   ROLE: SCRIPT
#   MAP_MODE: LOCALS
# END_MODULE_CONTRACT

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Source', 'Correctness', 'RealChromium', 'ArchitectureSecurity', 'ProvingSlice', 'All')]
    [string]$Suite
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path (Join-Path $scriptRoot '..') '..')).Path
$architectureGuard = Join-Path $scriptRoot 'Test-WorldlineArchitecture.ps1'

Set-Location -LiteralPath $repoRoot

function Invoke-WorldlineCommand {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Label,

        [Parameter(Mandatory = $true)]
        [string]$FilePath,

        [Parameter()]
        [string[]]$Arguments = @()
    )

    $displayArguments = if ($Arguments.Count -gt 0) {
        ' ' + ($Arguments -join ' ')
    } else {
        ''
    }

    Write-Host "==> ${Label}: ${FilePath}${displayArguments}"

    try {
        & $FilePath @Arguments
    } catch {
        Write-Error "${Label} could not be started: $($_.Exception.Message)"
        exit 1
    }

    $exitCode = $LASTEXITCODE
    if ($null -eq $exitCode) {
        $exitCode = 0
    }

    if ($exitCode -ne 0) {
        Write-Error "${Label} failed with exit code ${exitCode}."
        exit ([int]$exitCode)
    }
}

function Invoke-SourceSuite {
    Write-Host '--- Source suite ---'
    Invoke-WorldlineCommand -Label 'lines of code measurement' -FilePath 'pwsh' -Arguments @('-NoProfile', '-File', (Join-Path $scriptRoot 'Get-WorldlineLoc.ps1'))
    Invoke-WorldlineCommand -Label 'rustfmt check' -FilePath 'cargo' -Arguments @('fmt', '--all', '--', '--check')
    Invoke-WorldlineCommand -Label 'workspace check' -FilePath 'cargo' -Arguments @('check', '--workspace', '--all-targets')
    Invoke-WorldlineCommand -Label 'workspace clippy' -FilePath 'cargo' -Arguments @('clippy', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings')
    Invoke-WorldlineCommand -Label 'workspace documentation' -FilePath 'cargo' -Arguments @('doc', '--workspace', '--no-deps')
    Invoke-WorldlineCommand -Label 'plugin protocol manifest and envelope schema' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-plugin-protocol')
}

function Build-ExternalNativeFixtures {
    Invoke-WorldlineCommand -Label 'build native test helpers' -FilePath 'cargo' -Arguments @('build', '-p', 'worldline-native-host', '--bins')
}

function Invoke-CorrectnessSuite {
    Write-Host '--- Correctness suite ---'
    Build-ExternalNativeFixtures
    Invoke-WorldlineCommand -Label 'workspace tests' -FilePath 'cargo' -Arguments @('test', '--workspace')
    Invoke-WorldlineCommand -Label 'storage hard-kill recovery tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-storage', '--features', 'test-failpoints', '--test', 'recovery_acceptance', '--', '--test-threads=1')
    Invoke-WorldlineCommand -Label 'upgrade chaos recovery tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-storage', '--features', 'test-failpoints', '--test', 'upgrade_chaos_acceptance', '--', '--test-threads=1')
    Invoke-WorldlineCommand -Label 'compatibility acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-kernel', '--test', 'compatibility_acceptance')
    Invoke-WorldlineCommand -Label 'upgrade and rollback acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-kernel', '--test', 'upgrade_acceptance')
    Invoke-WorldlineCommand -Label 'cross-mode logical conformance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference-external', '--test', 'cross_mode_conformance')
    Invoke-WorldlineCommand -Label 'browser contract acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-contract', '--test', 'contract_acceptance')
    Invoke-WorldlineCommand -Label 'browser provider acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-provider')
    Invoke-WorldlineCommand -Label 'browser provider process acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-provider-process')
    Invoke-WorldlineCommand -Label 'browser cef acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-cef')
    Invoke-WorldlineCommand -Label 'browser engine spike acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-spike', '--test', 'spike_acceptance')
    Invoke-WorldlineCommand -Label 'browser engine spike measurements' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-spike', '--test', 'measurement_suite')
}

function Invoke-RealChromiumSuite {
    Write-Host '--- Real Chromium suite ---'
    Invoke-WorldlineCommand -Label 'real chromium engine spike acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-spike', '--features', 'real-chromium', '--test', 'real_chromium_acceptance', '--', '--test-threads=1')
}

function Invoke-ArchitectureSecuritySuite {
    Write-Host '--- Architecture and security suite ---'
    Build-ExternalNativeFixtures
    Invoke-WorldlineCommand -Label 'architecture guard' -FilePath 'pwsh' -Arguments @('-NoProfile', '-File', $architectureGuard)
    Invoke-WorldlineCommand -Label 'kernel acceptance tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-kernel')
    Invoke-WorldlineCommand -Label 'kernel property tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-kernel', '--test', 'property_tests')
    Invoke-WorldlineCommand -Label 'kernel negative security tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-kernel', '--test', 'negative_security')
    Invoke-WorldlineCommand -Label 'kernel fuzzing smoke tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-kernel', '--test', 'fuzz_smoke')
    Invoke-WorldlineCommand -Label 'storage contract tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-storage', '--test', 'contract_acceptance')
    Invoke-WorldlineCommand -Label 'malicious wasm acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference-external', '--test', 'malicious_wasm_acceptance')
    Invoke-WorldlineCommand -Label 'external protocol robustness' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference-external', '--test', 'protocol_robustness')
    Invoke-WorldlineCommand -Label 'browser authority separation tests' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-contract')
}

function Invoke-ProvingSliceSuite {
    Write-Host '--- Proving-slice suite ---'
    Build-ExternalNativeFixtures
    Invoke-WorldlineCommand -Label 'reference boundary acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference', '--test', 'boundary_acceptance')
    Invoke-WorldlineCommand -Label 'production persistence S1 acceptance' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference', '--test', 'persistence_acceptance')
    Invoke-WorldlineCommand -Label 'browser engine S2 proving slice' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference', '--test', 's2_acceptance')
    Invoke-WorldlineCommand -Label 'worldline-demo S0/S1 proving slice' -FilePath 'cargo' -Arguments @('run', '-p', 'worldline-demo')
    Invoke-WorldlineCommand -Label 'external-provider S1 proving path' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-reference-external', '--test', 'external_s1_proving')
    Invoke-WorldlineCommand -Label 'browser engine spike proving path' -FilePath 'cargo' -Arguments @('test', '-p', 'worldline-browser-spike', '--test', 'spike_acceptance')
}

switch ($Suite) {
    'Source' {
        Invoke-SourceSuite
        break
    }
    'Correctness' {
        Invoke-CorrectnessSuite
        break
    }
    'RealChromium' {
        Invoke-RealChromiumSuite
        break
    }
    'ArchitectureSecurity' {
        Invoke-ArchitectureSecuritySuite
        break
    }
    'ProvingSlice' {
        Invoke-ProvingSliceSuite
        break
    }
    'All' {
        Invoke-SourceSuite
        Invoke-CorrectnessSuite
        Invoke-RealChromiumSuite
        Invoke-ArchitectureSecuritySuite
        Invoke-ProvingSliceSuite
        break
    }
}

Write-Host "Worldline CI suite '${Suite}' passed."
