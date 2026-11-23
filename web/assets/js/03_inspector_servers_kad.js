// Pulse Core WebUI — Panel inspector inferior, gestión de servidores eD2K y red Kademlia DHT.
// Extraído de web/index.html.

// === GESTIÓN DEL PANEL INSPECTOR INFERIOR ===
window.selectDownload = (id) => {
  selectedDownloadId = id;
  const inspector = document.getElementById('download-inspector');
  inspector.classList.add('active');
  document.querySelectorAll('#downloads-list tr').forEach(r => {
    r.classList.toggle('selected-row', r.dataset.id === id);
  });
  refreshInspector(true);
};

window.closeInspector = () => {
  selectedDownloadId = null;
  document.getElementById('download-inspector').classList.remove('active');
  document.querySelectorAll('#downloads-list tr').forEach(r => r.classList.remove('selected-row'));
};

async function refreshInspector(resetView = false) {
  if (!selectedDownloadId) return;
  try {
    const detail = await client.call('downloads.get', { id: selectedDownloadId });
    if (!detail) return;

    const snap = detail.snapshot || detail;
    const isTorrent = snap.fileName && snap.fileName.startsWith('⚡ [Torrent]');
    const name = fixMojibake(snap.fileName || 'Archivo');

    document.getElementById('insp-title').textContent = name;
    document.getElementById('insp-proto-badge').textContent = isTorrent ? 'Torrent' : 'eD2k';
    document.getElementById('insp-proto-badge').className = isTorrent ? 'badge badge-torrent' : 'badge badge-ed2k';
    document.getElementById('insp-hash').textContent = detail.ed2kHash || detail.ed2k_hash || snap.ed2kHash || '-';
    document.getElementById('insp-dest').textContent = detail.targetPath || detail.target_path || 'Carpeta Incoming NAS';
    document.getElementById('insp-temp').textContent = detail.tempPath || detail.temp_path || '-';

    const transferred = snap.receivedBytes || snap.received_bytes || 0;
    const total = snap.totalBytes || snap.total_bytes || 0;
    const progress = total > 0 ? ((transferred / total) * 100).toFixed(1) : '0.0';
    document.getElementById('insp-transfer').textContent = `${formatBytes(transferred)} de ${formatBytes(total)} (${progress}%)`;

    const speed = snap.downloadRateBps || snap.download_rate_bps || 0;
    let etaText = 'Desconocido';
    if (detail.etaSeconds && detail.etaSeconds > 0) {
      const s = detail.etaSeconds;
      const h = Math.floor(s / 3600);
      const m = Math.floor((s % 3600) / 60);
      etaText = h > 0 ? `${h}h ${m}m` : `${m}m ${s % 60}s`;
    } else if (speed > 0 && total > transferred) {
      const rem = Math.ceil((total - transferred) / speed);
      const h = Math.floor(rem / 3600);
      const m = Math.floor((rem % 3600) / 60);
      etaText = h > 0 ? `${h}h ${m}m` : `${m}m`;
    }
    document.getElementById('insp-speed-eta').textContent = `${formatSpeed(speed)} | ETA: ${etaText}`;

    // Fase y Ubicación de Almacenamiento
    const statusLower = (snap.status || '').toLowerCase();
    const stageMeta = typeof getDownloadCompletionStageMeta === 'function' ? getDownloadCompletionStageMeta(snap) : { active: false };
    const stageBox = document.getElementById('insp-stage-box');
    const stageText = document.getElementById('insp-stage-text');
    const stageDot = document.getElementById('insp-stage-dot');
    if (stageBox && stageText) {
      if (stageMeta.active) {
        const isVerify = stageMeta.key === 'verifying_hashes';
        const isReconstruct = stageMeta.key === 'reconstructing';
        const isOrg = stageMeta.key === 'organizing';
        const color = isVerify ? '#c084fc' : (isReconstruct ? '#38bdf8' : (isOrg ? '#34d399' : '#fbbf24'));
        const bg = isVerify ? 'rgba(168, 85, 247, 0.08)' : (isReconstruct ? 'rgba(56, 189, 248, 0.08)' : (isOrg ? 'rgba(16, 185, 129, 0.08)' : 'rgba(245, 158, 11, 0.08)'));
        const border = isVerify ? 'rgba(168, 85, 247, 0.4)' : (isReconstruct ? 'rgba(56, 189, 248, 0.4)' : (isOrg ? 'rgba(16, 185, 129, 0.4)' : 'rgba(245, 158, 11, 0.4)'));
        stageBox.style.borderColor = border;
        stageBox.style.background = bg;
        if (stageDot) stageDot.style.background = color;
        stageText.innerHTML = `<strong style="color:${color};">${escapeHtml(stageMeta.pipelineTitle)}:</strong> ${escapeHtml(stageMeta.pipelineSub)}`;
      } else if (statusLower === 'completing') {
        stageBox.style.borderColor = 'rgba(245, 158, 11, 0.4)';
        stageBox.style.background = 'rgba(245, 158, 11, 0.08)';
        if (stageDot) stageDot.style.background = '#fbbf24';
        stageText.innerHTML = `<strong style="color:#fbbf24;">Consolidando archivo:</strong> Escribiendo datos definitivos en destino...`;
      } else if (statusLower === 'completed' || (total > 0 && transferred >= total)) {
        stageBox.style.borderColor = 'rgba(16, 185, 129, 0.4)';
        stageBox.style.background = 'rgba(16, 185, 129, 0.08)';
        if (stageDot) stageDot.style.background = '#34d399';
        stageText.innerHTML = `<strong style="color:#34d399;">Completado y verificado:</strong> Disponible en el almacenamiento y compartido en la red.`;
      } else if (statusLower === 'verifying') {
        stageBox.style.borderColor = 'rgba(168, 85, 247, 0.4)';
        stageBox.style.background = 'rgba(168, 85, 247, 0.08)';
        if (stageDot) stageDot.style.background = '#c084fc';
        stageText.innerHTML = `<strong style="color:#c084fc;">Verificando hashes:</strong> Comprobando integridad del archivo...`;
      } else {
        stageBox.style.borderColor = 'rgba(56, 189, 248, 0.3)';
        stageBox.style.background = 'rgba(56, 189, 248, 0.06)';
        if (stageDot) stageDot.style.background = '#38bdf8';
        stageText.innerHTML = `<strong style="color:#38bdf8;">Descarga activa:</strong> Recibiendo fragmentos en unidad local rápida (SSD). Al 100% se moverá al NAS.`;
      }
    }

    const prioSel = document.getElementById('insp-priority-select');
    if (prioSel && document.activeElement !== prioSel) {
      prioSel.value = (snap.priority || 'auto').toLowerCase();
    }

    // Partes
    const parts = detail.parts || snap.parts || [];
    document.getElementById('insp-parts-count').textContent = parts.length;
    renderExpandedParts(parts);

    // Actualizar fuentes para pestaña y badge
    const currentHash = detail.ed2kHash || detail.ed2k_hash || snap.ed2kHash;
    if (currentHash) {
      client.call('sources.list', { fileHash: currentHash }).then(sources => {
        const list = Array.isArray(sources) ? sources : (sources.sources || []);
        const badge = document.getElementById('insp-peers-count');
        if (badge) badge.textContent = list.length;
        if (inspectorActiveTab === 'insp-peers') {
          renderInspectorPeersTable(list);
        }
      }).catch(e => console.warn('Error fetching peer count:', e));
    }
  } catch (e) {
    console.error('Error refreshing inspector:', e);
  }
}

