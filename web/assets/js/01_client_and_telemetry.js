// Pulse Core WebUI — Cliente JSON-RPC/WebSocket (MuleClient), registros en vivo, modo tortuga y telemetría de almacenamiento.
// Extraído de web/index.html.

class MuleClient {
  constructor() {
    this.ws = null;
    this.msgId = 1;
    this.pendingRequests = new Map();
    this.eventListeners = new Map();
    this.reconnectTimer = null;
    this.requiresPassword = Boolean(window.MULE_SESSION && window.MULE_SESSION.requiresPassword);
    this.awaitingPassword = false;
    this.token = this.requiresPassword
      ? (sessionStorage.getItem('mule_webui_token') || '')
      : (localStorage.getItem('mule_token') || '');
  }

  connect() {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) {
      return;
    }
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    let endpoint = `${protocol}//${location.host}/rpc`;
    this.requiresPassword = Boolean(window.MULE_SESSION && window.MULE_SESSION.requiresPassword);
    if (window.MULE_SESSION && window.MULE_SESSION.endpoint) {
      let ep = window.MULE_SESSION.endpoint;
      if (ep.startsWith('/')) {
        endpoint = `${protocol}//${location.host}${ep}`;
      } else if (ep.includes('0.0.0.0')) {
        endpoint = ep.replace('0.0.0.0', location.hostname);
      } else {
        endpoint = ep;
      }
      if (!this.requiresPassword && window.MULE_SESSION.token) {
        this.token = window.MULE_SESSION.token;
      }
    }

    if (this.requiresPassword && !this.token) {
      this.awaitingPassword = true;
      document.getElementById('ws-indicator').className = 'indicator';
      document.getElementById('ws-label').textContent = '🔒 Requiere contraseña';
      if (typeof openWebuiLoginModal === 'function') {
        openWebuiLoginModal();
      }
      return;
    }
    this.awaitingPassword = false;

    document.getElementById('ws-label').textContent = 'Conectando...';
    try {
      this.ws = new WebSocket(endpoint);
    } catch (err) {
      console.error('WebSocket creation error:', err);
      return;
    }

    this.ws.onopen = async () => {
      document.getElementById('ws-indicator').className = 'indicator online';
      document.getElementById('ws-label').textContent = 'Conectado';
      try {
        const helloRes = await this.call('system.hello', { protocolVersion: 1, token: this.token });
        if (helloRes) {
          if (helloRes.workdirPath) document.getElementById('cfg-workdir').value = helloRes.workdirPath;
          if (helloRes.daemonVersion) {
            document.getElementById('cfg-version').value = helloRes.daemonVersion;
            const updateBadge = document.getElementById('update-current-ver-badge');
            if (updateBadge) updateBadge.textContent = `Versión instalada: v${helloRes.daemonVersion}`;
            const brandVer = document.getElementById('brand-version-pill');
            if (brandVer) brandVer.textContent = `v${helloRes.daemonVersion}`;
            const statusVer = document.getElementById('statusbar-version-label');
            if (statusVer) statusVer.textContent = `v${helloRes.daemonVersion}`;
          }
          if (typeof window._updateEnvFromHello === 'function') {
            window._updateEnvFromHello(helloRes);
          }
        }
        this.trigger('connected');
      } catch (e) {
        console.error('Handshake error:', e);
        if (e.message && /Auth|Unauthorized|Token/i.test(e.message)) {
          sessionStorage.removeItem('mule_webui_token');
          localStorage.removeItem('mule_token');
          this.token = '';
          this.awaitingPassword = true;
          try { this.ws.close(); } catch (err) { console.debug('[PulseCore] cierre WS:', err); }
          if (typeof openWebuiLoginModal === 'function') {
            openWebuiLoginModal();
          }
        }
      }
    };

