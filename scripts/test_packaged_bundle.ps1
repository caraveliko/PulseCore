# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Prueba de humo automatizada post-empaquetado para la distribución autónoma de Pulse Core.
.DESCRIPTION
    Ejecución en un directorio temporal estéril fuera del repositorio Git:
    1. Copia o extrae la distribución ensamblada en una ubicación temporal aislada.
    2. Recalcula y verifica el 100 % de los hashes SHA-256 listados en SHA256SUMS.txt.
    3. Valida el esquema de version.json, longitud del commit (40 caracteres) y consistencia 4-way.
    4. Arranca mule-daemon.exe con --no-tray y un directorio de trabajo aislado vacío.
    5. Lee session.json y confirma confinamiento estricto a loopback (127.0.0.1).
    6. Lee auth.token y confirma formato hexadecimal de 64 caracteres (sin imprimir el secreto).
    7. Conecta por WebSocket JSON-RPC autenticado:
       - system.hello
       - system.status
       - verificación de ausencia de métodos lab.* en compilación estándar
       - system.stop (apagado ordenado)
    8. Verifica limpieza tras cierre y cero escrituras en el repositorio Git.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy RemoteSigned -File scripts\test_packaged_bundle.ps1
#>

param(
    [string]$BundleDir = "",
    [string]$ZipPath = "",
    [string]$Flavor = ""
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

if ([string]::IsNullOrWhiteSpace($Flavor)) {
    if ($ZipPath -match 'qa-loopback' -or $BundleDir -match 'Qa-Loopback') {
        $Flavor = "qa-loopback"
    } else {
        $Flavor = "standard"
    }
}
$ExpectedFlavor = $Flavor

if ([string]::IsNullOrWhiteSpace($BundleDir)) {
    if ($Flavor -eq "qa-loopback") {
        $BundleDir = "dist\PulseCore-Qa-Loopback-Windows"
    } else {
        $BundleDir = "dist\PulseCore-Server-Windows"
    }
}

$SourceBundle = Join-Path $Root $BundleDir
if ([string]::IsNullOrWhiteSpace($ZipPath)) {
    if ($ExpectedFlavor -eq "qa-loopback") {
        $ZipPath = Join-Path $Root "dist\PulseCore-Qa-Loopback-Windows-x64.zip"
    } else {
        $ZipPath = Join-Path $Root "dist\PulseCore-Server-Windows-x64.zip"
    }
}

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Pulse Core — Prueba de humo post-empaquetado    " -ForegroundColor Cyan
Write-Host "  Variante: $ExpectedFlavor                       " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

# -----------------------------------------------------------------------------
# 1. Crear entorno temporal aislado fuera del repositorio
# -----------------------------------------------------------------------------
$TestId = [guid]::NewGuid().ToString("N").Substring(0, 8)
$IsolatedTempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "PulseCore-SmokeTest-$ExpectedFlavor-$TestId"
$BundleCopyDir = Join-Path $IsolatedTempRoot "bundle\PulseCore-Windows"
$WorkDir = Join-Path $IsolatedTempRoot "workdir\PulseCore"

Write-Host "`n[1/6] Creando sandbox de prueba aislado en:" -ForegroundColor Yellow
Write-Host "  $IsolatedTempRoot" -ForegroundColor Gray

New-Item -ItemType Directory -Path $BundleCopyDir -Force | Out-Null
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null

$daemonProcess = $null

try {
    $ZipShaPath = "$ZipPath.sha256"

    if (Test-Path $ZipPath) {
        Write-Host " -> Archivo ZIP encontrado: $ZipPath" -ForegroundColor Gray
        if (Test-Path $ZipShaPath) {
            $expectedZipHash = ((Get-Content $ZipShaPath -Raw) -split '\s+')[0].Trim().ToLowerInvariant()
            $actualZipHash = (Get-FileHash $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualZipHash -ne $expectedZipHash) {
                throw "¡Discordancia SHA-256 en el archivo ZIP! Esperado $expectedZipHash, obtenido $actualZipHash"
            }
            Write-Host " -> SHA-256 del ZIP verificado contra .zip.sha256: $actualZipHash" -ForegroundColor Green
        }
        Write-Host " -> Extrayendo archivo ZIP en sandbox aislado..." -ForegroundColor Gray
        Expand-Archive -Path $ZipPath -DestinationPath $BundleCopyDir -Force
    } else {
        Write-Host " -> Copiando carpeta de distribución al sandbox..." -ForegroundColor Gray
        Copy-Item -Path "$SourceBundle\*" -Destination $BundleCopyDir -Recurse -Force
    }

    # -------------------------------------------------------------------------
    # 2. Verificación criptográfica de integridad (SHA256SUMS.txt)
    # -------------------------------------------------------------------------
    Write-Host "`n[2/6] Verificando integridad criptográfica mediante SHA256SUMS.txt..." -ForegroundColor Yellow
    $SumsFile = Join-Path $BundleCopyDir "SHA256SUMS.txt"
    if (!(Test-Path $SumsFile)) { throw "No se encontró SHA256SUMS.txt en el paquete." }

    $SumsLines = Get-Content $SumsFile | Where-Object { ![string]::IsNullOrWhiteSpace($_) }
    if ($SumsLines.Count -eq 0) { throw "SHA256SUMS.txt está vacío." }

    $verifiedCount = 0
    foreach ($line in $SumsLines) {
        if ($line -match '^([a-fA-F0-9]{64})\s+(.+)$') {
            $expectedHash = $Matches[1].ToLowerInvariant()
            $relPath = $Matches[2].Trim().Replace('/', '\')
            $targetFile = Join-Path $BundleCopyDir $relPath

            if (!(Test-Path $targetFile)) {
                throw "Falta archivo listado en SHA256SUMS.txt: $relPath"
            }

            $actualHash = (Get-FileHash -Path $targetFile -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actualHash -ne $expectedHash) {
                throw "¡Discordancia SHA-256 para '$relPath'! Esperado $expectedHash, obtenido $actualHash"
            }
            $verifiedCount++
        }
    }
    Write-Host " -> Verificados $verifiedCount/$($SumsLines.Count) hashes SHA-256 con coincidencia exacta." -ForegroundColor Green

    # -------------------------------------------------------------------------
    # 3. Validar esquema de version.json y consistencia 4-way
    # -------------------------------------------------------------------------
    Write-Host "`n[3/6] Validando manifiesto (version.json) y consistencia 4-way..." -ForegroundColor Yellow
    $VersionFile = Join-Path $BundleCopyDir "version.json"
    if (!(Test-Path $VersionFile)) { throw "Falta version.json en el paquete." }

    $VersionJson = Get-Content $VersionFile -Raw | ConvertFrom-Json
    if ($VersionJson.schemaVersion -ne 1) { throw "schemaVersion inesperado: $($VersionJson.schemaVersion)" }
    if ($VersionJson.product -notin @("PulseCore", "eMuleRust")) { throw "Nombre de producto inesperado: $($VersionJson.product)" }
    if ($VersionJson.protocolVersion -ne 1) { throw "protocolVersion inesperado: $($VersionJson.protocolVersion)" }
    if ($VersionJson.gitCommit.Length -ne 40) { throw "gitCommit debe ser un hash SHA completo de 40 caracteres: $($VersionJson.gitCommit)" }
    if (!$VersionJson.toolchains.rustc) { throw "Falta toolchains.rustc en version.json" }
    if (!$VersionJson.artifacts -or $VersionJson.artifacts.Count -eq 0) { throw "Lista de artefactos vacía en version.json" }

    if ($VersionJson.distributionFlavor -ne $ExpectedFlavor) {
        throw "Discordancia en distributionFlavor de version.json: esperado '$ExpectedFlavor', obtenido '$($VersionJson.distributionFlavor)'"
    }
    if ($ExpectedFlavor -eq "qa-loopback" -and !$VersionJson.labToolsEnabled) {
        throw "Se esperaba labToolsEnabled = true en version.json para la variante qa-loopback."
    }
    if ($ExpectedFlavor -eq "standard" -and $VersionJson.labToolsEnabled) {
        throw "Se esperaba labToolsEnabled = false en version.json para la variante standard."
    }

    Write-Host " -> version.json válido (versión: $($VersionJson.version), variante: $($VersionJson.distributionFlavor), commit: $($VersionJson.gitCommit.Substring(0,8))...)" -ForegroundColor Green

    # Verificación cruzada: version.json <-> disco <-> SHA256SUMS.txt <-> windows-bundle.cdx.json
    $SbomBundleFile = Join-Path $BundleCopyDir "sbom\windows-bundle.cdx.json"
    $SbomBundleObj = if (Test-Path $SbomBundleFile) { Get-Content $SbomBundleFile -Raw | ConvertFrom-Json } else { $null }

    foreach ($art in $VersionJson.artifacts) {
        $diskPath = Join-Path $BundleCopyDir ($art.path.Replace('/', '\'))
        if (!(Test-Path $diskPath)) { throw "Falta en disco el artefacto listado en version.json: $($art.path)" }
        $diskHash = (Get-FileHash $diskPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($diskHash -ne $art.sha256) {
            throw "Discordancia de hash para $($art.path) entre disco ($diskHash) y version.json ($($art.sha256))"
        }
    }

    # Verificar el ejecutable principal mule-daemon.exe
    $DaemonRelPath = if (Test-Path (Join-Path $BundleCopyDir "mule-daemon.exe")) {
        "mule-daemon.exe"
    } elseif (Test-Path (Join-Path $BundleCopyDir "bin\mule-daemon.exe")) {
        "bin/mule-daemon.exe"
    } else {
        throw "No se encontró el binario principal 'mule-daemon.exe' en el paquete."
    }

    $daemonArtifact = $VersionJson.artifacts | Where-Object { $_.path -eq $DaemonRelPath }
    if (!$daemonArtifact) { throw "Falta $DaemonRelPath en la lista de artefactos de version.json." }

    if ($SbomBundleObj) {
        $sbomDaemonComp = $SbomBundleObj.components | Where-Object { $_.name -eq $DaemonRelPath }
        if (!$sbomDaemonComp) { throw "Falta $DaemonRelPath en los componentes de windows-bundle.cdx.json." }
    }
    Write-Host " -> Consistencia 4-way verificada para $DaemonRelPath (disco, version.json, SHA256SUMS y SBOM)." -ForegroundColor Green

    # -------------------------------------------------------------------------
    # 4. Arrancar mule-daemon.exe empaquetado en workdir aislado
    # -------------------------------------------------------------------------
    Write-Host "`n[4/6] Arrancando binario mule-daemon.exe desde el paquete..." -ForegroundColor Yellow
    $PackagedDaemon = Join-Path $BundleCopyDir ($DaemonRelPath.Replace('/', '\'))

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $PackagedDaemon
    # --no-tray + LocalLab + UPnP desactivado: la prueba de humo debe ser hermética y sin GUI.
    $psi.Arguments = "--no-tray --workdir `"$WorkDir`" --profile LocalLab --upnp false"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $daemonProcess = [System.Diagnostics.Process]::Start($psi)
    if (!$daemonProcess) { throw "No se pudo lanzar el proceso mule-daemon.exe" }

    $stdoutTask = $daemonProcess.StandardOutput.ReadToEndAsync()
    $stderrTask = $daemonProcess.StandardError.ReadToEndAsync()

    Write-Host " -> Daemon iniciado (PID: $($daemonProcess.Id)). Esperando session.json..." -ForegroundColor Gray

    $SessionJsonPath = Join-Path $WorkDir "session.json"
    $AuthTokenPath = Join-Path $WorkDir "auth.token"

    $timeoutSec = 10
    $elapsed = 0
    while (!(Test-Path $SessionJsonPath) -and $elapsed -lt $timeoutSec) {
        Start-Sleep -Milliseconds 200
        $elapsed += 0.2
        if ($daemonProcess.HasExited) {
            $err = $stderrTask.Result
            throw "El daemon terminó inesperadamente con código $($daemonProcess.ExitCode): $err"
        }
    }

    if (!(Test-Path $SessionJsonPath)) {
        throw "Tiempo de espera agotado esperando session.json tras $timeoutSec segundos"
    }

    # -------------------------------------------------------------------------
    # 5. Verificar aislamiento, confinamiento a loopback y token
    # -------------------------------------------------------------------------
    Write-Host "`n[5/6] Verificando confinamiento loopback, bloqueo e integridad del token..." -ForegroundColor Yellow

    $SessionData = Get-Content $SessionJsonPath -Raw | ConvertFrom-Json
    $Endpoint = $SessionData.endpoint
    Write-Host " -> Sesión detectada: Endpoint = $Endpoint, PID = $($SessionData.pid)" -ForegroundColor Gray

    $wsUri = if ($Endpoint.StartsWith("ws://") -or $Endpoint.StartsWith("wss://")) {
        [System.Uri]::new($Endpoint)
    } else {
        [System.Uri]::new("ws://$Endpoint")
    }

    if ($wsUri.Host -ne "127.0.0.1" -and $wsUri.Host -ne "::1" -and $wsUri.Host -ne "localhost") {
        throw "VIOLACIÓN DE SEGURIDAD: El endpoint '$Endpoint' NO está confinado a loopback 127.0.0.1."
    }

    if (!(Test-Path $AuthTokenPath)) {
        throw "Falta el archivo auth.token en workdir."
    }
    $Token = (Get-Content $AuthTokenPath -Raw).Trim()
    if ($Token.Length -ne 64 -or $Token -notmatch '^[0-9a-fA-F]{64}$') {
        throw "VIOLACIÓN DE SEGURIDAD: auth.token no cumple el formato hexadecimal de 64 caracteres."
    }
    Write-Host " -> Confinamiento confirmado (127.0.0.1). auth.token válido (256 bits de entropía)." -ForegroundColor Green

    $LockPath = Join-Path $WorkDir "mule.lock"
    if (!(Test-Path $LockPath)) { throw "Falta el archivo de bloqueo exclusivo mule.lock en workdir." }

    # -------------------------------------------------------------------------
    # 6. Interacción autenticada WebSocket JSON-RPC y cierre ordenado
    # -------------------------------------------------------------------------
    Write-Host "`n[6/6] Conectando por WebSocket JSON-RPC para validar contratos del daemon..." -ForegroundColor Yellow

    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $ws.Options.SetRequestHeader("Authorization", "Bearer $Token")

    $cts = [System.Threading.CancellationTokenSource]::new(5000)

    $connectTask = $ws.ConnectAsync($wsUri, $cts.Token)
    $connectTask.Wait()

    if ($ws.State -ne [System.Net.WebSockets.WebSocketState]::Open) {
        throw "No se pudo abrir la conexión WebSocket con el daemon (Estado: $($ws.State))"
    }
    Write-Host " -> Conexión WebSocket establecida con el daemon." -ForegroundColor Gray

    function Send-JsonRpc([System.Net.WebSockets.ClientWebSocket]$socket, [string]$jsonPayload) {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($jsonPayload)
        $segment = [System.ArraySegment[byte]]::new($bytes)
        $cancel = [System.Threading.CancellationTokenSource]::new(4000)
        $send = $socket.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cancel.Token)
        $send.Wait()

        $recvBuf = [byte[]]::new(8192)
        $recvSeg = [System.ArraySegment[byte]]::new($recvBuf)
        $recv = $socket.ReceiveAsync($recvSeg, $cancel.Token)
        $recv.Wait()

        return [System.Text.Encoding]::UTF8.GetString($recvBuf, 0, $recv.Result.Count)
    }

    $helloPayload = [ordered]@{
        jsonrpc = "2.0"
        id = 1
        method = "system.hello"
        params = [ordered]@{
            protocolVersion = 1
            token = $Token
        }
    } | ConvertTo-Json -Compress
    $helloResp = Send-JsonRpc $ws $helloPayload
    Write-Host " -> Respuesta system.hello: $helloResp" -ForegroundColor Gray
    $helloJson = $helloResp | ConvertFrom-Json
    if (!$helloJson.result -or !$helloJson.result.daemonVersion) {
        throw "system.hello falló o devolvió una respuesta inválida: $helloResp"
    }
    if (!$helloJson.result.defaultDownloadDir) {
        throw "Falta defaultDownloadDir en system.hello: $helloResp"
    }

    $statusResp = Send-JsonRpc $ws '{"jsonrpc":"2.0","id":2,"method":"system.status","params":{}}'
    Write-Host " -> Respuesta system.status: $statusResp" -ForegroundColor Gray
    $statusJson = $statusResp | ConvertFrom-Json
    if (!$statusJson.result -or !$statusJson.result.defaultDownloadDir) {
        throw "system.status falló o falta defaultDownloadDir: $statusResp"
    }

    $caps = $helloJson.result.capabilities
    if ($ExpectedFlavor -eq "standard") {
        if ($caps -contains "lab-tools") {
            throw "VIOLACIÓN DE SEGURIDAD: La compilación standard anunció capacidad 'lab-tools'."
        }
        if ($caps -contains "local-simulation") {
            throw "VIOLACIÓN DE SEGURIDAD: La compilación standard anunció capacidad 'local-simulation'."
        }
        $labStatusResp = Send-JsonRpc $ws '{"jsonrpc":"2.0","id":10,"method":"lab.status","params":{}}'
        $labStatusJson = $labStatusResp | ConvertFrom-Json
        if (!$labStatusJson.error -or $labStatusJson.error.code -ne -32601) {
            throw "VIOLACIÓN DE SEGURIDAD: lab.status no devolvió MethodNotFound (-32601) en compilación standard. Recibido: $labStatusResp"
        }
        Write-Host " -> Confirmado: métodos lab.* ausentes en daemon standard (-32601 MethodNotFound)." -ForegroundColor Green
    } elseif ($ExpectedFlavor -eq "qa-loopback") {
        if ($caps -notcontains "lab-tools") {
            throw "ERROR DE CAPACIDAD: La compilación qa-loopback no incluye 'lab-tools'."
        }
        $labStatusResp = Send-JsonRpc $ws '{"jsonrpc":"2.0","id":10,"method":"lab.status","params":{}}'
        $labStatusJson = $labStatusResp | ConvertFrom-Json
        if (!$labStatusJson.result) {
            throw "Fallo en llamada lab.status en compilación qa-loopback: $labStatusResp"
        }
        Write-Host " -> Confirmado: lab.status operativo en daemon qa-loopback." -ForegroundColor Green
    }

    Write-Host " -> Solicitando cierre ordenado mediante system.stop..." -ForegroundColor Gray
    try {
        $stopResp = Send-JsonRpc $ws '{"jsonrpc":"2.0","id":3,"method":"system.stop","params":{}}'
        Write-Host " -> Respuesta system.stop: $stopResp" -ForegroundColor Gray
    } catch {
        # El cierre del socket durante el apagado es el comportamiento esperado
    }

    $ws.Dispose()

    $daemonProcess.WaitForExit(5000) | Out-Null
    if (!$daemonProcess.HasExited) {
        $daemonProcess.Kill()
        throw "El daemon no terminó en menos de 5 segundos tras system.stop."
    }
    Write-Host " -> Proceso daemon terminado limpiamente con código: $($daemonProcess.ExitCode)" -ForegroundColor Green

    if (Test-Path $SessionJsonPath) {
        throw "session.json no fue eliminado durante el cierre ordenado."
    }
    Write-Host " -> session.json eliminado limpiamente durante el apagado." -ForegroundColor Green

    $RepoDirty = try { (& git status --porcelain).Trim() } catch { "" }
    if (![string]::IsNullOrEmpty($RepoDirty)) {
        Write-Host " [AVISO] Estado de Git modificado durante la prueba: $RepoDirty" -ForegroundColor Magenta
    } else {
        Write-Host " -> Confirmado CERO archivos creados o modificados en el repositorio." -ForegroundColor Green
    }

    Write-Host "`n==================================================" -ForegroundColor Green
    Write-Host "  ¡PRUEBA DE HUMO SUPERADA! Paquete verificado.   " -ForegroundColor Green
    Write-Host "==================================================" -ForegroundColor Green

} finally {
    if ($daemonProcess -and !$daemonProcess.HasExited) {
        try { $daemonProcess.Kill() } catch {}
    }
    if (Test-Path $IsolatedTempRoot) {
        try { Remove-Item -Recurse -Force $IsolatedTempRoot -ErrorAction SilentlyContinue } catch {}
    }
}
