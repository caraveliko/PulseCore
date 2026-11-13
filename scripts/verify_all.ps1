# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Unified Verification Script for Pulse Core / eMuleRust (Rust server + WebUI).
.DESCRIPTION
    Runs:
    1. Cargo Clippy on workspace with -D warnings
    2. Rust duplication guard
    3. Cargo Test on all workspace crates
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1
#>

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Pulse Core / eMuleRust Verification (Rust)     " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

# 1. Cargo Clippy
Write-Host "`n==> [1/3] Running Cargo Clippy (lints & safety checks)..." -ForegroundColor Yellow
cargo clippy --workspace --all-targets --all-features -- -D warnings
if ($LASTEXITCODE -ne 0) {
    Write-Error "Cargo Clippy failed."
    exit $LASTEXITCODE
}
Write-Host "  [OK] Cargo Clippy passed." -ForegroundColor Green

# 2. Duplication Guard (Rust)
Write-Host "`n==> [2/3] Running Rust duplication guard..." -ForegroundColor Yellow
powershell -NoProfile -ExecutionPolicy RemoteSigned -File "$Root\scripts\test_duplicates.ps1"
if ($LASTEXITCODE -ne 0) {
    Write-Error "Duplication guard failed."
    exit $LASTEXITCODE
}
Write-Host "  [OK] No new exact code clones." -ForegroundColor Green

# 3. Cargo Test
Write-Host "`n==> [3/3] Running Cargo Test Suite across workspace crates..." -ForegroundColor Yellow
cargo test --workspace
if ($LASTEXITCODE -ne 0) {
    Write-Error "Cargo Test suite failed."
    exit $LASTEXITCODE
}
Write-Host "  [OK] All Cargo tests passed." -ForegroundColor Green

Write-Host "`n==================================================" -ForegroundColor Green
Write-Host "  [SUCCESS] All verification steps passed (100% Green)! " -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
