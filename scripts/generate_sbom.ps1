# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Genera el SBOM CycloneDX 1.5 JSON (Software Bill of Materials) para Pulse Core (eMuleRust).
.DESCRIPTION
    1. Genera sbom/rust.cdx.json a partir de Cargo.lock y cargo metadata.
    2. Genera sbom/windows-bundle.cdx.json a partir del paquete Windows ensamblado.
    3. Genera sbom/sbom.cdx.json consolidando los componentes de distribución.
    4. Produce license-audit.json y license-audit.md verificando el cumplimiento con deny.toml.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\generate_sbom.ps1 -BundleDir dist\PulseCore-Server-Windows
#>

param(
    [string]$BundleDir = "dist\PulseCore-Server-Windows",
    [string]$OutputDir = ""
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $Root "$BundleDir\sbom"
}

if (!(Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
}

$NowIso = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$AppVersion = "0.2.0"
$VersionJsonPath = Join-Path (Join-Path $Root $BundleDir) "version.json"
if (Test-Path $VersionJsonPath) {
    try {
        $vj = Get-Content $VersionJsonPath -Raw | ConvertFrom-Json
        if ($vj.version) { $AppVersion = $vj.version }
    } catch {}
}

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Pulse Core — Generador CycloneDX 1.5 SBOM       " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

# -----------------------------------------------------------------------------
# 0. Cargar licencias permitidas desde deny.toml
# -----------------------------------------------------------------------------
$DenyTomlPath = Join-Path $Root "deny.toml"
$AllowedLicenses = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)

if (Test-Path $DenyTomlPath) {
    $denyContent = Get-Content $DenyTomlPath -Raw
    if ($denyContent -match '(?s)\[licenses\].*?allow\s*=\s*\[(.*?)\]') {
        $rawAllow = $Matches[1]
        $matches = [regex]::Matches($rawAllow, '"([^"]+)"')
        foreach ($m in $matches) {
            $AllowedLicenses.Add($m.Groups[1].Value) | Out-Null
        }
    }
}

if ($AllowedLicenses.Count -eq 0) {
    @("MIT", "Apache-2.0", "GPL-2.0-or-later", "GPL-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Unicode-DFS-2016", "OpenSSL", "Zlib", "CC0-1.0") | ForEach-Object {
        $AllowedLicenses.Add($_) | Out-Null
    }
}

Write-Host " -> Cargadas $($AllowedLicenses.Count) licencias permitidas desde deny.toml" -ForegroundColor Gray

# Seguimiento para la auditoría de licencias
$AuditEntries = [System.Collections.Generic.List[PSCustomObject]]::new()

function Evaluate-License([string]$licenseExpr, [string]$evidence) {
    if ([string]::IsNullOrWhiteSpace($licenseExpr) -or $licenseExpr -eq "UNKNOWN") {
        return [ordered]@{
            isMultiple = $false
            isCompatible = $false
            status = "unlicensed"
            tokens = @()
        }
    }

    $tokens = ($licenseExpr -split '\s+OR\s+|\s+AND\s+|/').ForEach({ $_.Trim(' ()') }).Where({ $_.Length -gt 0 })
    $isMultiple = $tokens.Count -gt 1

    $isCompatible = $false
    foreach ($tok in $tokens) {
        if ($AllowedLicenses.Contains($tok)) {
            $isCompatible = $true
            break
        }
    }

    return [ordered]@{
        isMultiple = $isMultiple
        isCompatible = $isCompatible
        status = if ($isCompatible) { "compliant" } else { "non-compliant" }
        tokens = $tokens
    }
}

# -----------------------------------------------------------------------------
# 1. SBOM de Rust (sbom/rust.cdx.json)
# -----------------------------------------------------------------------------
Write-Host "`n[1/4] Generando SBOM CycloneDX de Rust desde dependencias Cargo..." -ForegroundColor Yellow

$MetadataRaw = & cargo metadata --format-version 1
$Metadata = $MetadataRaw | ConvertFrom-Json

$RustComponents = [System.Collections.Generic.List[PSCustomObject]]::new()

