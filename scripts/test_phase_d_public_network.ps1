# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

<#
.SYNOPSIS
    Fase D - Validación supervisada de red real (docs/qa/PLAN_TEST_COMPLET.md).
.DESCRIPTION
    Extrae el paquete STANDARD (sin herramientas lab.*), inicia el demonio en
    perfil público predeterminado (UPnP activo) y ejercita la red real:
      D.1  bootstrap de server.met vía URL pública real (anti-SSRF
           permite tráfico público en perfil PublicEd2kKadPreview)
      D.2  conexión TCP a un servidor eD2K real + HighID/LowID
      D.3  búsqueda global en servidor
      D.4  bootstrap Kad vía nodes.dat real + contactos + búsqueda
      D.5  interruptor de corte (kill switch): corte limpio y reactivación
      D.6  parada limpia + desmapeo UPnP acotado
    ATENCIÓN: este script EMITE TRÁFICO REAL (servidores eD2K públicos,
    Kad DHT, pasarela UPnP). Ejecutar únicamente en sesiones supervisadas.
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\test_phase_d_public_network.ps1
#>

$ErrorActionPreference = "Stop"
$env:MULE_NO_TRAY = "1"  # Sin icono de bandeja en pruebas automatizadas
$Root = Split-Path -Parent $PSScriptRoot

# URLs canónicas eMule (upd.emule-security.org, mantenidas por la
# comunidad eMule Security).
$ServerMetUrl = "https://upd.emule-security.org/server.met"
$NodesDatUrl  = "https://upd.emule-security.org/nodes.dat"

$script:Results = New-Object System.Collections.Generic.List[object]
$script:ReqId = 2000

function Add-Result([string]$Section, [string]$Name, [string]$Outcome, [string]$Detail = "") {
    $script:Results.Add([pscustomobject]@{
        Section = $Section; Name = $Name; Outcome = $Outcome; Detail = $Detail
    })
    $color = switch ($Outcome) { "PASS" { "Green" } "FAIL" { "Red" } "SKIP" { "DarkYellow" } default { "Yellow" } }
    $detailStr = if ($Detail) { "  ($Detail)" } else { "" }
    Write-Host ("  [{0}] {1}{2}" -f $Outcome, $Name, $detailStr) -ForegroundColor $color
}

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

function Invoke-WsRpc([System.Net.WebSockets.ClientWebSocket]$ws, [hashtable]$request, [int]$timeoutMs = 15000) {
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
    throw "Tiempo de espera agotado para respuesta JSON-RPC id $reqId tras $timeoutMs ms"
}

