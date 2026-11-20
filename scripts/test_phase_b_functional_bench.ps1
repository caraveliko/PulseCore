# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Fase B - Banco funcional RPC completo (docs/qa/PLAN_TEST_COMPLET.md).
.DESCRIPTION
    Extrae el paquete QA loopback, inicia el demonio en modo portátil y
    ejercita toda la superficie JSON-RPC: identidad/créditos,
    ciclo de descargas, formatos .met/.dat, compartidos/subidas, fuentes,
    seguridad (anti-SSRF, ipfilter, perfiles), Kad, ajustes/límites,
    categorías, amigos, historial, escuchador y sistema.
    Veredicto: PASS/FAIL por llamada + informe final. 100 % sin conexión.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\test_phase_b_functional_bench.ps1
#>

$ErrorActionPreference = "Stop"
$env:MULE_NO_TRAY = "1"  # Sin icono de bandeja en pruebas automatizadas
$Root = Split-Path -Parent $PSScriptRoot

# -----------------------------------------------------------------------
# Resultados
# -----------------------------------------------------------------------
$script:Results = New-Object System.Collections.Generic.List[object]
$script:ReqId = 1000

function Add-Result([string]$Section, [string]$Name, [string]$Outcome, [string]$Detail = "") {
    $script:Results.Add([pscustomobject]@{
        Section = $Section; Name = $Name; Outcome = $Outcome; Detail = $Detail
    })
    $color = switch ($Outcome) { "PASS" { "Green" } "FAIL" { "Red" } default { "Yellow" } }
    $detailStr = if ($Detail) { "  ($Detail)" } else { "" }
    Write-Host ("  [{0}] {1}{2}" -f $Outcome, $Name, $detailStr) -ForegroundColor $color
}

# Envía una llamada JSON-RPC y clasifica el resultado.
# Expect: "ok" (resultado obligatorio), "err" (error obligatorio), "any" (cualquiera).
function Test-Rpc {
    param(
        [System.Net.WebSockets.ClientWebSocket]$Ws,
        [string]$Section, [string]$Name, [string]$Method,
        [hashtable]$Params = @{},
        [string]$Expect = "ok",
        [scriptblock]$Validate = $null,
        [int]$TimeoutMs = 8000
    )
    $script:ReqId++
    try {
        $resp = Invoke-WsRpc $Ws @{ jsonrpc = "2.0"; id = $script:ReqId; method = $Method; params = $Params } $TimeoutMs
        $hasErr = ($null -ne $resp.error)
        switch ($Expect) {
            "ok" {
                if ($hasErr) {
                    Add-Result $Section $Name "FAIL" "error inesperado: $($resp.error.message)"
                    return $null
                }
                if ($Validate) {
                    $vErr = & $Validate $resp.result
                    if ($vErr) { Add-Result $Section $Name "FAIL" $vErr; return $resp.result }
                }
                Add-Result $Section $Name "PASS"
                return $resp.result
            }
            "err" {
                if ($hasErr) { Add-Result $Section $Name "PASS" "rechazado: $($resp.error.message)"; return $resp.error }
                Add-Result $Section $Name "FAIL" "se esperaba error y se recibió resultado: $($resp.result | ConvertTo-Json -Compress -Depth 3)"
                return $null
            }
            default {
                if ($hasErr) { Add-Result $Section $Name "PASS" "error de negocio aceptado: $($resp.error.message)" }
                else { Add-Result $Section $Name "PASS" "ok" }
                return $resp
            }
        }
    } catch {
        Add-Result $Section $Name "FAIL" "excepción: $($_.Exception.Message)"
        return $null
    }
}

# -----------------------------------------------------------------------
# Auxiliares WS (mismo patrón que test_qa_loopback_e2e.ps1)
# -----------------------------------------------------------------------
function Receive-FullWsText([System.Net.WebSockets.ClientWebSocket]$ws, [System.Threading.CancellationTokenSource]$cts) {
    $ms = [System.IO.MemoryStream]::new()
    $buf = [byte[]]::new(8192)
    $recSeg = [System.ArraySegment[byte]]::new($buf)
    do {
        $task = $ws.ReceiveAsync($recSeg, $cts.Token)
        $task.Wait()
        $ms.Write($buf, 0, $task.Result.Count)
    } while (!$task.Result.EndOfMessage)
    return [System.Text.Encoding]::UTF8.GetString($ms.ToArray())
}

function Invoke-WsRpc([System.Net.WebSockets.ClientWebSocket]$ws, [hashtable]$request, [int]$timeoutMs = 8000) {
    $reqId = $request.id
    $json = $request | ConvertTo-Json -Compress -Depth 10
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $seg = [System.ArraySegment[byte]]::new($bytes)
    $cts = [System.Threading.CancellationTokenSource]::new($timeoutMs)
    $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cts.Token).Wait()

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $remain = [math]::Max(500, ($timeoutMs - $sw.ElapsedMilliseconds))
        $recCts = [System.Threading.CancellationTokenSource]::new([int]$remain)
        try {
            $respText = Receive-FullWsText $ws $recCts
        } catch {
            throw "Tiempo de espera o error al recibir WS para id $reqId"
        }
        $parsed = $respText | ConvertFrom-Json
        if ($null -ne $parsed.id -and $parsed.id -eq $reqId) {
            return $parsed
        }
    }
    throw "Tiempo de espera agotado esperando respuesta JSON-RPC id $reqId tras $timeoutMs ms"
}