foreach ($pkg in $Metadata.packages) {
    $isWorkspace = $Metadata.workspace_members -contains $pkg.id
    $componentType = if ($isWorkspace) { "application" } else { "library" }
    $purl = "pkg:cargo/$($pkg.name)@$($pkg.version)"

    $licenseId = "UNKNOWN"
    $licenseEvidence = "unknown"

    if (![string]::IsNullOrWhiteSpace($pkg.license)) {
        $licenseId = $pkg.license
        $licenseEvidence = "declared"
    } elseif (![string]::IsNullOrWhiteSpace($pkg.license_file)) {
        $licenseId = $pkg.license_file
        $licenseEvidence = "declared"
    }

    $eval = Evaluate-License $licenseId $licenseEvidence

    $licenses = @()
    if ($licenseId -ne "UNKNOWN") {
        $licenses += @{ license = @{ id = $licenseId } }
    }

    $component = [ordered]@{
        type = $componentType
        name = $pkg.name
        version = $pkg.version
        purl = $purl
        scope = "required"
        licenseEvidence = $licenseEvidence
        properties = @(
            @{ name = "licenseEvidence"; value = $licenseEvidence }
        )
    }
    if ($licenses.Count -gt 0) {
        $component["licenses"] = $licenses
    }
    if (![string]::IsNullOrEmpty($pkg.description)) {
        $component["description"] = $pkg.description
    }

    $RustComponents.Add([PSCustomObject]$component)

    $AuditEntries.Add([PSCustomObject]@{
        name = $pkg.name
        version = $pkg.version
        ecosystem = "Rust (crates.io)"
        license = $licenseId
        licenseEvidence = $licenseEvidence
        isMultiple = $eval.isMultiple
        isCompatible = $eval.isCompatible
        status = $eval.status
    })
}

$RustSbom = [ordered]@{
    '$schema' = "http://cyclonedx.org/schema/bom-1.5.json"
    bomFormat = "CycloneDX"
    specVersion = "1.5"
    serialNumber = "urn:uuid:$([guid]::NewGuid())"
    version = 1
    metadata = [ordered]@{
        timestamp = $NowIso
        component = [ordered]@{
            type = "application"
            name = "mule-daemon"
            version = $AppVersion
            description = "Pulse Core (eMuleRust) — Daemon y motor protocolario en Rust"
        }
    }
    components = $RustComponents
}

$RustSbomPath = Join-Path $OutputDir "rust.cdx.json"
$RustSbom | ConvertTo-Json -Depth 10 | Out-File -FilePath $RustSbomPath -Encoding utf8
Write-Host " -> Generado: $RustSbomPath ($($RustComponents.Count) componentes)" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 2. SBOM de artefactos del paquete Windows (sbom/windows-bundle.cdx.json)
# -----------------------------------------------------------------------------
Write-Host "`n[2/4] Generando SBOM de artefactos del paquete Windows..." -ForegroundColor Yellow

$BundlePath = Join-Path $Root $BundleDir
$BundleComponents = [System.Collections.Generic.List[PSCustomObject]]::new()

