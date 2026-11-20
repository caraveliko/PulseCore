# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Script unificado de verificación para Pulse Core / eMuleRust (servidor Rust + WebUI).
.DESCRIPTION
    Delega en `scripts/smart_test.py` según el modo seleccionado:
    - full  (predeterminado): clippy completo, guardia de duplicados B1, validación JS y todas las pruebas.
    - smart: únicamente crates y suites afectadas por los cambios locales en Git.
    - fast:  pruebas unitarias rápidas (--lib) sobre los crates afectados.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\verify_all.ps1 -Mode smart
#>

param(
    [ValidateSet("full", "smart", "fast")]
    [string]$Mode = "full"
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Verificación de Pulse Core / eMuleRust ($Mode)  " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

$PythonCmd = Get-Command python -ErrorAction SilentlyContinue
if (-not $PythonCmd) {
    $PythonCmd = Get-Command python3 -ErrorAction SilentlyContinue
}

if ($PythonCmd) {
    & $PythonCmd.Source "$Root\scripts\smart_test.py" --mode $Mode
    if ($LASTEXITCODE -ne 0) {
        Write-Error "La validación en modo '$Mode' falló con código $LASTEXITCODE."
        exit $LASTEXITCODE
    }
    Write-Host "`n[OK] Validación completada con éxito (100 % verde)." -ForegroundColor Green
    exit 0
}

# Respaldo sin Python (solo modo secuencial tradicional)
Write-Warning "Python no encontrado; ejecutando validación tradicional sin análisis inteligente."
cargo clippy --workspace --all-targets --all-features -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

powershell -NoProfile -ExecutionPolicy RemoteSigned -File "$Root\scripts\test_duplicates.ps1"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo test --workspace --all-features --lib --tests
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "`n[OK] Validación completada con éxito (100 % verde)." -ForegroundColor Green