function renderExpandedParts(parts) {
  const grid = document.getElementById('insp-parts-grid');
  if (!grid) return;
  if (parts.length === 0) {
    grid.innerHTML = '<div style="color: var(--text-muted); font-size: 0.85rem; padding: 10px;">Información de partes aún no disponible para esta descarga.</div>';
    return;
  }

  let verifiedCount = 0;
  let downloadingCount = 0;
  let html = '';
  parts.forEach(p => {
    const state = (p.state || 'missing').toLowerCase();
    if (state === 'verified' || state === 'completed') verifiedCount++;
    else if (state === 'downloading') downloadingCount++;

    html += `<div class="part-block ${state}" title="Parte #${p.partIndex}: ${state.toUpperCase()} (bytes ${formatBytes(p.verifiedBytes || 0)})"></div>`;
  });
  grid.innerHTML = html;
  document.getElementById('insp-parts-summary').textContent = `${parts.length} partes totales | ${verifiedCount} verificadas | ${downloadingCount} descargando`;
}

let lastRenderedPeersKey = '';

function renderInspectorPeersTable(list) {
  document.getElementById('insp-peers-count').textContent = list.length;
  document.getElementById('insp-peers-summary').textContent = `${list.length} fuentes registradas en red`;

  const tbody = document.getElementById('insp-peers-list');
  if (!tbody) return;
  if (list.length === 0) {
    lastRenderedPeersKey = '';
    tbody.innerHTML = '<tr><td colspan="6" style="text-align: center; color: var(--text-muted); padding: 20px;">Buscando fuentes activas en servidores eD2k y red Kad...</td></tr>';
    return;
  }

  const peersKey = list.map(s => `${s.address || `${s.ip}:${s.port}`}-${s.status}-${s.failCount}-${s.cooldownUntilUnixMs}`).join('|');
  if (lastRenderedPeersKey === peersKey && tbody.children.length === list.length) {
    // Nada ha cambiado en la lista de pares, evitar repintado del DOM
    return;
  }
  lastRenderedPeersKey = peersKey;

  let html = '';
  list.forEach(s => {
    const origin = s.origin || 'Kad / eD2k';
    const status = (s.status || 'Allowed').toLowerCase();
    let statusBadge = '<span class="badge badge-wait">Disponible</span>';
    if (status === 'connected') statusBadge = '<span class="badge badge-dl">Conectado</span>';
    else if (status === 'cooldown') statusBadge = '<span class="badge badge-pause">Cooldown</span>';
    else if (status === 'rejected') statusBadge = '<span class="badge badge-error">Rechazado</span>';

    html += `
      <tr>
        <td><code>${s.address || `${s.ip}:${s.port}`}</code></td>
        <td>${s.software || '<span style="color: var(--text-muted);">eMule Compatible</span>'}</td>
        <td><span style="font-size: 0.8rem; color: var(--text-muted);">${origin}</span></td>
        <td>${statusBadge}</td>
        <td>${s.failCount || 0}</td>
        <td>${s.cooldownUntilUnixMs ? Math.max(0, Math.ceil((s.cooldownUntilUnixMs - Date.now()) / 1000)) + ' s' : '-'}</td>
      </tr>
    `;
  });
  tbody.innerHTML = html;
}