function Connect-DaemonWebSocket([string]$endpoint, [string]$token) {
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $ws.Options.SetRequestHeader("Authorization", "Bearer $token")
    $ws.ConnectAsync([System.Uri]::new($endpoint), [System.Threading.CancellationTokenSource]::new(5000).Token).Wait()
    return $ws
}

function Start-PackagedDaemon([string]$binPath, [string]$dataDir) {
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $binPath
    # --profile LocalLab + --upnp false: aislamiento loopback estricto,
    # cero tráfico saliente (sin estos argumentos los paquetes QA arrancarían
    # en perfil público con mapeo UPnP automático sobre la pasarela real).
    $psi.Arguments = "--workdir `"$dataDir`" --mode portable --profile LocalLab --upnp false"
    $psi.WorkingDirectory = (Split-Path -Parent $binPath)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $proc = [System.Diagnostics.Process]::Start($psi)
    if (!$proc) { throw "No se pudo iniciar el proceso mule-daemon" }
    # Drenaje continuo: evita el bloqueo de tubería (~64 KiB).
    $script:DaemonStdoutTask = $proc.StandardOutput.ReadToEndAsync()
    $script:DaemonStderrTask = $proc.StandardError.ReadToEndAsync()

    $sessionPath = Join-Path $dataDir "session.json"
    $t = 0
    while (!(Test-Path $sessionPath) -and $t -lt 15) {
        Start-Sleep -Milliseconds 200
        $t += 0.2
        if ($proc.HasExited) {
            throw "El demonio finalizó durante el arranque ($($proc.ExitCode))"
        }
    }
    if (!(Test-Path $sessionPath)) { throw "Tiempo de espera agotado esperando session.json" }
    return $proc
}

# -----------------------------------------------------------------------
# Generadores de fixtures .met/.dat mínimos válidos
# -----------------------------------------------------------------------
function Write-BinFile([string]$Path, [byte[]]$Bytes) {
    [System.IO.File]::WriteAllBytes($Path, $Bytes)
    return $Path
}

function New-ServerMet {
    # [0xE0][u32 count=1][u32 ip][u16 port][u32 tagcount=0]
    $b = [System.Collections.Generic.List[byte]]::new()
    $b.Add(0xE0)
    $b.AddRange([BitConverter]::GetBytes([uint32]1))
    $b.AddRange([byte[]](195, 154, 122, 10))
    $b.AddRange([BitConverter]::GetBytes([uint16]4661))
    $b.AddRange([BitConverter]::GetBytes([uint32]0))
    return $b.ToArray()
}

function New-NodesDatV0 {
    # [u32 count=1][16B id][u32 ip][u16 udp][u16 tcp]
    $b = [System.Collections.Generic.List[byte]]::new()
    $b.AddRange([BitConverter]::GetBytes([uint32]1))
    $b.AddRange([byte[]](0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11,0x11))
    $b.AddRange([byte[]](80, 80, 80, 80))
    $b.AddRange([BitConverter]::GetBytes([uint16]4672))
    $b.AddRange([BitConverter]::GetBytes([uint16]4662))
    return $b.ToArray()
}

function New-ClientsMet {
    # v0x11 heredado: [0x11][u32 count=1][registro 38B = 16B clave + u32 upLo + u32 dlLo + u32 lastSeen + u32 upHi + u32 dlHi + u16 reservado]
    $b = [System.Collections.Generic.List[byte]]::new()
    $b.Add(0x11)
    $b.AddRange([BitConverter]::GetBytes([uint32]1))
    $rec = [byte[]]::new(38)
    for ($i = 0; $i -lt 16; $i++) { $rec[$i] = 0x42 }
    [BitConverter]::GetBytes([uint32]12345).CopyTo($rec, 16)   # subido bajo
    [BitConverter]::GetBytes([uint32]6789).CopyTo($rec, 20)    # descargado bajo
    [BitConverter]::GetBytes([uint32]1700000000).CopyTo($rec, 24) # última vez visto
    $b.AddRange($rec)
    return $b.ToArray()
}

function New-EmfriendsMet {
    # [0x0E][u32 count=1][16B hash][u32 ip][u16 port][u32 lastseen][u32 lastchatted][u32 tagcount=0]
    $b = [System.Collections.Generic.List[byte]]::new()
    $b.Add(0x0E)
    $b.AddRange([BitConverter]::GetBytes([uint32]1))
    $b.AddRange([byte[]](0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33,0x33))
    $b.AddRange([BitConverter]::GetBytes([uint32]0x0100007F))   # 127.0.0.1 LE
    $b.AddRange([BitConverter]::GetBytes([uint16]4662))
    $b.AddRange([BitConverter]::GetBytes([uint32]1700000000))
    $b.AddRange([BitConverter]::GetBytes([uint32]1700000000))
    $b.AddRange([BitConverter]::GetBytes([uint32]0))
    return $b.ToArray()
}

function New-CancelledMet {
    # v1: [0x0F][0x01][u32 semilla][u32 count=1][16B hash enmascarado][u8 tagcount=0]
    $b = [System.Collections.Generic.List[byte]]::new()
    $b.Add(0x0F)
    $b.Add(0x01)
    $b.AddRange([BitConverter]::GetBytes([uint32]0x12345678))
    $b.AddRange([BitConverter]::GetBytes([uint32]1))
    $b.AddRange([byte[]](0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77,0x77))
    $b.Add(0x00)
    return $b.ToArray()
}

function New-IpfilterDat {
    return [System.Text.Encoding]::ASCII.GetBytes("1.2.3.4 - 1.2.3.9 , 100 , bench-blocked`r`n# comentario`r`n5.6.7.8 - 5.6.7.8 , 090 , single`r`n")
}

# =======================================================================
# 0. Entorno aislado y arranque
# =======================================================================
Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Fase B - Banco funcional RPC (loopback)         " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

$zip = Get-ChildItem (Join-Path $Root "dist") -Filter "PulseCore-Qa-Loopback-Windows-x64.zip" |
       Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (!$zip) { throw "No se encontró el ZIP QA loopback en dist/; ejecuta primero build_windows_server.ps1 -Flavor qa-loopback" }

$WorkRoot = Join-Path $env:TEMP ("eMuleRust-PhaseB-" + (New-Guid).ToString("N").Substring(0,8))
$BundleDir = Join-Path $WorkRoot "bundle"
$WorkDir = Join-Path $WorkRoot "workdir"
New-Item -ItemType Directory -Force -Path $BundleDir, $WorkDir | Out-Null
Expand-Archive -Path $zip.FullName -DestinationPath $BundleDir -Force
# El demonio RPC es mule-daemon.exe (raíz del paquete servidor).
$DaemonBin = Get-ChildItem $BundleDir -Recurse -Filter "mule-daemon.exe" | Select-Object -First 1 -ExpandProperty FullName
if (!$DaemonBin) { throw "No se encontró mule-daemon.exe en el paquete extraído" }
Write-Host " -> Paquete : $($zip.Name)" -ForegroundColor Gray
Write-Host " -> Entorno : $WorkRoot" -ForegroundColor Gray

$daemonProcess = $null
$ws = $null
$exitCode = 0

try {
    $daemonProcess = Start-PackagedDaemon $DaemonBin $WorkDir
    $SessionData = Get-Content (Join-Path $WorkDir "session.json") -Raw | ConvertFrom-Json
    $Token = (Get-Content (Join-Path $WorkDir "auth.token") -Raw).Trim()
    $ws = Connect-DaemonWebSocket $SessionData.endpoint $Token
    Write-Host " -> Demonio activo (PID $($daemonProcess.Id)) en $($SessionData.endpoint)" -ForegroundColor Gray

    $hello = Invoke-WsRpc $ws @{ jsonrpc="2.0"; id=1; method="system.hello"; params=@{ protocolVersion=1; token=$Token } }
    Add-Result "B.0" "system.hello handshake" "PASS" "v$($hello.result.version)"

    # Carpeta de fixtures compartidos + directorio de compartidos para shares.add
    $FixDir = Join-Path $WorkRoot "bench-files"
    New-Item -ItemType Directory -Force -Path $FixDir | Out-Null
    $shareDir = Join-Path $WorkRoot "share-dir"
    New-Item -ItemType Directory -Force -Path $shareDir | Out-Null
    "eMuleRust contenido de prueba Fase B" | Out-File -Encoding ascii (Join-Path $shareDir "bench-share.txt")

    # ===================================================================
    # B.1 Identidad, SUI y créditos
    # ===================================================================
    Write-Host "`n[B.1] Identidad / SUI / Créditos" -ForegroundColor Yellow
    $id1 = Test-Rpc $ws "B.1" "identity.status" "identity.status" -Validate {
        param($r) if ($null -eq $r.hasKey -and $null -eq $r.pubkeyHash) { "faltan campos de identidad" }
    }
    $script:hash1 = if ($id1) { $id1.pubkeyHash } else { $null }
    Test-Rpc $ws "B.1" "identity.rotate" "identity.rotate" | Out-Null
    Test-Rpc $ws "B.1" "identity.status (tras rotación)" "identity.status" -Params @{} -Validate {
        param($r) if ($script:hash1 -and $r.pubkeyHash -eq $script:hash1) { "pubkeyHash sin cambios tras rotación" }
    } | Out-Null
    Test-Rpc $ws "B.1" "identity.importEmuleIdentity (archivo ausente)" "identity.importEmuleIdentity" @{ cryptkeyPath = "Z:\nonexistent\cryptkey.dat" } -Expect "err" | Out-Null
    Test-Rpc $ws "B.1" "credits.list" "credits.list" | Out-Null
    Test-Rpc $ws "B.1" "credits.stats" "credits.stats" | Out-Null
    $clientsMet = Write-BinFile (Join-Path $FixDir "clients.met") (New-ClientsMet)
    Test-Rpc $ws "B.1" "credits.importClientsMet (válido)" "credits.importClientsMet" @{ metPath = $clientsMet } | Out-Null
    $badMet = Write-BinFile (Join-Path $FixDir "garbage.met") ([byte[]](0xDE,0xAD,0xBE,0xEF))
    Test-Rpc $ws "B.1" "credits.importClientsMet (malformado)" "credits.importClientsMet" @{ metPath = $badMet } -Expect "err" | Out-Null

    # ===================================================================
    # B.2 Ciclo de descargas (transferencia real cubierta en prueba E2E dedicada)
    # ===================================================================
    Write-Host "`n[B.2] Ciclo de descargas" -ForegroundColor Yellow
    $fixResp = Test-Rpc $ws "B.2" "lab.createFixture" "lab.createFixture" @{ fixtureType = "sample_1mb" }
    $fixLink = $fixResp.ed2kLink; $fixHash = $fixResp.ed2kHash; $fixPath = $fixResp.filePath

    $addResp = Test-Rpc $ws "B.2" "downloads.addEd2kLink (válido)" "downloads.addEd2kLink" @{ link = $fixLink }
    $script:dlId = $addResp.downloadId
    Test-Rpc $ws "B.2" "downloads.addEd2kLink (malformado)" "downloads.addEd2kLink" @{ link = "ed2k://|file|x|abc|zz|/" } -Expect "err" | Out-Null
    Test-Rpc $ws "B.2" "downloads.addEd2kLink (no ed2k)" "downloads.addEd2kLink" @{ link = "http://example.com/f" } -Expect "err" | Out-Null
    Test-Rpc $ws "B.2" "downloads.list" "downloads.list" -Validate {
        param($r) if ($r.downloads.Count -lt 1) { "descarga no listada" }
    } | Out-Null
    Test-Rpc $ws "B.2" "downloads.get" "downloads.get" @{ id = $dlId } -Validate {
        param($r) if ($r.id -ne $script:dlId) { "descarga devuelta incorrecta" }
    } | Out-Null
    Test-Rpc $ws "B.2" "downloads.pause" "downloads.pause" @{ id = $dlId } | Out-Null
    Test-Rpc $ws "B.2" "downloads.resume" "downloads.resume" @{ id = $dlId } | Out-Null
    Test-Rpc $ws "B.2" "downloads.pause (de nuevo)" "downloads.pause" @{ id = $dlId } | Out-Null
    Test-Rpc $ws "B.2" "downloads.rename" "downloads.rename" @{ id = $dlId; newName = "bench-renamed.bin" } | Out-Null
    Test-Rpc $ws "B.2" "downloads.setPriority" "downloads.setPriority" @{ id = $dlId; priority = "high" } "any" | Out-Null
    Test-Rpc $ws "B.2" "downloads.setMaxSources" "downloads.setMaxSources" @{ id = $dlId; maxSources = 8 } "any" | Out-Null
    $catDir = Join-Path $WorkRoot "cat-dir"; New-Item -ItemType Directory -Force -Path $catDir | Out-Null
    Test-Rpc $ws "B.2" "downloads.setDestination" "downloads.setDestination" @{ id = $dlId; targetDirectory = $catDir } "any" | Out-Null
    Test-Rpc $ws "B.2" "downloads.getSources" "downloads.getSources" @{ id = $dlId } -Expect "any" | Out-Null
    Test-Rpc $ws "B.2" "downloads.retry" "downloads.retry" @{ id = $dlId } -Expect "any" | Out-Null
    Test-Rpc $ws "B.2" "downloads.isCancelled" "downloads.isCancelled" @{ fileHash = $fixHash } | Out-Null
    Test-Rpc $ws "B.2" "downloads.inspectPartMet (ausente)" "downloads.inspectPartMet" @{ metPath = "Z:\nope.part.met" } -Expect "err" | Out-Null
    Test-Rpc $ws "B.2" "downloads.importPartMet (ausente)" "downloads.importPartMet" @{ metPath = "Z:\nope.part.met" } -Expect "err" | Out-Null
    $cancelMet = Write-BinFile (Join-Path $FixDir "cancelled.met") (New-CancelledMet)
    Test-Rpc $ws "B.2" "downloads.importCancelledMet (válido)" "downloads.importCancelledMet" @{ metPath = $cancelMet } -Expect "any" | Out-Null
    Test-Rpc $ws "B.2" "downloads.importCancelledMet (malformado)" "downloads.importCancelledMet" @{ metPath = $badMet } -Expect "err" | Out-Null
    # debug.injectRange sobre la descarga antes de eliminarla
    Test-Rpc $ws "B.2" "debug.injectRange" "debug.injectRange" @{ downloadId = $dlId; offset = 0; dataHex = "DEADBEEF01020304" } -Expect "any" | Out-Null
    Test-Rpc $ws "B.2" "downloads.setCategory" "downloads.setCategory" @{ id = $dlId; categoryId = "" } -Expect "any" | Out-Null
    # Nota: downloads.remove se pospone al final de B.5 porque las fuentes
    # permanecen vinculadas al fileHash de la descarga.

    # ===================================================================
    # B.3 Formatos .met/.dat (importaciones vía RPC)
    # ===================================================================
    Write-Host "`n[B.3] Formatos .met / .dat" -ForegroundColor Yellow
    $serverMet = Write-BinFile (Join-Path $FixDir "server.met") (New-ServerMet)
    Test-Rpc $ws "B.3" "servers.inspectMet (válido)" "servers.inspectMet" @{ path = $serverMet } | Out-Null
    $truncMet = Write-BinFile (Join-Path $FixDir "server_trunc.met") ([byte[]](0xE0,0x01,0x00,0x00,0x00))
    Test-Rpc $ws "B.3" "servers.inspectMet (truncado)" "servers.inspectMet" @{ path = $truncMet } -Expect "err" | Out-Null
    Test-Rpc $ws "B.3" "servers.importMet (válido)" "servers.importMet" @{ path = $serverMet } | Out-Null
    Test-Rpc $ws "B.3" "servers.list (tras importación)" "servers.list" -Validate {
        param($r) if ($r.servers.Count -lt 1) { "servidor importado ausente en lista" }
    } | Out-Null

    $nodesDat = Write-BinFile (Join-Path $FixDir "nodes.dat") (New-NodesDatV0)
    Test-Rpc $ws "B.3" "kad.inspectNodesDat (válido)" "kad.inspectNodesDat" @{ path = $nodesDat } | Out-Null
    Test-Rpc $ws "B.3" "kad.importNodesDat (válido)" "kad.importNodesDat" @{ path = $nodesDat } | Out-Null
    Test-Rpc $ws "B.3" "kad.inspectNodesDat (malformado)" "kad.inspectNodesDat" @{ path = $badMet } -Expect "err" | Out-Null

    $emfriends = Write-BinFile (Join-Path $FixDir "emfriends.met") (New-EmfriendsMet)
    Test-Rpc $ws "B.3" "friends.importEmfriendsMet (válido)" "friends.importEmfriendsMet" @{ metPath = $emfriends } -Expect "any" | Out-Null
    Test-Rpc $ws "B.3" "friends.importEmfriendsMet (malformado)" "friends.importEmfriendsMet" @{ metPath = $badMet } -Expect "err" | Out-Null
    Test-Rpc $ws "B.3" "shares.importKnownMet (ausente)" "shares.importKnownMet" @{ metPath = "Z:\nope\known.met" } -Expect "err" | Out-Null

    # ===================================================================
    # B.4 Compartidos / subidas
    # ===================================================================
    Write-Host "`n[B.4] Compartidos / Subidas" -ForegroundColor Yellow
    # shares.add registra un archivo (fileHash/fileName/fileSize/filePath)
    Test-Rpc $ws "B.4" "shares.add" "shares.add" @{ fileHash = $fixHash; fileName = "sample_1mb.mkv"; fileSize = 1048576; filePath = $fixPath } | Out-Null
    Test-Rpc $ws "B.4" "shares.list" "shares.list" -Validate {
        param($r) if ($r.sharedFiles.Count -lt 1) { "archivo compartido no listado" }
    } | Out-Null
    $shareKey = $fixHash
    if ($shareKey) {
        Test-Rpc $ws "B.4" "shares.rename" "shares.rename" @{ fileHash = $shareKey; newName = "bench-share-renamed.mkv" } -Expect "any" | Out-Null
        Test-Rpc $ws "B.4" "shares.setEnabled(false)" "shares.setEnabled" @{ fileHash = $shareKey; enabled = $false } -Expect "any" | Out-Null
        Test-Rpc $ws "B.4" "shares.setUploadPriority" "shares.setUploadPriority" @{ fileHash = $shareKey; priority = "high" } -Expect "any" | Out-Null
        Test-Rpc $ws "B.4" "shares.setCategory" "shares.setCategory" @{ fileHash = $shareKey; categoryId = "" } -Expect "any" | Out-Null
    } else {
        Add-Result "B.4" "mutaciones de compartidos" "FAIL" "sin hash compartido disponible"
    }
    Test-Rpc $ws "B.4" "shares.rescan" "shares.rescan" @{ directory = $shareDir } -Expect "any" | Out-Null
    Test-Rpc $ws "B.4" "uploads.list" "uploads.list" | Out-Null
    Test-Rpc $ws "B.4" "uploads.status" "uploads.status" | Out-Null
    Test-Rpc $ws "B.4" "uploads.cancel (sin activas)" "uploads.cancel" @{ clientAddress = "127.0.0.1:1" } -Expect "any" | Out-Null
    if ($shareKey) {
        Test-Rpc $ws "B.4" "shares.remove" "shares.remove" @{ fileHash = $shareKey } -Expect "any" | Out-Null
    }

    # ===================================================================
    # B.5 Fuentes
    # ===================================================================
    Write-Host "`n[B.5] Fuentes" -ForegroundColor Yellow
    Test-Rpc $ws "B.5" "sources.addManual" "sources.addManual" @{ fileHash = $fixHash; address = "127.0.0.1:9999" } -Expect "any" | Out-Null
    Test-Rpc $ws "B.5" "sources.addManual (dirección inválida)" "sources.addManual" @{ fileHash = $fixHash; address = "not-an-addr" } -Expect "err" | Out-Null
    Test-Rpc $ws "B.5" "sources.list" "sources.list" @{ fileHash = $fixHash } | Out-Null
    Test-Rpc $ws "B.5" "sources.status" "sources.status" @{ fileHash = $fixHash } -Expect "any" | Out-Null
    Test-Rpc $ws "B.5" "sources.connect (par caído)" "sources.connect" @{ address = "127.0.0.1:1" } -Expect "any" | Out-Null
    Test-Rpc $ws "B.5" "sources.disconnect" "sources.disconnect" @{ address = "127.0.0.1:1" } -Expect "any" | Out-Null
    Test-Rpc $ws "B.5" "sources.clear" "sources.clear" @{ fileHash = $fixHash } | Out-Null
    Test-Rpc $ws "B.5" "downloads.remove" "downloads.remove" @{ id = $script:dlId } | Out-Null

    # ===================================================================
    # B.6 Seguridad: anti-SSRF, ipfilter y perfiles
    # ===================================================================
    Write-Host "`n[B.6] Seguridad" -ForegroundColor Yellow
    Test-Rpc $ws "B.6" "servers.updateFromUrl (link-local SSRF)" "servers.updateFromUrl" @{ url = "http://169.254.169.254/latest/meta-data" } "err" | Out-Null
    Test-Rpc $ws "B.6" "servers.updateFromUrl (LAN privada)" "servers.updateFromUrl" @{ url = "http://192.168.1.254/server.met" } "err" | Out-Null
    Test-Rpc $ws "B.6" "servers.updateFromUrl (esquema file)" "servers.updateFromUrl" @{ url = "file:///c:/windows/win.ini" } "err" | Out-Null
    Test-Rpc $ws "B.6" "servers.updateFromUrl (pública, rechazada en LocalLab)" "servers.updateFromUrl" @{ url = "http://8.8.8.8/server.met" } "err" | Out-Null
    # URL loopback permitida por la política -> petición real, conexión
    # rechazada en puerto cerrado -> error de negocio (flujo completo sin conexión)
    Test-Rpc $ws "B.6" "servers.updateFromUrl (loopback, conexión rechazada)" "servers.updateFromUrl" @{ url = "http://127.0.0.1:9/server.met" } "err" | Out-Null
    Test-Rpc $ws "B.6" "kad.bootstrapFromUrl (link-local SSRF)" "kad.bootstrapFromUrl" @{ url = "http://169.254.169.254/x" } "err" | Out-Null
    Test-Rpc $ws "B.6" "kad.bootstrapFromUrl (esquema file)" "kad.bootstrapFromUrl" @{ url = "file:///etc/passwd" } "err" | Out-Null

    $ipf = Write-BinFile (Join-Path $FixDir "ipfilter.dat") (New-IpfilterDat)
    Test-Rpc $ws "B.6" "filter.importFile" "filter.importFile" @{ filePath = $ipf } "any" | Out-Null
    Test-Rpc $ws "B.6" "filter.getStatus" "filter.getStatus" | Out-Null
    Test-Rpc $ws "B.6" "filter.setEnabled(true)" "filter.setEnabled" @{ enabled = $true } "any" | Out-Null
    Test-Rpc $ws "B.6" "filter.setEnabled(false)" "filter.setEnabled" @{ enabled = $false } "any" | Out-Null
    Test-Rpc $ws "B.6" "filter.clear" "filter.clear" | Out-Null
    Test-Rpc $ws "B.6" "network.getPolicy" "network.getPolicy" -Validate {
        param($r) if (!$r.profile) { "falta profile" }
    } | Out-Null
    Test-Rpc $ws "B.6" "network.setProfile (LocalLab)" "network.setProfile" @{ profile = "LocalLab" } | Out-Null
    Test-Rpc $ws "B.6" "network.getPolicy (aislamiento)" "network.getPolicy" -Validate {
        param($r) if ($r.profile -notmatch 'LocalLab|local') { "se esperaba LocalLab, recibido $($r.profile)" }
    } | Out-Null
    Test-Rpc $ws "B.6" "network.setProfile (inválido)" "network.setProfile" @{ profile = "TotalChaos" } "err" | Out-Null
    Test-Rpc $ws "B.6" "network.getAuditLog" "network.getAuditLog" | Out-Null
    Test-Rpc $ws "B.6" "network.setBudgets" "network.setBudgets" @{ budgets = @{} } "any" | Out-Null
    Test-Rpc $ws "B.6" "network.disconnectAll" "network.disconnectAll" | Out-Null
    Test-Rpc $ws "B.6" "network.enable" "network.enable" | Out-Null
    Test-Rpc $ws "B.6" "listener.status" "listener.status" | Out-Null
    Test-Rpc $ws "B.6" "listener.stop" "listener.stop" @{} "any" | Out-Null
    Test-Rpc $ws "B.6" "listener.start" "listener.start" @{} "any" | Out-Null
    Test-Rpc $ws "B.6" "upnp.status" "upnp.status" | Out-Null
    Test-Rpc $ws "B.6" "upnp.setEnabled(false)" "upnp.setEnabled" @{ enabled = $false } "any" | Out-Null

    # ===================================================================
    # B.7 Kad
    # ===================================================================
    Write-Host "`n[B.7] Kad" -ForegroundColor Yellow
    Test-Rpc $ws "B.7" "kad.status" "kad.status" | Out-Null
    Test-Rpc $ws "B.7" "kad.listContacts" "kad.listContacts" | Out-Null
    Test-Rpc $ws "B.7" "kad.clearContacts" "kad.clearContacts" | Out-Null
    Test-Rpc $ws "B.7" "kad.startUdp" "kad.startUdp" @{} "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.bootstrap (nodo caído)" "kad.bootstrap" @{ ip = "127.0.0.1"; port = 1 } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.findNode" "kad.findNode" @{ targetId = "00112233445566778899AABBCCDDEEFF" } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.ping" "kad.ping" @{ ip = "127.0.0.1"; port = 4672 } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.searchKeywords" "kad.searchKeywords" @{ term = "bench" } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.searchSources" "kad.searchSources" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF" } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.searchNotes" "kad.searchNotes" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF" } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.publishSource" "kad.publishSource" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF"; fileName = "bench.bin"; fileSize = 1024 } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.publishKeywords" "kad.publishKeywords" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF"; keywords = @("bench") } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.publishNote" "kad.publishNote" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF"; fileName = "bench.bin"; fileSize = 1024; comment = "bench"; rating = 3 } "any" | Out-Null
    Test-Rpc $ws "B.7" "kad.stopUdp" "kad.stopUdp" @{} "any" | Out-Null

    # ===================================================================
    # B.8 Sistema / ajustes / categorías / amigos / historial
    # ===================================================================
    Write-Host "`n[B.8] Sistema / Ajustes / Supervisión" -ForegroundColor Yellow
    Test-Rpc $ws "B.8" "system.status" "system.status" -Validate {
        param($r) if ($null -eq $r.healthy -and $null -eq $r.status) { "falta campo de salud" }
    } | Out-Null
    Test-Rpc $ws "B.8" "settings.get" "settings.get" | Out-Null
    Test-Rpc $ws "B.8" "settings.set (nickname)" "settings.set" @{ nickname = "BenchBot" } | Out-Null
    Test-Rpc $ws "B.8" "settings.get (verificar nickname)" "settings.get" -Validate {
        param($r) if ($r.nickname -ne "BenchBot") { "nickname no persistido" }
    } | Out-Null
    Test-Rpc $ws "B.8" "settings.set (clave desconocida)" "settings.set" @{ bogusKey123 = 1 } "any" | Out-Null
    Test-Rpc $ws "B.8" "settings.setRetentionDays" "settings.setRetentionDays" @{ retentionDays = 30 } | Out-Null
    Test-Rpc $ws "B.8" "settings.resetDownloadDir" "settings.resetDownloadDir" @{} "any" | Out-Null
    Test-Rpc $ws "B.8" "limits.get" "limits.get" | Out-Null
    Test-Rpc $ws "B.8" "limits.set (1MB/s)" "limits.set" @{ downloadBytesPerSec = 1048576 } | Out-Null
    Test-Rpc $ws "B.8" "limits.get (verificar)" "limits.get" -Validate {
        param($r) if ($r.downloadBytesPerSec -gt 1572864) { "límite no aplicado" }
    } | Out-Null
    Test-Rpc $ws "B.8" "limits.set (ilimitado)" "limits.set" @{ downloadBytesPerSec = 0 } | Out-Null

    $cat = Test-Rpc $ws "B.8" "categories.create" "categories.create" @{ name = "BenchCat" }
    $catId = if ($cat) { $cat.id } else { $null }
    if (!$catId -and $cat) { $catId = $cat.category.id }
    # categories.list devuelve un array plano (sin envoltorio {categories})
    Test-Rpc $ws "B.8" "categories.list" "categories.list" -Validate {
        param($r) if (@($r).Count -lt 1) { "categoría no listada" }
    } | Out-Null
    if ($catId) {
        Test-Rpc $ws "B.8" "categories.update" "categories.update" @{ id = $catId; name = "BenchCat2" } "any" | Out-Null
        Test-Rpc $ws "B.8" "categories.reorder" "categories.reorder" @{ categoryIds = @($catId) } "any" | Out-Null
        Test-Rpc $ws "B.8" "categories.delete" "categories.delete" @{ id = $catId } "any" | Out-Null
    }
    Test-Rpc $ws "B.8" "friends.add" "friends.add" @{ userHash = "A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4"; name = "BenchFriend" } "any" | Out-Null
    Test-Rpc $ws "B.8" "friends.list" "friends.list" | Out-Null
    Test-Rpc $ws "B.8" "friends.remove" "friends.remove" @{ userHash = "A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4" } "any" | Out-Null
    Test-Rpc $ws "B.8" "history.list" "history.list" | Out-Null
    Test-Rpc $ws "B.8" "history.stats" "history.stats" | Out-Null
    Test-Rpc $ws "B.8" "history.export (json)" "history.export" @{ format = "json" } "any" | Out-Null
    Test-Rpc $ws "B.8" "history.clear" "history.clear" | Out-Null
    Test-Rpc $ws "B.8" "stats.resetLifetimeTotals" "stats.resetLifetimeTotals" @{} "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.status" "servers.status" | Out-Null
    Test-Rpc $ws "B.8" "servers.setEnabled(false)" "servers.setEnabled" @{ address = "127.0.0.1:4661"; enabled = $false } "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.search (sin conexión)" "servers.search" @{ term = "bench" } "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.querySources" "servers.querySources" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF" } "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.connect (caído)" "servers.connect" @{ address = "127.0.0.1:1" } "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.disconnect" "servers.disconnect" @{} "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.delete" "servers.delete" @{ address = "195.154.122.10:4661" } "any" | Out-Null
    Test-Rpc $ws "B.8" "servers.clear" "servers.clear" @{} "any" | Out-Null
    Test-Rpc $ws "B.8" "lab.status" "lab.status" | Out-Null

    # ===================================================================
    # Cierre limpio
    # ===================================================================
    Write-Host "`n[Cierre] system.stop" -ForegroundColor Yellow
    Test-Rpc $ws "Cierre" "system.stop" "system.stop" @{} | Out-Null
    try { $ws.Dispose() } catch {}
    $daemonProcess.WaitForExit(8000) | Out-Null
    if (!$daemonProcess.HasExited) {
        Stop-Process -Id $daemonProcess.Id -Force
        Add-Result "Cierre" "salida del demonio" "FAIL" "no terminó en 8 s"
    } else {
        Add-Result "Cierre" "código de salida del demonio $($daemonProcess.ExitCode)" ($(if ($daemonProcess.ExitCode -eq 0) {"PASS"} else {"FAIL"}))
    }
}
catch {
    Write-Host "`nFATAL: $($_.Exception.Message)" -ForegroundColor Red
    Add-Result "FATAL" "prueba abortada" "FAIL" $_.Exception.Message
    $exitCode = 1
}
finally {
    try { if ($ws) { $ws.Dispose() } } catch {}
    if ($daemonProcess -and !$daemonProcess.HasExited) {
        Stop-Process -Id $daemonProcess.Id -Force -ErrorAction SilentlyContinue
    }
}

