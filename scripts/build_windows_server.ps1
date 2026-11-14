# SPDX-License-Identifier: GPL-2.0-or-later
<#
.SYNOPSIS
    Compila y empaqueta la edición Servidor Windows (Headless + WebUI Embebida + Bandeja) de Pulse Core.
.DESCRIPTION
    Compila únicamente mule-daemon.exe y mule-cli.exe (sin dependencia de Flutter),
    firma opcionalmente los binarios con Authenticode, genera version.json, SBOM CycloneDX,
    SHA256SUMS.txt y el archivo comprimido dist\PulseCore-Server-Windows-x64.zip.
    No crea portable.flag por defecto para conservar tus configuraciones y base de datos
    existentes en %LOCALAPPDATA%\eMuleRust.
    La variante -Flavor qa-loopback compila con la feature lab-tools y sirve EXCLUSIVAMENTE
    para las pruebas locales en loopback (test_qa_loopback_e2e.ps1, test_phase_b_functional_bench.ps1).
    Nunca debe publicarse.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_server.ps1
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\build_windows_server.ps1 -Flavor qa-loopback
#>

param(
    [ValidateSet("standard", "qa-loopback")]
    [string]$Flavor = "standard",
    [string]$OutputDir = "",
    [switch]$Portable = $false,
    [string]$CertificateThumbprint = ""
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$IsQa = $Flavor -eq "qa-loopback"
$BundleName = if ($IsQa) { "PulseCore-Qa-Loopback-Windows" } else { "PulseCore-Server-Windows" }
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = "dist\$BundleName"
}
if ($IsQa) {
    # La variante de laboratorio siempre es portable (datos junto al ejecutable).
    $Portable = $true
}

# Versión única tomada de [workspace.package] en Cargo.toml
$AppVersion = "0.0.0"
$CargoToml = Get-Content (Join-Path $Root "Cargo.toml") -Raw
if ($CargoToml -match '(?ms)\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
    $AppVersion = $Matches[1]
}

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Pulse Core — Compilación Servidor Windows x64   " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

# 1. Verificar Cargo / Rust y rustc
$CargoCmd = Get-Command "cargo" -ErrorAction SilentlyContinue
$Cargo = if ($CargoCmd) { $CargoCmd.Source } elseif (Test-Path "$env:USERPROFILE\.cargo\bin\cargo.exe") { "$env:USERPROFILE\.cargo\bin\cargo.exe" } else { $null }
if (!$Cargo -or !(Test-Path $Cargo)) {
    throw "No se encontró 'cargo'. Instala Rust desde https://rustup.rs/"
}
$RustcVersion = try { (& rustc --version).Trim() } catch { "rustc unknown" }

# 2. Compilar mule-daemon y mule-cli en Release
Write-Host "`n[1/5] Compilando mule-daemon.exe y mule-cli.exe (release, variante $Flavor, v$AppVersion)..." -ForegroundColor Yellow
if ($IsQa) {
    & $Cargo build --release --bin mule-daemon --bin mule-cli --features mule-daemon/lab-tools
} else {
    & $Cargo build --release --bin mule-daemon --bin mule-cli
}
if ($LASTEXITCODE -ne 0) {
    throw "Error al compilar Rust (código $LASTEXITCODE)"
}

$DaemonExe = Join-Path $Root "target\release\mule-daemon.exe"
$CliExe = Join-Path $Root "target\release\mule-cli.exe"
if (!(Test-Path $DaemonExe)) { throw "No se encontró $DaemonExe" }
if (!(Test-Path $CliExe)) { throw "No se encontró $CliExe" }

# 3. Ensamblar carpeta de distribución
Write-Host "`n[2/5] Ensamblando paquete en $OutputDir..." -ForegroundColor Yellow
$DestPath = Join-Path $Root $OutputDir
if (Test-Path $DestPath) {
    Get-Process -Name "mule-daemon" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item -Path $DestPath -Recurse -Force
}
New-Item -ItemType Directory -Path $DestPath -Force | Out-Null

$DestDaemon = Join-Path $DestPath "mule-daemon.exe"
$DestCli = Join-Path $DestPath "mule-cli.exe"
Copy-Item -Path $DaemonExe -Destination $DestDaemon -Force
Copy-Item -Path $CliExe -Destination $DestCli -Force

if (![string]::IsNullOrWhiteSpace($CertificateThumbprint)) {
    Write-Host " -> Firmando ejecutables con certificado $CertificateThumbprint..." -ForegroundColor Gray
    $cert = Get-Item "Cert:\CurrentUser\My\$CertificateThumbprint"
    foreach ($exePath in @($DestDaemon, $DestCli)) {
        $sig = Set-AuthenticodeSignature -FilePath $exePath -Certificate $cert -HashAlgorithm SHA256 -TimestampServer "http://timestamp.digicert.com"
        if ($sig.Status -ne "Valid") {
            throw "Error al firmar ${exePath}: $($sig.StatusMessage)"
        }
    }
}