async function refreshInspectorPeers(fileHash) {
  if (!fileHash) return;
  try {
    const sources = await client.call('sources.list', { fileHash });
    const list = Array.isArray(sources) ? sources : (sources.sources || []);
    renderInspectorPeersTable(list);
  } catch (e) {
    console.error('Error fetching sources for inspector:', e);
  }
}

// Configurar pestañas del inspector
document.querySelectorAll('.insp-tab-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.insp-tab-btn').forEach(b => b.classList.remove('active'));
    document.querySelectorAll('.insp-view-panel').forEach(p => p.style.display = 'none');
    btn.classList.add('active');
    inspectorActiveTab = btn.dataset.inspTab;
    const target = document.getElementById(inspectorActiveTab);
    if (target) {
      target.style.display = 'block';
      if (inspectorActiveTab === 'insp-peers') {
        refreshInspector(false);
      }
    }
  });
});

document.getElementById('btn-close-inspector').onclick = window.closeInspector;

document.getElementById('btn-save-insp-priority').onclick = async () => {
  if (!selectedDownloadId) return;
  const priority = document.getElementById('insp-priority-select').value;
  try {
    await client.call('downloads.setPriority', { id: selectedDownloadId, priority });
    addLog('info', `Prioridad cambiada a ${priority.toUpperCase()} para la descarga actual.`);
    refreshDownloads();
  } catch (e) {
    alert('Error cambiando prioridad: ' + (e.message || JSON.stringify(e)));
  }
};

document.getElementById('btn-add-manual-peer').onclick = async () => {
  if (!selectedDownloadId) return;
  const addr = prompt('Introduce la dirección IP:Puerto del peer (Ej: 192.168.1.50:4662):');
  if (!addr) return;
  try {
    const hash = document.getElementById('insp-hash').textContent;
    await client.call('sources.addManual', { fileHash: hash, address: addr.trim() });
    addLog('success', `Fuente manual ${addr} añadida con éxito.`);
    refreshInspectorPeers(hash);
  } catch (e) {
    alert('Error al añadir fuente: ' + (e.message || JSON.stringify(e)));
  }
};