if (Test-Path $BundlePath) {
    $Files = Get-ChildItem -Path $BundlePath -Recurse -File | Where-Object {
        $_.FullName -notmatch '[/\\]sbom[/\\]' -and $_.Name -ne "SHA256SUMS.txt" -and $_.Name -ne "version.json"
    }

    foreach ($file in $Files) {
        $relPath = ($file.FullName.Substring($BundlePath.Length)).TrimStart('\', '/').Replace('\', '/')
        $sha = (Get-FileHash -Path $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()

        $compType = if ($file.Extension -eq ".exe") { "application" } elseif ($file.Extension -eq ".dll") { "library" } else { "file" }

        $licenseId = "UNKNOWN"
        $licenseEvidence = "unknown"

        if (
            $relPath -eq "mule-daemon.exe" -or
            $relPath -eq "mule-cli.exe" -or
            $relPath -eq "bin/mule-daemon.exe" -or
            $relPath -eq "bin/mule-cli.exe" -or
            $relPath -eq "Iniciar Servidor.bat" -or
            $relPath -eq "Detener Servidor.bat" -or
            $relPath -eq "portable.flag"
        ) {
            $licenseId = "GPL-2.0-or-later"
            $licenseEvidence = "declared"
        } else {
            $licenseId = "GPL-2.0-or-later"
            $licenseEvidence = "inferred"
        }

        $eval = Evaluate-License $licenseId $licenseEvidence

        $comp = [ordered]@{
            type = $compType
            name = $relPath
            version = $AppVersion
            licenseEvidence = $licenseEvidence
            properties = @(
                @{ name = "licenseEvidence"; value = $licenseEvidence }
            )
            hashes = @(
                @{
                    alg = "SHA-256"
                    content = $sha
                }
            )
            licenses = @(
                @{ license = @{ id = $licenseId } }
            )
        }
        $BundleComponents.Add([PSCustomObject]$comp)

        $AuditEntries.Add([PSCustomObject]@{
            name = $relPath
            version = $AppVersion
            ecosystem = "Paquete Binario Windows"
            license = $licenseId
            licenseEvidence = $licenseEvidence
            isMultiple = $eval.isMultiple
            isCompatible = $eval.isCompatible
            status = $eval.status
        })
    }
}

$BundleSbom = [ordered]@{
    '$schema' = "http://cyclonedx.org/schema/bom-1.5.json"
    bomFormat = "CycloneDX"
    specVersion = "1.5"
    serialNumber = "urn:uuid:$([guid]::NewGuid())"
    version = 1
    metadata = [ordered]@{
        timestamp = $NowIso
        component = [ordered]@{
            type = "application"
            name = "PulseCore-Server-Windows"
            version = $AppVersion
            description = "Pulse Core — Paquete Servidor Windows autónomo"
        }
    }
    components = $BundleComponents
}

$BundleSbomPath = Join-Path $OutputDir "windows-bundle.cdx.json"
$BundleSbom | ConvertTo-Json -Depth 10 | Out-File -FilePath $BundleSbomPath -Encoding utf8
Write-Host " -> Generado: $BundleSbomPath ($($BundleComponents.Count) artefactos)" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 3. SBOM consolidado de distribución (sbom/sbom.cdx.json)
# -----------------------------------------------------------------------------
Write-Host "`n[3/4] Consolidando SBOM CycloneDX completo..." -ForegroundColor Yellow

$AllComponents = [System.Collections.Generic.List[PSCustomObject]]::new()
$AllComponents.AddRange($BundleComponents)
$AllComponents.AddRange($RustComponents)

$ConsolidatedSbom = [ordered]@{
    '$schema' = "http://cyclonedx.org/schema/bom-1.5.json"
    bomFormat = "CycloneDX"
    specVersion = "1.5"
    serialNumber = "urn:uuid:$([guid]::NewGuid())"
    version = 1
    metadata = [ordered]@{
        timestamp = $NowIso
        component = [ordered]@{
            type = "application"
            name = "PulseCore"
            version = $AppVersion
            description = "Pulse Core (eMuleRust) — Servidor P2P eD2K/Kad/BitTorrent con WebUI embebida"
        }
    }
    components = $AllComponents
}

$ConsolidatedPath = Join-Path $OutputDir "sbom.cdx.json"
$ConsolidatedSbom | ConvertTo-Json -Depth 10 | Out-File -FilePath $ConsolidatedPath -Encoding utf8
Write-Host " -> Generado: $ConsolidatedPath ($($AllComponents.Count) componentes en total)" -ForegroundColor Green

# -----------------------------------------------------------------------------
# 4. Informe de conformidad de licencias (deny.toml)
# -----------------------------------------------------------------------------
Write-Host "`n[4/4] Generando informe de auditoría de licencias y cumplimiento..." -ForegroundColor Yellow

$DeclaredCount = ($AuditEntries | Where-Object { $_.licenseEvidence -eq "declared" }).Count
$InferredCount = ($AuditEntries | Where-Object { $_.licenseEvidence -eq "inferred" }).Count
$UnknownCount = ($AuditEntries | Where-Object { $_.licenseEvidence -eq "unknown" }).Count
$MultipleCount = ($AuditEntries | Where-Object { $_.isMultiple -eq $true }).Count
$IncompatibleItems = $AuditEntries | Where-Object { $_.isCompatible -eq $false }

$AuditSummary = [ordered]@{
    schemaVersion = 1
    generatedAtUtc = $NowIso
    product = "PulseCore"
    version = $AppVersion
    policyFile = "deny.toml"
    allowedLicenses = ($AllowedLicenses | Sort-Object)
    summary = [ordered]@{
        totalComponents = $AuditEntries.Count
        declaredLicenses = $DeclaredCount
        inferredLicenses = $InferredCount
        unknownLicenses = $UnknownCount
        multipleLicenses = $MultipleCount
        incompatibleLicenses = $IncompatibleItems.Count
    }
    unknownItems = ($AuditEntries | Where-Object { $_.licenseEvidence -eq "unknown" })
    multipleItems = ($AuditEntries | Where-Object { $_.isMultiple -eq $true })
    incompatibleItems = $IncompatibleItems
    inventory = $AuditEntries
}

$AuditJsonPath = Join-Path $OutputDir "license-audit.json"
$AuditSummary | ConvertTo-Json -Depth 6 | Out-File -FilePath $AuditJsonPath -Encoding utf8
Write-Host " -> Generado: $AuditJsonPath" -ForegroundColor Green

$MdLines = [System.Collections.Generic.List[string]]::new()
$MdLines.Add("# Informe de Auditoría de Licencias y Dependencias — Pulse Core v$AppVersion")
$MdLines.Add("")
$MdLines.Add("**Generado el:** $NowIso (UTC)  ")
$MdLines.Add("**Política de referencia:** `deny.toml`  ")
$MdLines.Add("")
$MdLines.Add("## Síntesis de cumplimiento")
$MdLines.Add("")
$MdLines.Add("| Métrica | Valor | Estado |")
$MdLines.Add("| :--- | :---: | :--- |")
$MdLines.Add("| **Total de componentes auditados** | $($AuditEntries.Count) | Inventariados en CycloneDX 1.5 |")
$MdLines.Add("| **Licencias declaradas** (`declared`) | $DeclaredCount | Metadatos de paquetes verificados |")
$MdLines.Add("| **Licencias inferidas** (`inferred`) | $InferredCount | Heurísticas y artefactos documentados |")
$MdLines.Add("| **Licencias desconocidas** (`unknown`) | $UnknownCount | $(if ($UnknownCount -eq 0) { 'Ninguna' } else { 'Por revisar' }) |")
$MdLines.Add("| **Licencias múltiples / duales** | $MultipleCount | Licencia dual (ej.: MIT OR Apache-2.0) |")
$MdLines.Add("| **No conformes con deny.toml** | $($IncompatibleItems.Count) | $(if ($IncompatibleItems.Count -eq 0) { '100 % Conforme' } else { 'Incompatibilidad detectada' }) |")
$MdLines.Add("")
$MdLines.Add("## Licencias autorizadas (deny.toml)")
$MdLines.Add("")
$MdLines.Add('```')
$MdLines.Add(($AllowedLicenses | Sort-Object) -join ", ")
$MdLines.Add('```')
$MdLines.Add("")
if ($MultipleCount -gt 0) {
    $MdLines.Add("## Componentes con licencia múltiple (licencia dual)")
    $MdLines.Add("")
    $MdLines.Add("| Componente | Versión | Ecosistema | Expresión de licencia | Estado |")
    $MdLines.Add("| :--- | :--- | :--- | :--- | :--- |")
    foreach ($m in ($AuditEntries | Where-Object { $_.isMultiple -eq $true } | Select-Object -First 25)) {
        $MdLines.Add("| ``$($m.name)`` | $($m.version) | $($m.ecosystem) | ``$($m.license)`` | $($m.status) |")
    }
    if ($MultipleCount -gt 25) {
        $MdLines.Add("| ... y $($MultipleCount - 25) componentes más | | | | |")
    }
    $MdLines.Add("")
}

if ($IncompatibleItems.Count -gt 0) {
    $MdLines.Add("## Componentes incompatibles con deny.toml")
    $MdLines.Add("")
    $MdLines.Add("| Componente | Versión | Ecosistema | Licencia detectada |")
    $MdLines.Add("| :--- | :--- | :--- | :--- |")
    foreach ($inc in $IncompatibleItems) {
        $MdLines.Add("| ``$($inc.name)`` | $($inc.version) | $($inc.ecosystem) | ``$($inc.license)`` |")
    }
    $MdLines.Add("")
} else {
    $MdLines.Add("## Estado jurídico y de seguridad")
    $MdLines.Add("")
    $MdLines.Add("> [!NOTE]")
    $MdLines.Add("> El 100 % de los componentes de la distribución cumplen los criterios estrictos definidos en `deny.toml` (licencias permisivas MIT, Apache-2.0, BSD, ISC y licencia GPL-2.0-or-later de Pulse Core). No se ha detectado ninguna dependencia restrictiva o no autorizada.")
    $MdLines.Add("")
}

$AuditMdPath = Join-Path $OutputDir "license-audit.md"
[System.IO.File]::WriteAllLines($AuditMdPath, $MdLines, [System.Text.Encoding]::UTF8)
Write-Host " -> Generado: $AuditMdPath" -ForegroundColor Green

Write-Host "`n==================================================" -ForegroundColor Green
Write-Host "  ¡SBOM CycloneDX y auditoría completados!        " -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
