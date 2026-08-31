# FILE: scripts/ci/Initialize-WorldlineCi.ps1
# VERSION: 1.0.0
# START_MODULE_CONTRACT
#   PURPOSE: Bootstrap and verify declared Rust toolchain requirements for CI execution.
#   SCOPE: rustup toolchain, components (rustfmt, clippy), and target (wasm32-unknown-unknown).
#   DEPENDS: M-CI-BASELINE
#   LINKS: M-CI-BASELINE
#   ROLE: SCRIPT
#   MAP_MODE: LOCALS
# END_MODULE_CONTRACT

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$toolchain = '1.98.0'
$components = @('rustfmt', 'clippy')
$targets = @('wasm32-unknown-unknown')

Write-Host "Ensuring Rust toolchain $toolchain with required components and targets..."

if (Get-Command rustup -ErrorAction SilentlyContinue) {
    Write-Host "Installing/updating toolchain $toolchain via rustup..."
    & rustup toolchain install $toolchain --profile minimal --no-self-update
    if ($LASTEXITCODE -ne 0) {
        throw "rustup toolchain install $toolchain failed with exit code $LASTEXITCODE"
    }

    foreach ($comp in $components) {
        Write-Host "Ensuring component $comp..."
        & rustup component add $comp --toolchain $toolchain
        if ($LASTEXITCODE -ne 0) {
            throw "rustup component add $comp failed with exit code $LASTEXITCODE"
        }
    }

    foreach ($tgt in $targets) {
        Write-Host "Ensuring target $tgt..."
        & rustup target add $tgt --toolchain $toolchain
        if ($LASTEXITCODE -ne 0) {
            throw "rustup target add $tgt failed with exit code $LASTEXITCODE"
        }
    }
} else {
    Write-Host "rustup not found in PATH; verifying available cargo/rustc..."
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "Neither rustup nor cargo is available in PATH."
    }
}

Write-Host "Toolchain bootstrap complete."