// === SERVIDORES ED2K ===
let cachedServers = [];
async function refreshServers() {
  try {
    const res = await client.call('servers.list');
    const servers = res.servers || [];
    cachedServers = servers;
    document.getElementById('servers-summary').textContent = `${servers.length} servidores configurados`;

    const sStatus = await client.call('servers.status');
    const connCard = document.getElementById('connected-server-card');
    const headerEd2k = document.getElementById('ed2k-status');

    const statusStr = (sStatus && sStatus.status ? sStatus.status : '').toLowerCase();
    if (statusStr === 'connected') {
      connCard.style.display = 'block';
      document.getElementById('cs-name').textContent = sStatus.serverName || sStatus.serverAddress || 'Servidor';
      document.getElementById('cs-desc').textContent = `Dirección: ${sStatus.serverAddress || '-'} | ${sStatus.serverDesc || ''}`;
      document.getElementById('cs-highid-badge').textContent = sStatus.isHighId ? 'HighID ✔' : 'LowID ⚠';
      document.getElementById('cs-highid-badge').className = sStatus.isHighId ? 'badge badge-completed' : 'badge badge-completing';
      document.getElementById('cs-stats').textContent = `${(sStatus.currentUsers || 0).toLocaleString()} users / ${(sStatus.currentFiles || 0).toLocaleString()} files`;

      headerEd2k.innerHTML = `<span class="sb-lbl">eD2k</span> <span style="color: ${sStatus.isHighId ? '#34d399' : '#fbbf24'};">${sStatus.isHighId ? 'HighID' : 'LowID'}</span>`;
      headerEd2k.style.color = '';
      const p2pDot = document.getElementById('p2p-net-dot');
      if (p2pDot) {
        p2pDot.textContent = '';
        p2pDot.style.background = sStatus.isHighId ? '#34d399' : '#fbbf24';
        p2pDot.style.boxShadow = sStatus.isHighId ? '0 0 6px rgba(52, 211, 153, 0.6)' : '0 0 6px rgba(251, 191, 36, 0.5)';
      }

      // Chequeo proactivo de LowID para ofrecer tutorial
      evaluateLowIdState(sStatus.isHighId, null);
    } else if (statusStr === 'connecting') {
      connCard.style.display = 'none';
      headerEd2k.innerHTML = '<span class="sb-lbl">eD2k</span> <span style="color: #fbbf24;">Conectando...</span>';
      headerEd2k.style.color = '';
      const p2pDot = document.getElementById('p2p-net-dot');
      if (p2pDot) {
        p2pDot.textContent = '';
        p2pDot.style.background = '#fbbf24';
        p2pDot.style.boxShadow = 'none';
      }
    } else {
      connCard.style.display = 'none';
      headerEd2k.innerHTML = '<span class="sb-lbl">eD2k</span> <span style="color: var(--text-muted);">Off</span>';
      headerEd2k.style.color = '';
      const p2pDot = document.getElementById('p2p-net-dot');
      if (p2pDot) {
        p2pDot.textContent = '';
        p2pDot.style.background = '#64748b';
        p2pDot.style.boxShadow = 'none';
      }
    }

    const tbody = document.getElementById('servers-list');
    if (servers.length === 0) {
      tbody.innerHTML = `<tr><td colspan="6" style="text-align:center; padding: 25px; color: var(--text-muted);">No hay servidores en la lista. Pulsa "Actualizar server.met desde URL" o "Importar server.met de aMule".</td></tr>`;
      return;
    }

    let html = '';
    servers.forEach(s => {
      const isConnected = statusStr === 'connected' && sStatus.serverAddress === s.address;
      const statusBadge = isConnected ? '<span class="badge badge-completed">🟢 Conectado</span>' : '<span class="badge badge-wait">⚪ Disponible</span>';

      html += `
        <tr>
          <td><strong style="color: #fff;">${s.name || 'Servidor eD2k'}</strong></td>
          <td><code>${s.address}</code></td>
          <td style="color: var(--text-muted); font-size: 0.8rem;">${s.description || '-'}</td>
          <td>${s.ping > 0 ? s.ping + ' ms' : '-'}</td>
          <td>${statusBadge}</td>
          <td style="text-align: right;">
            ${isConnected ? 
              `<button class="btn btn-sm btn-danger" onclick="disconnectServer()">Desconectar</button>` : 
              `<button class="btn btn-sm btn-primary" onclick="connectServer('${s.address}')">Conectar</button>`}
          </td>
        </tr>
      `;
    });
    tbody.innerHTML = html;
  } catch (e) {
    console.error('Error refreshing servers:', e);
  }
}

// === KADEMLIA STATUS & CONTACTOS ===
async function refreshKad() {
  try {
    const kStatus = await client.call('kad.status');
    const kadHeader = document.getElementById('kad-status');
    const natPill = document.getElementById('nat-pill');

    if (kStatus && kStatus.isUdpActive) {
      const count = kStatus.totalContacts || kStatus.dbContactsCount || 0;
      kadHeader.innerHTML = `<span class="sb-lbl">Kad</span> <span class="sb-val" style="color: ${count > 0 ? '#e2e8f0' : '#fbbf24'};">${count} nodos</span>`;
      kadHeader.style.color = '';

      document.getElementById('kad-card-state').textContent = count > 0 ? '🟢 Conectado (DHT Activo)' : '🟡 Conectando...';
      document.getElementById('kad-card-state').style.color = count > 0 ? '#34d399' : '#fbbf24';
      document.getElementById('kad-card-bind').textContent = `UDP: ${kStatus.udpBindAddress || '0.0.0.0:4672'}`;
      document.getElementById('kad-card-nodes').textContent = kStatus.totalContacts || 0;
      document.getElementById('kad-card-db-nodes').textContent = `${kStatus.dbContactsCount || 0} contactos en DB`;

      const fw = kStatus.udpFirewallStatus || 'unverified';
      let fwLabel = 'Abierto (OK)';
      let fwColor = 'var(--download-color)';
      if (fw === 'firewalled') {
        fwLabel = 'Tras NAT (Firewalled)';
        fwColor = 'var(--warning)';
      } else if (fw === 'testing' || fw === 'unverified') {
        fwLabel = 'Verificando...';
        fwColor = 'var(--text-muted)';
      }
      document.getElementById('kad-card-firewall').textContent = fwLabel;
      document.getElementById('kad-card-firewall').style.color = fwColor;
      natPill.textContent = `Kad: ${fwLabel}`;
      natPill.style.color = fwColor;

      document.getElementById('kad-card-packets').textContent = `⬇ ${kStatus.packetsReceived || 0} pkts / ⬆ ${kStatus.packetsSent || 0} pkts`;

      // Evaluar si Kad está en estado firewalled
      evaluateLowIdState(null, fw);
    } else {
      kadHeader.textContent = '⚪ Kad: Desconectado';
      kadHeader.style.color = 'var(--text-muted)';
      document.getElementById('kad-card-state').textContent = '⚪ Desconectado';
      document.getElementById('kad-card-state').style.color = 'var(--text-muted)';
      natPill.textContent = 'Kad: Desconectado';
      natPill.style.color = 'var(--text-muted)';
    }
  } catch (e) {
    console.error('Error refreshing Kad:', e);
  }
}