    this.ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.id && this.pendingRequests.has(data.id)) {
          const { resolve, reject } = this.pendingRequests.get(data.id);
          this.pendingRequests.delete(data.id);
          if (data.error) reject(data.error);
          else resolve(data.result);
        } else if (data.method) {
          this.trigger(data.method, data.params);
        }
      } catch (e) {
        console.error('Failed to parse WS msg:', e);
      }
    };

    this.ws.onclose = () => {
      // Rechazar peticiones en vuelo para evitar promesas colgadas tras desconexión
      for (const [, pending] of this.pendingRequests.entries()) {
        pending.reject(new Error('WebSocket desconectado'));
      }
      this.pendingRequests.clear();

      document.getElementById('ws-indicator').className = 'indicator';
      document.getElementById('ws-label').textContent = this.awaitingPassword ? '🔒 Requiere contraseña' : 'Desconectado';
      if (!this.awaitingPassword && !this.reconnectTimer) {
        this.reconnectTimer = setTimeout(() => {
          this.reconnectTimer = null;
          this.connect();
        }, 2500);
      }
    };
  }

  call(method, params = {}) {
    return new Promise((resolve, reject) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        return reject(new Error('WebSocket not connected'));
      }
      const id = this.msgId++;
      this.pendingRequests.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
    });
  }

  on(event, cb) {
    if (!this.eventListeners.has(event)) this.eventListeners.set(event, []);
    this.eventListeners.get(event).push(cb);
  }

  trigger(event, data) {
    const cbs = this.eventListeners.get(event) || [];
    cbs.forEach(cb => cb(data));
  }
}

const client = new MuleClient();

