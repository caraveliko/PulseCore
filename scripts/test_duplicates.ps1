# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Salvaguarda antiduplicación Rust para eMuleRust (Pulse Core).
.DESCRIPTION
    Ejecuta scripts/find_duplicates.py (análisis sin conexión: símbolos homónimos,
    cuerpos de funciones idénticos o idénticos tras renombrar, ventanas de clones entre
    archivos) y falla si aparece algún NUEVO grupo de clones exactos (B1) fuera de la
    lista blanca versionada scripts/dup_allowlist.txt.

    Las secciones A (homónimos) y B2 (cuerpos normalizados) son informativas: contienen
    falsos positivos legítimos (métodos de trait, esqueletos de códecs de cable,
    accesores bit-field) que no deben bloquear la integración continua.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_duplicates.ps1
#>

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$Report = Join-Path $Root "target\dup-report.txt"
$Allowlist = Join-Path $Root "scripts\dup_allowlist.txt"

if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    Write-Error "python no encontrado: es obligatorio para scripts/find_duplicates.py"
    exit 1
}

Write-Host "==> Análisis de duplicación Rust (símbolos + cuerpos + ventanas)..." -ForegroundColor Yellow
python scripts/find_duplicates.py | Out-File -Encoding utf8 $Report
if ($LASTEXITCODE -ne 0) {
    Write-Error "find_duplicates.py falló con error."
    exit $LASTEXITCODE
}
Write-Host "  Informe completo: $Report" -ForegroundColor DarkGray

# -- Línea base -------------------------------------------------------------
$baseline = Select-String -Path $Report -Pattern '^== Baseline ==\s+(\d+) archivos, (\d+) líneas' |
    Select-Object -First 1
if ($baseline) {
    Write-Host ("  Línea base: {0} archivos, {1} líneas" -f $baseline.Matches[0].Groups[1].Value, $baseline.Matches[0].Groups[2].Value) -ForegroundColor DarkGray
}

# -- Lista blanca -----------------------------------------------------------
$allowed = @{}
Get-Content $Allowlist | ForEach-Object {
    $line = $_.Trim()
    if ($line.Length -gt 0 -and -not $line.StartsWith('#')) {
        $allowed[$line] = $true
    }
}

# -- Sección B1: grupos de cuerpos estrictamente idénticos -----------------
$lines = Get-Content $Report
$inB1 = $false
$newGroups = @()
$knownGroups = @()
foreach ($line in $lines) {
    if ($line -match '^== B1\.') { $inB1 = $true; continue }
    if ($line -match '^== B2\.') { break }
    if ($inB1 -and $line -match '^\s*\[(.+)\] x(\d+)') {
        $group = $Matches[1]
        if ($allowed.ContainsKey($group)) {
            $knownGroups += $group
            $allowed.Remove($group)
        } else {
            $newGroups += $group
        }
    }
}

foreach ($stale in $allowed.Keys) {
    Write-Host "  [INFO] Entrada de lista blanca obsoleta: [$stale]" -ForegroundColor DarkYellow
}

if ($newGroups.Count -gt 0) {
    Write-Host "`n  [FALLO] Nuevos clones exactos detectados:" -ForegroundColor Red
    foreach ($g in $newGroups) {
        Write-Host "    - [$g]" -ForegroundColor Red
    }
    Write-Host "  Consulta $Report (sección B1). Unifica el código o justifica" -ForegroundColor Red
    Write-Host "  cada excepción en scripts/dup_allowlist.txt." -ForegroundColor Red
    exit 1
}

Write-Host ("  [OK] Sin nuevos clones exactos ({0} grupo(s) permitido(s) según lista blanca)." -f $knownGroups.Count) -ForegroundColor Green
exit 0