async function refreshKadContacts() {
  try {
    const res = await client.call('kad.listContacts', { limit: 100 });
    const list = res.contacts || [];
    const tbody = document.getElementById('kad-contacts-list');
    document.getElementById('kad-contacts-summary').textContent = `${list.length} contactos en tabla de enrutamiento`;

    if (list.length === 0) {
      tbody.innerHTML = `<tr><td colspan="5" style="text-align:center; padding: 25px; color: var(--text-muted);">No hay contactos en memoria. Pulsa "Bootstrap Kad desde URL" para descubrir nodos.</td></tr>`;
      return;
    }

    let html = '';
    list.slice(0, 50).forEach(c => {
      html += `
        <tr>
          <td><code>${c.ip}:${c.udpPort}</code></td>
          <td><code style="font-size: 0.75rem; color: var(--text-muted);">${(c.kadId || '').substring(0, 16)}...</code></td>
          <td>v${c.version || 0}</td>
          <td>${c.verified ? '<span class="badge badge-dl">Sí</span>' : '<span class="badge badge-wait">No</span>'}</td>
          <td>${c.failCount || 0}</td>
        </tr>
      `;
    });
    tbody.innerHTML = html;
  } catch (e) {
    console.error('Error fetching Kad contacts:', e);
  }
}

let sharesCache = [];
let sharesFilter = 'all';
let sharesSearch = '';
let sharesSortKey = 'name';
let sharesSortDir = 'asc';
let sharesViewMode = 'table';
const sharesOpenFolders = new Set();

function classifyShare(sf) {
  const name = sf.fileName || '';
  const path = sf.filePath || '';
  const isSeries = /\/series\//i.test(path) || /\/tv shows\//i.test(path) || /S\d{1,2}E\d{1,2}/i.test(name);
  const kind = isSeries ? 'series' : 'movies';

  const resMatch = name.match(/\b(2160p|4K|1080p|720p|480p|SD)\b/i);
  const resolution = resMatch ? resMatch[1].toUpperCase() : '';
  const extMatch = name.match(/\.([a-z0-9]{2,5})$/i);
  const ext = extMatch ? extMatch[1].toUpperCase() : '';

  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/').filter(Boolean);
  const parentParts = parts.length > 1 ? parts.slice(0, -1) : [];
  const cleanParts = parentParts.map(p => p.replace(/\s*\[(tmdb|tvdb)id-\d+\]/gi, '').trim()).filter(Boolean);
  const relFolder = cleanParts.slice(-2).join(' › ');
  const folderGroup = cleanParts.length > 0
    ? (/^season\s+\d+/i.test(cleanParts[cleanParts.length - 1]) && cleanParts.length >= 2
        ? cleanParts[cleanParts.length - 2]
        : cleanParts[cleanParts.length - 1])
    : (isSeries ? 'Series' : 'Películas');

  return { kind, resolution, ext, relFolder, folderGroup };
}