if ($Portable) {
    New-Item -ItemType File -Path (Join-Path $DestPath "portable.flag") -Force | Out-Null
    Write-Host " -> Modo portable activado (portable.flag)" -ForegroundColor Gray
}

# 4. Crear lanzadores cómodos para Windows
Write-Host "`n[3/5] Generando scripts de arranque..." -ForegroundColor Yellow

$StartBat = @(
    '@echo off',
    'title Pulse Core Servidor',
    'tasklist /FI "IMAGENAME eq mule-daemon.exe" 2>nul | find /I "mule-daemon.exe" >nul',
    'if errorlevel 1 start "" "%~dp0mule-daemon.exe"',
    'timeout /t 2 /nobreak >nul',
    'start "" "http://127.0.0.1:4660/"'
)
[System.IO.File]::WriteAllLines((Join-Path $DestPath "Iniciar Servidor.bat"), $StartBat, [System.Text.Encoding]::ASCII)

$StopBat = @(
    '@echo off',
    'taskkill /IM mule-daemon.exe /F >nul 2>&1',
    'echo Servidor detenido.',
    'timeout /t 2 >nul'
)
[System.IO.File]::WriteAllLines((Join-Path $DestPath "Detener Servidor.bat"), $StopBat, [System.Text.Encoding]::ASCII)

# 5. Generar version.json y SBOM CycloneDX
Write-Host "`n[4/5] Generando version.json y SBOM CycloneDX..." -ForegroundColor Yellow

$GitCommit = try { (& git rev-parse HEAD 2>$null).Trim() } catch { "" }
if ([string]::IsNullOrWhiteSpace($GitCommit) -or $GitCommit.Length -ne 40) {
    $GitCommit = "0" * 40
}

$ArtifactList = [System.Collections.Generic.List[object]]::new()
$CoreFiles = @("mule-daemon.exe", "mule-cli.exe", "Iniciar Servidor.bat", "Detener Servidor.bat")
if ($Portable) { $CoreFiles += "portable.flag" }

foreach ($rel in $CoreFiles) {
    $full = Join-Path $DestPath $rel
    $item = Get-Item $full
    $sha = (Get-FileHash -Path $full -Algorithm SHA256).Hash.ToLowerInvariant()
    $ArtifactList.Add([ordered]@{
        path = $rel
        sizeBytes = $item.Length
        sha256 = $sha
    })
}

$VersionObj = [ordered]@{
    schemaVersion = 1
    product = "PulseCore"
    version = $AppVersion
    protocolVersion = 1
    distributionFlavor = $Flavor
    labToolsEnabled = $IsQa
    builtAtUtc = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    gitCommit = $GitCommit
    toolchains = [ordered]@{
        rustc = $RustcVersion
    }
    artifacts = $ArtifactList
}

$VersionPath = Join-Path $DestPath "version.json"
$VersionObj | ConvertTo-Json -Depth 6 | Out-File -FilePath $VersionPath -Encoding utf8

& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Root "scripts\generate_sbom.ps1") -BundleDir $OutputDir
if ($LASTEXITCODE -ne 0) {
    throw "Error al generar SBOM CycloneDX (código $LASTEXITCODE)"
}

# 6. Generar SHA256SUMS.txt y archivo ZIP comprimido
Write-Host "`n[5/5] Generando SHA256SUMS.txt y paquete ZIP..." -ForegroundColor Yellow

$SumsLines = [System.Collections.Generic.List[string]]::new()
$AllBundleFiles = Get-ChildItem -Path $DestPath -Recurse -File | Where-Object { $_.Name -ne "SHA256SUMS.txt" } | Sort-Object FullName
foreach ($f in $AllBundleFiles) {
    $rel = ($f.FullName.Substring($DestPath.Length)).TrimStart('\', '/').Replace('\', '/')
    $hash = (Get-FileHash -Path $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $SumsLines.Add("$hash  $rel")
}
$SumsPath = Join-Path $DestPath "SHA256SUMS.txt"
[System.IO.File]::WriteAllLines($SumsPath, $SumsLines, [System.Text.Encoding]::ASCII)

$ZipName = "$BundleName-x64.zip"
$ZipPath = Join-Path $Root "dist\$ZipName"
if (Test-Path $ZipPath) { Remove-Item -Path $ZipPath -Force }
Compress-Archive -Path "$DestPath\*" -DestinationPath $ZipPath -Force
$ZipHash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
"$ZipHash  $ZipName" | Out-File -FilePath "$ZipPath.sha256" -Encoding ascii

Write-Host "`n==================================================" -ForegroundColor Green
Write-Host "  ¡Servidor Windows compilado correctamente!" -ForegroundColor Green
Write-Host "  Carpeta: $DestPath" -ForegroundColor Green
Write-Host "  ZIP:     $ZipPath" -ForegroundColor Green
Write-Host "  WebUI:   http://127.0.0.1:4660" -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