function Test-Rpc {
    param(
        [System.Net.WebSockets.ClientWebSocket]$Ws,
        [string]$Section, [string]$Name, [string]$Method,
        [hashtable]$Params = @{},
        [string]$Expect = "ok",
        [scriptblock]$Validate = $null,
        [int]$TimeoutMs = 15000
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
                Add-Result $Section $Name "FAIL" "se esperaba error y llegó resultado"
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

function Connect-DaemonWebSocket([string]$endpoint, [string]$token) {
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $ws.Options.SetRequestHeader("Authorization", "Bearer $token")
    $ws.ConnectAsync([System.Uri]::new($endpoint), [System.Threading.CancellationTokenSource]::new(5000).Token).Wait()
    return $ws
}

# =======================================================================
# 0. Entorno aislado y arranque (paquete STANDARD, perfil público por defecto)
# =======================================================================
Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Fase D - Red real supervisada (eD2K + Kad)      " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  ATENCIÓN: tráfico de red real hacia servidores eD2K/Kad públicos" -ForegroundColor Yellow

$zip = Get-ChildItem (Join-Path $Root "dist") -Filter "PulseCore-Server-Windows-x64.zip" |
       Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (!$zip) { throw "No se encontró el ZIP estándar en dist/; ejecuta primero build_windows_server.ps1" }

$WorkRoot = Join-Path $env:TEMP ("eMuleRust-PhaseD-" + (New-Guid).ToString("N").Substring(0,8))
$BundleDir = Join-Path $WorkRoot "bundle"
$WorkDir = Join-Path $WorkRoot "workdir"
New-Item -ItemType Directory -Force -Path $BundleDir, $WorkDir | Out-Null
Expand-Archive -Path $zip.FullName -DestinationPath $BundleDir -Force
$DaemonBin = Join-Path $BundleDir "mule-daemon.exe"
if (!(Test-Path $DaemonBin)) { throw "No se encontró mule-daemon.exe en el paquete extraído" }
Write-Host " -> Paquete : $($zip.Name)" -ForegroundColor Gray
Write-Host " -> Entorno : $WorkRoot" -ForegroundColor Gray

$daemonProcess = $null
$ws = $null
$exitCode = 0

try {
    # Arranque: sin --profile (público por defecto), UPnP activo.
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $DaemonBin
    $psi.Arguments = "--workdir `"$WorkDir`" --mode portable"
    $psi.WorkingDirectory = (Split-Path -Parent $DaemonBin)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $daemonProcess = [System.Diagnostics.Process]::Start($psi)
    # Drenaje continuo: evita el bloqueo de tubería (~64 KiB).
    $script:DaemonStdoutTask = $daemonProcess.StandardOutput.ReadToEndAsync()
    $script:DaemonStderrTask = $daemonProcess.StandardError.ReadToEndAsync()

    $sessionPath = Join-Path $WorkDir "session.json"
    $t = 0
    while (!(Test-Path $sessionPath) -and $t -lt 15) {
        Start-Sleep -Milliseconds 200; $t += 0.2
        if ($daemonProcess.HasExited) { throw "El demonio finalizó durante el arranque ($($daemonProcess.ExitCode))" }
    }
    if (!(Test-Path $sessionPath)) { throw "Tiempo de espera agotado esperando session.json" }

    $session = Get-Content $sessionPath -Raw | ConvertFrom-Json
    $endpoint = $session.endpoint
    $token = (Get-Content (Join-Path $WorkDir "auth.token") -Raw).Trim()
    Write-Host " -> Demonio activo (PID $($daemonProcess.Id)) en $endpoint" -ForegroundColor Gray

    $ws = Connect-DaemonWebSocket $endpoint $token
    # La autenticación se valida mediante el handshake hello (protocolVersion + token).
    $hello = Test-Rpc $ws "D.0" "system.hello" "system.hello" @{ protocolVersion = 1; token = $token }
    if (!$hello) { throw "Handshake rechazado; abortando prueba de red pública" }

    $status = Test-Rpc $ws "D.0" "system.status (perfil público)" "system.status" -Validate {
        param($r) if ($r.networkProfile -notmatch 'public') { "perfil inesperado: $($r.networkProfile)" }
    }

    # ===================================================================
    # D.1 Bootstrap server.met vía URL pública real
    # ===================================================================
    Write-Host "`n[D.1] Importación server.met (URL pública real)" -ForegroundColor Yellow
    $upd = Test-Rpc $ws "D.1" "servers.updateFromUrl (real)" "servers.updateFromUrl" @{ url = $ServerMetUrl } -TimeoutMs 30000
    $srvList = Test-Rpc $ws "D.1" "servers.list" "servers.list" -Validate {
        param($r) if ($r.servers.Count -lt 1) { "sin servidores importados" }
    }
    if ($upd) { Write-Host "     -> importados=$($upd.imported) filtrados=$($upd.filtered)" -ForegroundColor Gray }

    # ===================================================================
    # D.2 Conexión TCP a servidor eD2K real + ID
    # ===================================================================
    Write-Host "`n[D.2] Conexión a servidor eD2K real" -ForegroundColor Yellow
    $serverAddr = $null
    if ($srvList -and $srvList.servers.Count -gt 0) {
        $serverAddr = $srvList.servers[0].address
        Write-Host "     -> probando servidor: $($srvList.servers[0].name) ($serverAddr)" -ForegroundColor Gray
    }
    if ($serverAddr) {
        Test-Rpc $ws "D.2" "servers.connect (real)" "servers.connect" @{ address = $serverAddr } -TimeoutMs 30000 -Expect "any" | Out-Null
        # Sondeo hasta Connected (handshake TCP + login eD2K ~10-30 s)
        $connected = $false
        for ($i = 0; $i -lt 12 -and !$connected; $i++) {
            Start-Sleep -Seconds 5
            $snap = $null
            try { $snap = (Invoke-WsRpc $ws @{ jsonrpc="2.0"; id=(++$script:ReqId); method="servers.status"; params=@{} } 10000).result } catch {}
            if ($snap -and $snap.status -eq "Connected") { $connected = $true }
            elseif ($snap) { Write-Host "     -> estado: $($snap.status)" -ForegroundColor Gray }
        }
        if ($connected) {
            $cid = $snap.clientId
            $idKind = if ($cid -ge 16777216) { "HighID" } else { "LowID" }
            Add-Result "D.2" "server session Connected" "PASS" "$idKind (clientId=$cid, servidor=$($snap.server_address))"
            Test-Rpc $ws "D.2" "servers.querySources (real)" "servers.querySources" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF" } -Expect "any" -TimeoutMs 20000 | Out-Null
        } else {
            Add-Result "D.2" "server session Connected" "FAIL" "no conectado tras 60 s (último: $($snap.status))"
        }
    } else {
        Add-Result "D.2" "server session Connected" "SKIP" "sin servidores en lista"
    }

    # ===================================================================
    # D.3 Búsqueda global en servidor
    # ===================================================================
    Write-Host "`n[D.3] Búsqueda global" -ForegroundColor Yellow
    if ($connected) {
        $sr = Test-Rpc $ws "D.3" "servers.search (real)" "servers.search" @{ term = "ubuntu" } -Expect "any" -TimeoutMs 30000
        if ($sr -and $sr.result) { Write-Host "     -> resultados recibidos" -ForegroundColor Gray }
    } else {
        Add-Result "D.3" "servers.search (real)" "SKIP" "no conectado"
    }

    # ===================================================================
    # D.4 Kad: bootstrap nodes.dat real + contactos + búsqueda
    # ===================================================================
    Write-Host "`n[D.4] Kad bootstrap + búsqueda" -ForegroundColor Yellow
    Test-Rpc $ws "D.4" "kad.bootstrapFromUrl (real)" "kad.bootstrapFromUrl" @{ url = $NodesDatUrl } -TimeoutMs 30000 -Expect "any" | Out-Null
    Test-Rpc $ws "D.4" "kad.startUdp" "kad.startUdp" @{} -Expect "any" | Out-Null
    # Bootstrap activo hacia el primer contacto importado (requiere ip+puerto)
    $contact = Test-Rpc $ws "D.4" "kad.listContacts" "kad.listContacts" -Expect "any"
    if ($contact -and $contact.contacts -and $contact.contacts.Count -gt 0) {
        $c0 = $contact.contacts[0]
        Write-Host "     -> bootstrap hacia $($c0.ip):$($c0.udpPort)" -ForegroundColor Gray
        Test-Rpc $ws "D.4" "kad.bootstrap (contacto real)" "kad.bootstrap" @{ ip = $c0.ip; port = $c0.udpPort } -Expect "any" | Out-Null
    }
    $kadOk = $false
    for ($i = 0; $i -lt 12 -and !$kadOk; $i++) {
        Start-Sleep -Seconds 5
        $ks = $null
        try { $ks = (Invoke-WsRpc $ws @{ jsonrpc="2.0"; id=(++$script:ReqId); method="kad.status"; params=@{} } 10000).result } catch {}
        if ($ks) {
            Write-Host "     -> kad.status: udp=$($ks.isUdpActive) contactos=$($ks.totalContacts) fw=$($ks.udpFirewallStatus) tx=$($ks.activeTransactions)" -ForegroundColor Gray
            if ($ks.totalContacts -gt 0) { $kadOk = $true }
        }
    }
    if ($kadOk) {
        Add-Result "D.4" "kad contacts > 0" "PASS"
        Test-Rpc $ws "D.4" "kad.searchKeywords (real)" "kad.searchKeywords" @{ term = "ubuntu" } -Expect "any" -TimeoutMs 30000 | Out-Null
        Test-Rpc $ws "D.4" "kad.searchSources (real)" "kad.searchSources" @{ fileHash = "0123456789ABCDEF0123456789ABCDEF"; fileSize = 1024 } -Expect "any" -TimeoutMs 30000 | Out-Null
    } else {
        Add-Result "D.4" "kad contacts > 0" "FAIL" "sin contactos tras 60 s"
    }

    # ===================================================================
    # D.5 Interruptor de corte (kill switch): corte y reactivación
    # ===================================================================
    Write-Host "`n[D.5] Interruptor de corte" -ForegroundColor Yellow
    # El kill switch de producción es network.disconnectAll / network.enable.
    Test-Rpc $ws "D.5" "network.disconnectAll" "network.disconnectAll" @{} | Out-Null
    Start-Sleep -Seconds 3
    Test-Rpc $ws "D.5" "servers.status (tras disconnectAll)" "servers.status" -Validate {
        param($r) if ($r.status -eq "Connected") { "sigue conectado tras disconnectAll" }
    } | Out-Null
    Test-Rpc $ws "D.5" "network.enable" "network.enable" @{} | Out-Null

    # ===================================================================
    # D.6 Estado UPnP + parada limpia (desmapeo acotado 500 ms)
    # ===================================================================
    Write-Host "`n[D.6] UPnP + parada" -ForegroundColor Yellow
    $upnp = Test-Rpc $ws "D.6" "upnp.status" "upnp.status" -Expect "any"
    if ($upnp) { Write-Host "     -> upnp: $($upnp | ConvertTo-Json -Compress -Depth 3)" -ForegroundColor Gray }
    Test-Rpc $ws "D.6" "system.stop" "system.stop" @{} | Out-Null
    try { $ws.Dispose() } catch {}
    $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
    $daemonProcess.WaitForExit(15000) | Out-Null
    if (!$daemonProcess.HasExited) {
        Stop-Process -Id $daemonProcess.Id -Force
        Add-Result "D.6" "daemon exit" "FAIL" "no salió en 15 s"
    } else {
        Add-Result "D.6" "daemon exit code $($daemonProcess.ExitCode) en $($sw2.ElapsedMilliseconds)ms" ($(if ($daemonProcess.ExitCode -eq 0) {"PASS"} else {"FAIL"}))
    }
    if (Test-Path $sessionPath) { Add-Result "D.6" "session.json cleanup" "FAIL" "session.json sigue presente" }
    else { Add-Result "D.6" "session.json cleanup" "PASS" }
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
    # Volcar stderr en caso de fallo para diagnóstico
    if ($script:DaemonStderrTask -and $script:DaemonStderrTask.IsCompleted) {
        $err = $script:DaemonStderrTask.Result
        if ($err) {
            $logPath = Join-Path $WorkRoot "daemon-stderr.log"
            [System.IO.File]::WriteAllText($logPath, $err)
            Write-Host " -> registro stderr del demonio: $logPath" -ForegroundColor Gray
        }
    }
}

# ===================================================================
# Informe final
# ===================================================================
$pass = ($script:Results | Where-Object { $_.Outcome -eq "PASS" }).Count
$fail = ($script:Results | Where-Object { $_.Outcome -eq "FAIL" }).Count
$skip = ($script:Results | Where-Object { $_.Outcome -eq "SKIP" }).Count

Write-Host "`n==================================================" -ForegroundColor Cyan
Write-Host "  INFORME FASE D                                  " -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan
$script:Results | Group-Object Section | ForEach-Object {
    $p = ($_.Group | Where-Object { $_.Outcome -eq "PASS" }).Count
    $f = ($_.Group | Where-Object { $_.Outcome -eq "FAIL" }).Count
    $s = ($_.Group | Where-Object { $_.Outcome -eq "SKIP" }).Count
    Write-Host ("  {0,-10} PASS={1,-3} FAIL={2,-3} SKIP={3}" -f $_.Name, $p, $f, $s)
}
Write-Host "  ------------------------------------------"
Write-Host ("  TOTAL: {0} PASS / {1} FAIL / {2} SKIP" -f $pass, $fail, $skip) -ForegroundColor ($(if ($fail -eq 0) {"Green"} else {"Red"}))
$script:Results | Where-Object { $_.Outcome -eq "FAIL" } | ForEach-Object {
    Write-Host ("  FAIL: [{0}] {1} - {2}" -f $_.Section, $_.Name, $_.Detail) -ForegroundColor Red
}

if ($fail -gt 0) { $exitCode = 1 }
Write-Host ("`n  FASE D RED PÚBLICA: {0}" -f $(if ($exitCode -eq 0) {"APROBADA"} else {"FALLIDA"})) -ForegroundColor ($(if ($exitCode -eq 0) {"Green"} else {"Red"}))
exit $exitCode