function updateSharesKpisAndCounts(list) {
  let totalSize = 0;
  let totalUploaded = 0;
  let activeRate = 0;
  let activeCount = 0;
  let moviesCount = 0;
  let seriesCount = 0;
  let withUploadsCount = 0;

  list.forEach(sf => {
    totalSize += sf.fileSize || 0;
    totalUploaded += sf.uploadedBytes || 0;
    if ((sf.uploadRateBps || 0) > 0) {
      activeRate += sf.uploadRateBps;
      activeCount++;
    }
    if ((sf.uploadedBytes || 0) > 0) withUploadsCount++;
    const meta = classifyShare(sf);
    if (meta.kind === 'series') seriesCount++;
    else moviesCount++;
  });

  const badge = document.getElementById('shares-count-badge');
  if (badge) badge.textContent = `${list.length} archivos`;

  const kpiFiles = document.getElementById('shares-kpi-files');
  const kpiSize = document.getElementById('shares-kpi-size');
  const kpiUploaded = document.getElementById('shares-kpi-uploaded');
  const kpiWithUploads = document.getElementById('shares-kpi-with-uploads');
  const kpiRatio = document.getElementById('shares-kpi-ratio');
  const kpiLive = document.getElementById('shares-kpi-live');
  const kpiLiveSub = document.getElementById('shares-kpi-live-sub');
  const kpiLiveCard = document.getElementById('shares-kpi-live-card');

  if (kpiFiles) kpiFiles.textContent = `${list.length} archivos`;
  if (kpiSize) kpiSize.textContent = `${formatBytes(totalSize)} compartidos`;
  if (kpiUploaded) kpiUploaded.textContent = formatBytes(totalUploaded);
  if (kpiWithUploads) kpiWithUploads.textContent = `${withUploadsCount} archivos con tráfico`;
  if (kpiRatio) {
    const globalRatio = totalSize > 0 ? (totalUploaded / totalSize) : 0;
    kpiRatio.textContent = `${globalRatio.toFixed(2)}×`;
  }
  if (kpiLive) kpiLive.textContent = activeCount > 0 ? `${formatBytes(activeRate)}/s` : '-';
  if (kpiLiveSub) {
    kpiLiveSub.textContent = activeCount > 0
      ? `${activeCount} ${activeCount === 1 ? 'subida activa' : 'subidas activas'}`
      : 'Sin envíos activos';
  }
  if (kpiLiveCard) kpiLiveCard.classList.toggle('is-live', activeCount > 0);

  const setCnt = (id, val) => {
    const el = document.getElementById(id);
    if (el) el.textContent = `(${val})`;
  };
  setCnt('shares-cnt-all', list.length);
  setCnt('shares-cnt-uploading', activeCount);
  setCnt('shares-cnt-movies', moviesCount);
  setCnt('shares-cnt-series', seriesCount);
  setCnt('shares-cnt-uploaded', withUploadsCount);
}