function formatBytes(bytes) {
  if (!bytes || bytes === 0) return '0 B';
  const k = 1024, sizes = ['B', 'kB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
}
function formatSpeed(bytesPerSec) {
  return formatBytes(bytesPerSec) + '/s';
}
function copyInput(id) {
  const el = document.getElementById(id);
  if (el) {
    el.select();
    navigator.clipboard.writeText(el.value);
    alert('Copiado al portapapeles: ' + el.value);
  }
}

// === SISTEMA DE REGISTRO DEL SISTEMA (LIVE & HISTORICAL LOGS) ===
let autoScrollLogs = true;
let currentLogFilter = 'all';
let logSearchQuery = '';
let isFetchingLogs = false;

async function refreshSystemLogs() {
  if (isFetchingLogs) return;
  isFetchingLogs = true;
  try {
    const params = { limit: 300 };
    if (currentLogFilter !== 'all') params.level = currentLogFilter;
    if (logSearchQuery.trim()) params.search = logSearchQuery.trim();
    const res = await client.call('system.logs', params);
    if (res && Array.isArray(res.logs)) {
      const container = document.getElementById('log-console-container');
      if (!container) return;
      let html = '';
      res.logs.forEach(entry => {
        const time = entry.timestamp ? (entry.timestamp.split('T')[1] || '').split('+')[0] : '';
        const lvl = (entry.level || 'INFO').toLowerCase();
        const targetShort = (entry.target || '').replace('mule_daemon::', '');
        html += `
          <div class="log-entry" style="font-family: monospace; font-size: 0.8rem; line-height: 1.45; display: flex; gap: 8px;">
            <span class="log-time" style="color: var(--text-muted); flex-shrink: 0;">[${time}]</span>
            <span class="log-lvl ${lvl}" style="font-weight: 700; flex-shrink: 0; min-width: 58px; text-align: center; border-radius: 3px; font-size: 0.72rem; padding: 1px 4px;">[${lvl.toUpperCase()}]</span>
            <span style="color: #64748b; font-size: 0.75rem; min-width: 130px; flex-shrink: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${escapeHtml(targetShort)}">[${escapeHtml(targetShort)}]</span>
            <span class="log-msg" style="color: var(--text); word-break: break-word;">${escapeHtml(entry.message)}</span>
          </div>
        `;
      });
      if (res.logs.length === 0) {
        html = '<div style="color: var(--text-muted); padding: 25px; text-align: center;">No hay registros en el archivo para el filtro seleccionado.</div>';
      }
      container.innerHTML = html;
      if (autoScrollLogs) container.scrollTop = container.scrollHeight;
    }
  } catch (err) {
    console.warn('Error al obtener logs del sistema:', err);
  } finally {
    isFetchingLogs = false;
  }
}

function addLog(level, message, category = '') {
  const container = document.getElementById('log-console-container');
  if (!container) return;
  const now = new Date();
  const timeStr = now.toTimeString().split(' ')[0] + '.' + String(now.getMilliseconds()).padStart(3, '0');
  const div = document.createElement('div');
  div.className = 'log-entry';
  div.style.cssText = 'font-family: monospace; font-size: 0.8rem; line-height: 1.45; display: flex; gap: 8px;';
  div.innerHTML = `
    <span class="log-time" style="color: var(--text-muted); flex-shrink: 0;">[${timeStr}]</span>
    <span class="log-lvl ${level.toLowerCase()}" style="font-weight: 700; flex-shrink: 0; min-width: 58px; text-align: center; border-radius: 3px; font-size: 0.72rem; padding: 1px 4px;">[${level.toUpperCase()}]</span>
    <span style="color: #38bdf8; font-size: 0.75rem; min-width: 130px; flex-shrink: 0;">[ui::event]</span>
    <span class="log-msg" style="color: var(--text); word-break: break-word;">${escapeHtml(message)}</span>
  `;
  container.appendChild(div);
  if (autoScrollLogs) container.scrollTop = container.scrollHeight;
}

function escapeHtml(str) {
  return String(str || '').replace(/[&<>"']/g, m => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  })[m]);
}

function copyInspectorHash() {
  const h = document.getElementById('insp-hash').textContent;
  if (h && h !== '-') {
    navigator.clipboard.writeText(h);
    alert('Hash copiado al portapapeles: ' + h);
  }
}

// === MODO TORTUGA (LÍMITES RÁPIDOS ALTERNATIVOS) ===
let turtleModeActive = localStorage.getItem('mule_turtle_mode') === 'true';

function updateTurtleModeUI() {
  const btn = document.getElementById('btn-turtle-mode');
  const lbl = document.getElementById('turtle-status-label');
  const badge = document.getElementById('turtle-tt-badge');
  const desc = document.getElementById('turtle-tt-desc');
  const bwBtn = document.getElementById('btn-bw-turtle-toggle');
  if (bwBtn) {
    bwBtn.textContent = turtleModeActive ? '🐢 Modo Tortuga: ON' : '🐢 Modo Tortuga: OFF';
    bwBtn.classList.toggle('btn-primary', turtleModeActive);
  }
  if (!btn) return;
  if (turtleModeActive) {
    btn.classList.add('turtle-active');
    btn.title = 'Modo Tortuga ACTIVO: Límites de 500 kB/s bajada y 100 kB/s subida (clic para desactivar)';
    if (lbl) lbl.textContent = 'Tortuga: ON (500k/100k)';
    if (badge) {
      badge.textContent = 'ACTIVO (500k/100k)';
      badge.style.background = 'rgba(63, 185, 80, 0.25)';
      badge.style.color = '#3fb950';
      badge.style.border = '1px solid #3fb950';
    }
    if (desc) {
      desc.innerHTML = 'Velocidad limitada a <b style="color:#38bdf8;">500 kB/s</b> bajada y <b style="color:#34d399;">100 kB/s</b> subida. Haz clic para restablecer la velocidad máxima.';
    }
  } else {
    btn.classList.remove('turtle-active');
    btn.title = 'Modo Tortuga: Limita temporalmente el ancho de banda a 500 kB/s bajada y 100 kB/s subida (clic para activar)';
    if (lbl) lbl.textContent = 'Tortuga: OFF';
    if (badge) {
      badge.textContent = 'Inactivo';
      badge.style.background = 'rgba(255, 255, 255, 0.08)';
      badge.style.color = 'var(--text-muted)';
      badge.style.border = 'none';
    }
    if (desc) {
      desc.textContent = 'Limita temporalmente la velocidad para no saturar tu conexión ni el NAS. Haz clic para activar.';
    }
  }
}

async function toggleTurtleMode() {
  turtleModeActive = !turtleModeActive;
  localStorage.setItem('mule_turtle_mode', turtleModeActive ? 'true' : 'false');
  updateTurtleModeUI();
  try {
    if (turtleModeActive) {
      await client.call('limits.set', { downloadLimit: 500, uploadLimit: 100, uss: { enabled: false } });
      addLog('warn', '🐢 Modo Tortuga activado: Ancho de banda limitado a 500 kB/s bajada y 100 kB/s subida.');
    } else {
      await client.call('limits.set', { downloadLimit: 0, uploadLimit: 0 });
      addLog('info', '🐢 Modo Tortuga desactivado: Restablecida velocidad máxima sin límites.');
    }
    await refreshBandwidthLimits();
  } catch (e) {
    addLog('error', 'Error configurando límites de velocidad: ' + (e.message || JSON.stringify(e)));
  }
}

// === MONITOR DE ESPACIO DE ALMACENAMIENTO Y TELEMETRÍA HOST / VOLÚMENES ===
let lastDiskSpaceInfo = null;
let lastNasTelemetryInfo = null;

function updateStatusMeterBar(elId, pct) {
  const bar = document.getElementById(elId);
  if (!bar) return;
  const clamped = Math.max(0, Math.min(100, Math.round(pct || 0)));
  bar.style.width = `${clamped}%`;
  if (clamped >= 88) {
    bar.style.background = '#fb7185';
  } else if (clamped >= 70) {
    bar.style.background = '#f59e0b';
  } else {
    bar.style.background = 'var(--primary)';
  }
}

function inferStorageLabel(pathStr, nfsInfo, fallbackLabel) {
  const fsKind = (nfsInfo && nfsInfo.fsKind ? String(nfsInfo.fsKind) : '').toLowerCase();
  if (nfsInfo && nfsInfo.isNetworkMount) {
    return 'NAS';
  }
  if (fsKind === 'nfs' || fsKind === 'smb' || fsKind === 'remote') {
    return 'NAS';
  }
  const p = String(pathStr || '').trim();
  const winDrive = p.match(/^([A-Za-z]):[\\/]/);
  if (winDrive) {
    return `Disco ${winDrive[1].toUpperCase()}:`;
  }
  const mountMatch =
    p.match(/^\/(?:mnt|volumes)\/([^/]+)/i) ||
    p.match(/^\/media\/[^/]+\/([^/]+)/i) ||
    p.match(/^\/run\/media\/[^/]+\/([^/]+)/i);
  if (mountMatch && mountMatch[1]) {
    const raw = mountMatch[1].trim();
    const lower = raw.toLowerCase();
    if (lower === 'nas') return 'NAS';
    if (lower === 'hdd' || lower === 'ssd' || lower === 'nvme') return raw.toUpperCase();
    return raw.charAt(0).toUpperCase() + raw.slice(1);
  }
  return fallbackLabel;
}

async function refreshStorage() {
  try {
    const res = await client.call('system.diskSpace');
    if (res) {
      lastDiskSpaceInfo = res;
      // 1. Telemetría del servidor host (CPU, RAM y almacenamiento principal) en la barra inferior
      if (res.host) {
        const hostNameEl = document.getElementById('host-name-label');
        if (hostNameEl && res.host.hostname) hostNameEl.textContent = res.host.hostname;
        const hostAddrEl = document.getElementById('host-addr-label');
        const currentHost = window.location.hostname || '127.0.0.1';
        if (hostAddrEl) hostAddrEl.textContent = currentHost;
        const arrHostEl = document.getElementById('arr-host-input');
        if (arrHostEl) arrHostEl.value = currentHost;

        const cpuPct = Number(res.host.cpuPct || 0);
        const cpuEl = document.getElementById('rpi-cpu-val');
        if (cpuEl) cpuEl.textContent = `${cpuPct}%`;
        updateStatusMeterBar('rpi-cpu-bar', cpuPct);

        const ramEl = document.getElementById('rpi-ram-val');
        if (ramEl && res.host.memTotalBytes) {
          const usedGb = (res.host.memUsedBytes / (1024 * 1024 * 1024)).toFixed(1);
          const totalGb = (res.host.memTotalBytes / (1024 * 1024 * 1024)).toFixed(1);
          const ramPct = (res.host.memUsedBytes / res.host.memTotalBytes) * 100;
          ramEl.textContent = `${usedGb}/${totalGb} GB`;
          updateStatusMeterBar('rpi-ram-bar', ramPct);
        }
      }

      const primaryVol = (res.incoming && res.incoming.freeBytes != null) ? res.incoming : res.dataDir;
      if (primaryVol && primaryVol.freeBytes != null) {
        const primaryLabel = inferStorageLabel(primaryVol.path, null, 'Local');
        const primaryLblEl = document.getElementById('primary-disk-label');
        if (primaryLblEl) primaryLblEl.textContent = primaryLabel;
        const diskEl = document.getElementById('rpi-disk-val');
        if (diskEl) diskEl.textContent = `${formatBytes(primaryVol.freeBytes)} libre`;
        const primaryPct = primaryVol.totalBytes
          ? ((primaryVol.totalBytes - primaryVol.freeBytes) / primaryVol.totalBytes) * 100
          : 0;
        updateStatusMeterBar('rpi-disk-bar', primaryPct);
        const ssdVal = document.getElementById('storage-val-ssd');
        if (ssdVal) ssdVal.textContent = `${formatBytes(primaryVol.freeBytes)} libre`;
        const hostMetricsEl = document.getElementById('header-rpi-metrics');
        if (hostMetricsEl && primaryVol.path) {
          const totalInfo = primaryVol.totalBytes ? ` de ${formatBytes(primaryVol.totalBytes)}` : '';
          hostMetricsEl.title = `Almacenamiento principal (${primaryVol.path}): ${formatBytes(primaryVol.freeBytes)} libres${totalInfo}`;
        }
      }

      // 2. Segundo volumen de almacenamiento (solo visible si es distinto al primario o es montaje en red)
      const nasTarget = res.nas && res.nas.freeBytes != null ? res.nas : null;
      const nasBox = document.getElementById('sidebar-nas-box');
      const nasSep = document.getElementById('nas-status-sep');
      const isNetMount = Boolean(nasTarget && nasTarget.nfs && nasTarget.nfs.isNetworkMount);
      const sameVolumeAsPrimary = Boolean(
        nasTarget &&
        !isNetMount &&
        primaryVol &&
        nasTarget.totalBytes &&
        primaryVol.totalBytes &&
        nasTarget.totalBytes === primaryVol.totalBytes &&
        Math.abs((nasTarget.freeBytes || 0) - (primaryVol.freeBytes || 0)) < 1024 * 1024 * 64
      );

      if (nasTarget && !sameVolumeAsPrimary) {
        const storageLabel = inferStorageLabel(nasTarget.path, nasTarget.nfs, 'Mediateca');
        const lblEl = document.getElementById('sidebar-storage-label');
        if (lblEl) lblEl.textContent = storageLabel;

        if (nasBox) nasBox.style.display = 'inline-flex';
        if (nasSep) nasSep.style.display = 'none';
        const freeStr = formatBytes(nasTarget.freeBytes);
        const totalStr = nasTarget.totalBytes ? formatBytes(nasTarget.totalBytes) : '';
        const pct = nasTarget.totalBytes ? Math.round(((nasTarget.totalBytes - nasTarget.freeBytes) / nasTarget.totalBytes) * 100) : 0;
        const sbFree = document.getElementById('sidebar-nas-free');
        if (sbFree) sbFree.textContent = `${freeStr} libre`;
        const sbTotal = document.getElementById('sidebar-nas-total');
        if (sbTotal && totalStr) sbTotal.textContent = `de ${totalStr}`;
        const sbPct = document.getElementById('sidebar-nas-pct');
        if (sbPct) sbPct.textContent = `${pct}%`;
        updateStatusMeterBar('sidebar-nas-bar', pct);
        if (nasBox) {
          nasBox.title = `${storageLabel} (${nasTarget.path || ''}): ${freeStr} libres${totalStr ? ' de ' + totalStr : ''}`;
        }
      } else {
        if (nasBox) nasBox.style.display = 'none';
        if (nasSep) nasSep.style.display = 'none';
      }
    }
  } catch (e) { console.warn('[PulseCore] error en telemetría:', e); }

  // Actualizar telemetría profunda del NAS en segundo plano
  refreshNasTelemetry();
}

async function refreshNasTelemetry(forceModalUpdate = false) {
  try {
    const res = await client.call('system.nasInfo');
    if (!res) return;
    lastNasTelemetryInfo = res;

    const pill = document.getElementById('storage-pill-nas');
    const pillDot = document.getElementById('nas-pill-dot');
    const pillVal = document.getElementById('storage-val-nas');
    const isNetMount = Boolean(lastDiskSpaceInfo && lastDiskSpaceInfo.nas && lastDiskSpaceInfo.nas.nfs && lastDiskSpaceInfo.nas.nfs.isNetworkMount);

    // Actualizar el widget secundario solo si existe montaje en red real o telemetría remota activa
    if (res.volume && (res.reachable || isNetMount)) {
      const freeStr = formatBytes(res.volume.freeBytes);
      const totalStr = formatBytes(res.volume.totalBytes);
      const pct = res.volume.usedPct != null ? res.volume.usedPct : (res.volume.totalBytes ? Math.round(((res.volume.totalBytes - res.volume.freeBytes) / res.volume.totalBytes) * 100) : 0);
      const lblEl = document.getElementById('sidebar-storage-label');
      if (lblEl) lblEl.textContent = 'NAS';
      const sbFree = document.getElementById('sidebar-nas-free');
      if (sbFree) sbFree.textContent = `${freeStr} libre`;
      const sbTotal = document.getElementById('sidebar-nas-total');
      if (sbTotal) sbTotal.textContent = `de ${totalStr}`;
      const sbPct = document.getElementById('sidebar-nas-pct');
      if (sbPct) sbPct.textContent = `${pct}%`;
      updateStatusMeterBar('sidebar-nas-bar', pct);
    }

    if (res.reachable) {
      if (pill) pill.style.display = 'flex';
      const cpu = res.cpu ? res.cpu.pctEstimated : 0;
      const isHighCpu = cpu >= 85 || (res.cpu && res.cpu.load1 > 2.0);
      if (pillDot) {
        pillDot.textContent = '🗄️';
      }
      const freeGb = res.volume ? formatBytes(res.volume.freeBytes) : '--';
      if (pillVal) {
        pillVal.dataset.hasTelemetry = 'true';
        pillVal.textContent = `NAS: ${freeGb} libre`;
        pillVal.style.color = '#e2e8f0';
      }
      const alertBadge = document.getElementById('nas-cpu-alert');
      if (alertBadge) {
        alertBadge.style.display = isHighCpu ? 'inline-block' : 'none';
        alertBadge.textContent = `⚠️ CPU ${cpu}%`;
      }
      if (pill) {
        pill.style.borderColor = isHighCpu ? 'rgba(245,158,11,0.5)' : '';
      }
    } else {
      if (pillDot) pillDot.textContent = '🗄️';
      if (res.volume && res.volume.freeBytes) {
        if (pillVal) {
          pillVal.dataset.hasTelemetry = 'true';
          pillVal.textContent = `NAS: ${formatBytes(res.volume.freeBytes)} lib (NFS)`;
        }
      }
    }

    // Si el modal está visible o se solicitó explícitamente:
    const modal = document.getElementById('nas-details-modal');
    if (modal && (modal.classList.contains('active') || forceModalUpdate)) {
      if (res.reachable) {
        document.getElementById('nas-modal-badge').textContent = res.stale ? '🟡 En caché' : '🟢 Conectado';
        document.getElementById('nas-modal-badge').style.color = res.stale ? '#f59e0b' : '#22c55e';
        
        // CPU
        const cpuPct = res.cpu ? res.cpu.pctEstimated : 0;
        document.getElementById('nas-cpu-pct').textContent = `${cpuPct}%`;
        document.getElementById('nas-cpu-load').textContent = res.cpu ? `Load: ${res.cpu.load1}, ${res.cpu.load5}, ${res.cpu.load15}` : '--';
        document.getElementById('nas-cpu-bar').style.width = `${cpuPct}%`;
        document.getElementById('nas-cpu-bar').style.background = cpuPct > 80 ? '#ef4444' : (cpuPct > 50 ? '#f59e0b' : '#3b82f6');

        // RAM
        if (res.memory) {
          const usedMb = Math.round(res.memory.usedBytes / 1048576);
          const totalMb = Math.round(res.memory.totalBytes / 1048576);
          const availMb = Math.round(res.memory.availableBytes / 1048576);
          document.getElementById('nas-ram-total').textContent = `Total: ${totalMb} MB`;
          document.getElementById('nas-ram-used').textContent = `${usedMb} MB`;
          document.getElementById('nas-ram-avail').textContent = `Disp: ${availMb} MB`;
          document.getElementById('nas-ram-pct').textContent = `${res.memory.usedPct}% en uso`;
          document.getElementById('nas-ram-bar').style.width = `${res.memory.usedPct}%`;
        }

        // Swap
        if (res.swap) {
          const sUsedMb = Math.round(res.swap.usedBytes / 1048576);
          const sTotalMb = Math.round(res.swap.totalBytes / 1048576);
          document.getElementById('nas-swap-info').textContent = `Swap: ${sUsedMb} / ${sTotalMb} MB (${res.swap.usedPct}%)`;
        }

        // Volumen
        if (res.volume) {
          const freeStr = formatBytes(res.volume.freeBytes);
          const totalStr = formatBytes(res.volume.totalBytes);
          const pct = res.volume.usedPct != null ? res.volume.usedPct : (res.volume.totalBytes ? Math.round(((res.volume.totalBytes - res.volume.freeBytes) / res.volume.totalBytes) * 100) : 0);
          const sbFree = document.getElementById('sidebar-nas-free');
          if (sbFree) sbFree.textContent = `${freeStr} lib`;
          const sbTotal = document.getElementById('sidebar-nas-total');
          if (sbTotal) sbTotal.textContent = `de ${totalStr}`;
          const sbPct = document.getElementById('sidebar-nas-pct');
          if (sbPct) sbPct.textContent = `${pct}%`;
          const sbBar = document.getElementById('sidebar-nas-bar');
          if (sbBar) sbBar.style.width = `${pct}%`;
          document.getElementById('nas-vol-free').textContent = `${freeStr} libres`;
          document.getElementById('nas-vol-total').textContent = `de ${formatBytes(res.volume.totalBytes)}`;
          document.getElementById('nas-vol-pct').textContent = `${pct}% usado`;
          document.getElementById('nas-vol-bar').style.width = `${pct}%`;
        }

        // RAID
        if (res.raid) {
          document.getElementById('nas-raid-status').textContent = res.raid.healthy ? 'RAID 1 (Espejo) Sincronizado [2/2 UU]' : 'Atención: ' + res.raid.status;
          document.getElementById('nas-raid-status').style.color = res.raid.healthy ? '#22c55e' : '#ef4444';
        }

        // Discos
        if (res.disks && res.disks.length >= 2) {
          if (res.disks[0].tempC != null) {
            document.getElementById('nas-disk0-temp').textContent = `🌡 ${res.disks[0].tempC} °C`;
          }
          if (res.disks[1].tempC != null) {
            document.getElementById('nas-disk1-temp').textContent = `🌡 ${res.disks[1].tempC} °C`;
          }
        }
      }
    }
  } catch (e) {
    console.warn('Error consultando telemetría NAS:', e);
  }
}