# ===================================================================
# Informe final
# ===================================================================
$pass = ($script:Results | Where-Object { $_.Outcome -eq "PASS" }).Count
$fail = ($script:Results | Where-Object { $_.Outcome -eq "FAIL" }).Count

Write-Host "`n==================================================" -ForegroundColor Cyan
Write-Host "  INFORME FASE B                                  " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan
$script:Results | Group-Object Section | ForEach-Object {
    $p = ($_.Group | Where-Object { $_.Outcome -eq "PASS" }).Count
    $f = ($_.Group | Where-Object { $_.Outcome -eq "FAIL" }).Count
    Write-Host ("  {0,-10} PASS={1,-3} FAIL={2}" -f $_.Name, $p, $f)
}
Write-Host "  ------------------------------------------"
Write-Host ("  TOTAL: {0} PASS / {1} FAIL" -f $pass, $fail) -ForegroundColor ($(if ($fail -eq 0) {"Green"} else {"Red"}))
$script:Results | Where-Object { $_.Outcome -eq "FAIL" } | ForEach-Object {
    Write-Host ("  FAIL: [{0}] {1} - {2}" -f $_.Section, $_.Name, $_.Detail) -ForegroundColor Red
}

if ($fail -gt 0) { $exitCode = 1 }
Write-Host ("`n  BANCO FUNCIONAL FASE B: {0}" -f $(if ($exitCode -eq 0) {"APROBADO"} else {"FALLIDO"})) -ForegroundColor ($(if ($exitCode -eq 0) {"Green"} else {"Red"}))
exit $exitCode