function getFilteredAndSortedShares() {
  const q = sharesSearch.trim().toLowerCase();
  const filtered = sharesCache.filter(sf => {
    const meta = classifyShare(sf);
    if (sharesFilter === 'uploading' && !(sf.uploadRateBps > 0)) return false;
    if (sharesFilter === 'movies' && meta.kind !== 'movies') return false;
    if (sharesFilter === 'series' && meta.kind !== 'series') return false;
    if (sharesFilter === 'uploaded' && !(sf.uploadedBytes > 0)) return false;

    if (q) {
      const hay = `${sf.fileName || ''} ${sf.filePath || ''} ${meta.relFolder}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });

  const dir = sharesSortDir === 'asc' ? 1 : -1;
  filtered.sort((a, b) => {
    switch (sharesSortKey) {
      case 'size':
        return ((a.fileSize || 0) - (b.fileSize || 0)) * dir;
      case 'uploaded':
        return ((a.uploadedBytes || 0) - (b.uploadedBytes || 0)) * dir;
      case 'rate':
        return ((a.uploadRateBps || 0) - (b.uploadRateBps || 0)) * dir;
      case 'priority':
        return String(a.upPriority || 'normal').localeCompare(String(b.upPriority || 'normal')) * dir;
      case 'name':
      default:
        return String(a.fileName || '').localeCompare(String(b.fileName || ''), 'es', { sensitivity: 'base' }) * dir;
    }
  });
  return filtered;
}

function updateSharesSortIcons() {
  ['name', 'size', 'priority', 'uploaded', 'rate'].forEach(k => {
    const el = document.getElementById(`shares-sort-${k}`);
    if (el) {
      el.textContent = sharesSortKey === k ? (sharesSortDir === 'asc' ? '▲' : '▼') : '';
    }
  });
}

function renderShareBadgesHtml(meta) {
  let badges = '';
  if (meta.resolution) {
    badges += `<span class="share-meta-chip is-res">${escapeHtml(meta.resolution)}</span>`;
  }
  if (meta.ext) {
    badges += `<span class="share-meta-chip">${escapeHtml(meta.ext)}</span>`;
  }
  return badges;
}

function renderShareTypeIcon(kind) {
  if (kind === 'series') {
    return `<span class="share-type-icon is-series" title="Serie"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="20" height="15" x="2" y="7" rx="2" ry="2"/><polyline points="17 2 12 7 7 2"/></svg></span>`;
  }
  return `<span class="share-type-icon is-movie" title="Película"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18"/><path d="M17 3v18"/><path d="M3 12h18"/></svg></span>`;
}

function renderSharesTableRows(list) {
  const tbody = document.getElementById('shares-list');
  if (!tbody) return;
  if (list.length === 0) {
    tbody.innerHTML = `<tr><td colspan="5" style="text-align:center; padding: 28px; color: var(--text-muted);">No hay archivos compartidos que coincidan con los filtros actuales.</td></tr>`;
    return;
  }

  let html = '';
  list.forEach(sf => {
    const meta = classifyShare(sf);
    const fileSize = sf.fileSize || 0;
    const uploaded = sf.uploadedBytes || 0;
    const rate = sf.uploadRateBps || 0;
    const isUploading = rate > 0;
    const sizeStr = formatBytes(fileSize);
    const upTotalStr = formatBytes(uploaded);
    const rateStr = isUploading ? `${formatBytes(rate)}/s` : '-';
    const priority = sf.upPriority || 'normal';
    const ratio = fileSize > 0 ? (uploaded / fileSize) : 0;
    const ratioWidth = Math.min(100, Math.round(ratio * 100));
    const ratioLabel = ratio >= 0.05 ? `${ratio.toFixed(1)}×` : '';

    html += `
      <tr class="${isUploading ? 'share-row-uploading' : ''}">
        <td title="${escapeHtml(sf.filePath || sf.fileName || '')}">
          <div class="share-file-cell">
            ${renderShareTypeIcon(meta.kind)}
            <div style="min-width: 0; flex: 1;">
              <div class="share-file-title-row">
                <span class="share-file-name">${escapeHtml(sf.fileName || '')}</span>
                ${renderShareBadgesHtml(meta)}
              </div>
              <div class="share-path-sub">${escapeHtml(meta.relFolder || sf.filePath || '')}</div>
            </div>
          </div>
        </td>
        <td class="tabular-nums">${sizeStr}</td>
        <td><span class="badge badge-sub" style="text-transform: capitalize;">${escapeHtml(priority)}</span></td>
        <td class="tabular-nums">
          <div style="color: ${uploaded > 0 ? 'var(--text)' : 'var(--text-dim)'}; font-weight: ${uploaded > 0 ? '600' : '400'};">${upTotalStr}</div>
          ${uploaded > 0 ? `<div class="share-ratio-wrap"><div class="share-ratio-bar"><div class="share-ratio-fill" style="width: ${ratioWidth}%"></div></div>${ratioLabel ? `<span class="share-ratio-badge">${ratioLabel}</span>` : ''}</div>` : ''}
        </td>
        <td class="tabular-nums">
          ${isUploading
            ? `<span class="share-live-rate"><span class="share-live-dot"></span>${rateStr}</span>`
            : `<span style="color: var(--text-dim);">-</span>`}
        </td>
      </tr>
    `;
  });
  tbody.innerHTML = html;
}

function renderSharesFolders(list) {
  const container = document.getElementById('shares-folders-list');
  if (!container) return;
  if (list.length === 0) {
    container.innerHTML = `<div style="text-align:center; padding: 28px; color: var(--text-muted); background: var(--card-bg); border: 1px solid var(--border-subtle); border-radius: 10px;">No hay carpetas que coincidan con los filtros actuales.</div>`;
    return;
  }

  const groupsMap = new Map();
  list.forEach(sf => {
    const meta = classifyShare(sf);
    const key = `${meta.kind}:${meta.folderGroup}`;
    if (!groupsMap.has(key)) {
      groupsMap.set(key, {
        key,
        name: meta.folderGroup,
        kind: meta.kind,
        items: [],
        totalSize: 0,
        totalUploaded: 0,
        activeRate: 0,
      });
    }
    const g = groupsMap.get(key);
    g.items.push({ sf, meta });
    g.totalSize += sf.fileSize || 0;
    g.totalUploaded += sf.uploadedBytes || 0;
    g.activeRate += sf.uploadRateBps || 0;
  });

  const groups = Array.from(groupsMap.values());
  let html = '';
  groups.forEach(g => {
    const isOpen = sharesOpenFolders.has(g.key) || sharesSearch.trim().length > 0 || sharesFilter === 'uploading';
    const safeKey = encodeURIComponent(g.key);
    html += `
      <div class="share-folder-card ${isOpen ? 'open' : ''}">
        <button type="button" class="share-folder-head" onclick="toggleShareFolder('${safeKey}')">
          <div style="display: flex; align-items: center; gap: 10px; min-width: 0;">
            <span class="share-folder-chevron">▶</span>
            ${renderShareTypeIcon(g.kind)}
            <span class="share-folder-title">${escapeHtml(g.name)}</span>
            <span class="badge badge-sub">${g.items.length} ${g.items.length === 1 ? 'archivo' : 'archivos'}</span>
            ${g.activeRate > 0 ? `<span class="share-live-rate" style="font-size: 0.74rem;"><span class="share-live-dot"></span>${formatBytes(g.activeRate)}/s</span>` : ''}
          </div>
          <div class="share-folder-meta tabular-nums">
            <span>${formatBytes(g.totalSize)}</span>
            <span style="color: ${g.totalUploaded > 0 ? 'var(--primary)' : 'var(--text-dim)'};">↑ ${formatBytes(g.totalUploaded)}</span>
          </div>
        </button>
        <div class="share-folder-body">
          ${g.items.map(({ sf, meta }) => {
            const rate = sf.uploadRateBps || 0;
            const uploaded = sf.uploadedBytes || 0;
            return `
              <div class="share-folder-row ${rate > 0 ? 'is-uploading' : ''}">
                <div style="display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1;">
                  <span class="share-file-name">${escapeHtml(sf.fileName || '')}</span>
                  ${renderShareBadgesHtml(meta)}
                </div>
                <div class="share-folder-row-stats tabular-nums">
                  <span>${formatBytes(sf.fileSize || 0)}</span>
                  <span style="color: ${uploaded > 0 ? 'var(--text)' : 'var(--text-dim)'};">↑ ${formatBytes(uploaded)}</span>
                  <span style="min-width: 72px; text-align: right;">${rate > 0 ? `<span class="share-live-rate">${formatBytes(rate)}/s</span>` : '<span style="color: var(--text-dim);">-</span>'}</span>
                </div>
              </div>
            `;
          }).join('')}
        </div>
      </div>
    `;
  });
  container.innerHTML = html;
}

function renderSharesView() {
  updateSharesSortIcons();
  const filtered = getFilteredAndSortedShares();
  const tableWrap = document.getElementById('shares-table-wrap');
  const foldersWrap = document.getElementById('shares-folders-wrap');

  if (sharesViewMode === 'folders') {
    if (tableWrap) tableWrap.style.display = 'none';
    if (foldersWrap) foldersWrap.style.display = 'block';
    renderSharesFolders(filtered);
  } else {
    if (tableWrap) tableWrap.style.display = 'block';
    if (foldersWrap) foldersWrap.style.display = 'none';
    renderSharesTableRows(filtered);
  }
}

window.setSharesFilter = function(filter) {
  sharesFilter = filter;
  document.querySelectorAll('#shares-filter-group .filter-btn').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-shares-filter') === filter);
  });
  renderSharesView();
};

window.setSharesSearch = function(val) {
  sharesSearch = val || '';
  renderSharesView();
};

window.sortSharesBy = function(key) {
  if (sharesSortKey === key) {
    sharesSortDir = sharesSortDir === 'asc' ? 'desc' : 'asc';
  } else {
    sharesSortKey = key;
    sharesSortDir = (key === 'name' || key === 'priority') ? 'asc' : 'desc';
  }
  renderSharesView();
};

window.setSharesViewMode = function(mode) {
  sharesViewMode = mode === 'folders' ? 'folders' : 'table';
  document.querySelectorAll('#shares-view-toggle .opt-mode-btn').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-shares-view') === sharesViewMode);
  });
  renderSharesView();
};

window.toggleShareFolder = function(encodedKey) {
  const key = decodeURIComponent(encodedKey);
  if (sharesOpenFolders.has(key)) {
    sharesOpenFolders.delete(key);
  } else {
    sharesOpenFolders.add(key);
  }
  renderSharesView();
};

window.copyShareEd2kLink = async function(hash, encodedName, size) {
  const name = decodeURIComponent(encodedName || '');
  const link = `ed2k://|file|${name}|${size}|${hash}|/`;
  try {
    await navigator.clipboard.writeText(link);
    showToast('Enlace eD2k copiado al portapapeles', 'success', '🔗');
  } catch (_) {
    showToast('No se pudo copiar al portapapeles', 'warning', '⚠️');
  }
};

async function refreshShares() {
  try {
    const res = await client.call('shares.list', {});
    sharesCache = res.sharedFiles || [];
    updateSharesKpisAndCounts(sharesCache);
    renderSharesView();
  } catch (e) {
    console.error('Error refreshing shares:', e);
    const tbody = document.getElementById('shares-list');
    if (tbody) {
      tbody.innerHTML = `<tr><td colspan="5" style="text-align:center; padding: 20px; color: #f85149;">Error al cargar compartidos: ${escapeHtml(e.message || String(e))}</td></tr>`;
    }
  }
}

async function importNasShares() {
  try {
    const res = await client.call('shares.importKnownMet', {
      metPath: '/home/caraveliko/.aMule/known.met',
      directory: '/home/caraveliko/cineseries',
      verify: false
    });
    showToast(`NAS sincronizado: ${res.imported || 0} importados, ${res.enriched || 0} enriquecidos (${res.scanned_files || 0} archivos)`, 'success', '✅');
    refreshShares();
  } catch (e) {
    showToast(`Error al sincronizar con known.met: ${e.message || e}`, 'error', '❌');
  }
}

