// Pulse Core WebUI — Panel en vivo del Motor Híbrido DHT sin indexadores (eD2K ⇄ Kad ⇄ BitTorrent)
// y controles de aceleración criptográfica MD4 en el inspector de descargas.

let hybridStatusCache = null;
let hybridDhtSearchResults = [];

/**
 * Formatea una etiqueta legible para el origen de un puente o hallazgo DHT.
 */
function formatHybridOriginLabel(origin) {
  switch ((origin || '').toLowerCase()) {
    case 'dht_bep51':
      return 'DHT BEP 51';
    case 'dht_passive':
      return 'DHT Pasivo';
    case 'dht_exact_size':
      return 'Tamaño exacto DHT';
    case 'dht_token_exact':
      return 'DHT local (búsqueda activa)';
    case 'torznab_exact_size':
      return 'Torznab (tamaño exacto)';
    case 'torznab_pack_exact':
      return 'Torznab (pack multifichero)';
    case 'public_bt_exact':
      return 'BT Público (tamaño exacto)';
    case 'public_bt_pack':
      return 'BT Público (pack multifichero)';
    case 'kad_note':
      return 'Nota Kad';
    case 'torrent_ed2k_tag':
      return 'Etiqueta ed2k (.torrent)';
    case 'local_hash':
      return 'Cosecha BT→eD2k';
    case 'local_completed':
      return 'BT local';
    case 'manual':
      return 'Enlace manual';
    default:
      return origin || '-';
  }
}

/**
 * Devuelve la insignia visual del estado de verificación criptográfica de un puente.
 */
function formatHybridBridgeBadge(status) {
  const s = (status || '').toLowerCase();
  if (s === 'verified') {
    return '<span class="badge badge-completed">✔ Verificado MD4</span>';
  }
  if (s === 'rejected') {
    return '<span class="badge badge-error">✖ Rechazado</span>';
  }
  return '<span class="badge badge-wait">⏳ Candidato</span>';
}

/**
 * Acorta un hash hexadecimal largo preservando prefijo y sufijo legibles.
 */
function shortHybridHash(hash) {
  const clean = String(hash || '');
  if (clean.length <= 16) return clean || '-';
  return `${clean.slice(0, 8)}…${clean.slice(-6)}`;
}

/**
 * Refresca el estado completo del Motor Híbrido DHT (`hybrid.status` y `hybrid.listBridges`).
 */
async function refreshHybridView() {
  if (!client || !client.ws || client.ws.readyState !== WebSocket.OPEN) {
    return;
  }
  try {
    const [status, bridgesRes] = await Promise.all([
      client.call('hybrid.status', {}),
      client.call('hybrid.listBridges', { limit: 60 }),
    ]);
    hybridStatusCache = status;
    renderHybridStatusCards(status);
    renderHybridActiveFusions(status?.activeFusions || []);
    renderHybridBridgesTable(bridgesRes?.bridges || []);
  } catch (err) {
    console.warn('[PulseCore] Error refrescando Motor Híbrido DHT:', err);
  }
}

function renderHybridStatusCards(status) {
  if (!status) return;
  const h = status.dhtHarvester || {};
  const c = status.catalog || {};
  const activeCount = Array.isArray(status.activeFusions) ? status.activeFusions.length : 0;

  const harvesterEl = document.getElementById('hybrid-card-harvester');
  const harvesterMetaEl = document.getElementById('hybrid-card-harvester-meta');
  if (harvesterEl) {
    harvesterEl.textContent = h.running ? 'Activo' : 'Inactivo';
    harvesterEl.style.color = h.running ? '#34d399' : 'var(--text-muted)';
  }
  if (harvesterMetaEl) {
    const portInfo = h.listenAddr ? ` (${h.listenAddr})` : '';
    harvesterMetaEl.textContent =
      `${h.routingNodes || 0} nodos · ${h.affinityNodes || 0} afinidad${portInfo} · ⬇ ${h.packetsReceived || 0} / ⬆ ${h.packetsSent || 0}`;
  }

  const catalogEl = document.getElementById('hybrid-card-catalog');
  const catalogMetaEl = document.getElementById('hybrid-card-catalog-meta');
  if (catalogEl) catalogEl.textContent = String(c.catalogEntries || 0);
  if (catalogMetaEl) {
    catalogMetaEl.textContent =
      `${h.infohashesSampled || 0} BEP 51 · ${h.pexPeersDiscovered || 0} PEX · ${h.nativeEd2kTagsFound || 0} tags ed2k`;
  }

  const bridgesEl = document.getElementById('hybrid-card-bridges');
  const bridgesMetaEl = document.getElementById('hybrid-card-bridges-meta');
  if (bridgesEl) bridgesEl.textContent = String(c.bridgesVerified || 0);
  if (bridgesMetaEl) {
    bridgesMetaEl.textContent =
      `${c.bridgesCandidate || 0} candidatos · ${c.bridgesRejected || 0} rechazados · ${status.kadNotesPublished || 0} notas Kad`;
  }

  const fusedEl = document.getElementById('hybrid-card-fused');
  const fusedMetaEl = document.getElementById('hybrid-card-fused-meta');
  if (fusedEl) fusedEl.textContent = formatBytes(status.bytesFusedFromBt || 0);
  if (fusedMetaEl) {
    fusedMetaEl.textContent =
      `${activeCount} fusiones activas · ${status.rendezvousPeersFound || 0} pares Rendezvous`;
  }
}

function renderHybridActiveFusions(fusions) {
  const countEl = document.getElementById('hybrid-active-fusions-count');
  const tbody = document.getElementById('hybrid-active-fusions-list');
  if (countEl) {
    countEl.textContent = `${fusions.length} ${fusions.length === 1 ? 'activa' : 'activas'}`;
  }
  if (!tbody) return;

  if (!fusions.length) {
    tbody.innerHTML =
      '<tr><td colspan="6" style="text-align: center; padding: 18px; color: var(--text-muted);">Sin fusiones activas en este momento. Al detectar un puente válido para una descarga eD2k aparecerá aquí en directo.</td></tr>';
    return;
  }

  tbody.innerHTML = fusions
    .map((f) => {
      const stateBadge = f.verified
        ? '<span class="badge badge-completed">✔ Verificado MD4</span>'
        : '<span class="badge badge-wait">⏳ Verificando MD4</span>';
      return `
        <tr>
          <td><code title="${escapeHtml(f.ed2kHash)}">${escapeHtml(shortHybridHash(f.ed2kHash))}</code></td>
          <td><code title="${escapeHtml(f.btInfoHash)}">${escapeHtml(shortHybridHash(f.btInfoHash))}</code></td>
          <td>${stateBadge}</td>
          <td>${Number(f.peerCount || 0)}</td>
          <td style="font-family: var(--font-mono); color: #38bdf8;">${formatSpeed(f.btDownloadRateBps || 0)}</td>
          <td style="font-family: var(--font-mono); color: #34d399;">${formatBytes(f.bytesFused || 0)}</td>
        </tr>
      `;
    })
    .join('');
}

function renderHybridBridgesTable(bridges) {
  const summaryEl = document.getElementById('hybrid-bridges-summary');
  const tbody = document.getElementById('hybrid-bridges-list');
  if (summaryEl) {
    summaryEl.textContent = `${bridges.length} puentes registrados`;
  }
  if (!tbody) return;

  if (!bridges.length) {
    tbody.innerHTML =
      '<tr><td colspan="7" style="text-align: center; padding: 20px; color: var(--text-muted);">Todavía no hay puentes híbridos registrados.</td></tr>';
    return;
  }

  tbody.innerHTML = bridges
    .map((b) => {
      const kadLabel = b.publishedKadAtMs
        ? '<span class="badge badge-completed">Sembrado</span>'
        : '<span style="color: var(--text-muted); font-size: 0.78rem;">Pendiente</span>';
      return `
        <tr>
          <td><code title="${escapeHtml(b.ed2kHash)}">${escapeHtml(shortHybridHash(b.ed2kHash))}</code></td>
          <td><code title="${escapeHtml(b.btInfoHash)}">${escapeHtml(shortHybridHash(b.btInfoHash))}</code></td>
          <td style="font-family: var(--font-mono);">${formatBytes(b.fileSize || 0)}</td>
          <td>${formatHybridBridgeBadge(b.status)}</td>
          <td style="font-family: var(--font-mono);">${Number(b.verifiedChunks || 0)}</td>
          <td>${escapeHtml(formatHybridOriginLabel(b.origin))}</td>
          <td>${kadLabel}</td>
        </tr>
      `;
    })
    .join('');
}

/**
 * Busca torrents de vídeo en el catálogo Mainline DHT local (por texto o por tamaño exacto en bytes).
 */
async function searchHybridDhtCatalog() {
  const input = document.getElementById('hybrid-dht-search-input');
  const summaryEl = document.getElementById('hybrid-dht-search-summary');
  const wrapEl = document.getElementById('hybrid-dht-results-wrap');
  const tbody = document.getElementById('hybrid-dht-results-list');
  if (!input || !tbody) return;

  const raw = input.value.trim();
  if (!raw) {
    if (typeof appNotify === 'function') {
      appNotify('Introduce un título o un tamaño exacto en bytes para buscar en la DHT local.', 'warning', '🔍');
    }
    return;
  }

  const isExactBytes = /^\d{5,}$/.test(raw);
  const params = isExactBytes
    ? { exactSize: Number(raw), limit: 50 }
    : { query: raw, limit: 50 };

  if (summaryEl) summaryEl.textContent = 'Buscando en catálogo DHT local...';
  try {
    const res = await client.call('hybrid.searchDht', params);
    const items = Array.isArray(res?.items) ? res.items : [];
    hybridDhtSearchResults = items;

    if (wrapEl) wrapEl.style.display = 'block';
    if (summaryEl) {
      summaryEl.textContent = isExactBytes
        ? `${items.length} coincidencia(s) exactas para ${formatBytes(Number(raw))}`
        : `${items.length} vídeo(s) encontrados en DHT local sin indexadores`;
    }

    if (!items.length) {
      tbody.innerHTML =
        '<tr><td colspan="6" style="text-align: center; padding: 20px; color: var(--text-muted);">Sin coincidencias aún en el catálogo DHT local. El Sabueso DHT sigue indexando torrents de vídeo en segundo plano.</td></tr>';
      return;
    }

    tbody.innerHTML = items
      .map((item, idx) => {
        const displayName = item.videoFileName || item.name || 'Vídeo';
        return `
          <tr>
            <td style="max-width: 380px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${escapeHtml(displayName)}">${escapeHtml(displayName)}</td>
            <td style="font-family: var(--font-mono); white-space: nowrap;">${formatBytes(item.videoFileSize || item.totalBytes || 0)}</td>
            <td><code title="${escapeHtml(item.infoHash)}">${escapeHtml(shortHybridHash(item.infoHash))}</code></td>
            <td>${Number(item.estimatedPeers || 1)}</td>
            <td>${escapeHtml(formatHybridOriginLabel(item.source))}</td>
            <td style="text-align: right; white-space: nowrap;">
              <button type="button" class="btn btn-sm btn-primary" onclick="downloadHybridCatalogItem(${idx})" style="padding: 2px 8px; font-size: 0.74rem;">⬇ Descargar</button>
              <button type="button" class="btn btn-sm" onclick="copyHybridCatalogMagnet(${idx})" style="padding: 2px 7px; font-size: 0.74rem;" title="Copiar enlace magnet">📋</button>
            </td>
          </tr>
        `;
      })
      .join('');
  } catch (err) {
    console.error('[PulseCore] Error buscando en catálogo DHT local:', err);
    if (summaryEl) summaryEl.textContent = 'Error en la consulta DHT local';
    if (typeof appNotify === 'function') {
      appNotify('Error buscando en el catálogo DHT local: ' + (err.message || String(err)), 'error', '❌');
    }
  }
}

async function downloadHybridCatalogItem(index) {
  const item = hybridDhtSearchResults[index];
  if (!item || !item.magnet) return;
  try {
    await client.call('downloads.addEd2kLink', {
      link: item.magnet,
      categoryId: 'video',
      priority: 'auto',
    });
    if (typeof appNotify === 'function') {
      appNotify(`Descarga BitTorrent iniciada desde DHT local: ${item.videoFileName || item.name}`, 'success', '⚡');
    }
    if (typeof refreshDownloads === 'function') {
      refreshDownloads().catch((err) => console.warn('[PulseCore] refreshDownloads tras magnet DHT:', err));
    }
  } catch (err) {
    console.error('[PulseCore] Error iniciando magnet desde DHT local:', err);
    if (typeof appNotify === 'function') {
      appNotify('No se pudo añadir el torrent: ' + (err.message || String(err)), 'error', '❌');
    }
  }
}

async function copyHybridCatalogMagnet(index) {
  const item = hybridDhtSearchResults[index];
  if (!item || !item.magnet) return;
  try {
    await navigator.clipboard.writeText(item.magnet);
    if (typeof appNotify === 'function') {
      appNotify('Enlace magnet copiado al portapapeles.', 'success', '📋');
    }
  } catch (err) {
    console.warn('[PulseCore] Error copiando enlace magnet al portapapeles:', err);
  }
}

/**
 * Actualiza el estado del puente híbrido en el inspector inferior para la descarga eD2K seleccionada.
 */
async function refreshInspectorHybridStatus(snap, isTorrent) {
  const row = document.getElementById('insp-hybrid-row');
  const statusEl = document.getElementById('insp-hybrid-status');
  if (!row || !statusEl) return;

  if (isTorrent || !snap || !snap.id) {
    row.style.display = 'none';
    return;
  }
  row.style.display = '';

  const ed2kHash = snap.ed2kHash || snap.ed2k_hash || '';
  if (!ed2kHash) {
    statusEl.textContent = 'Sin hash eD2k disponible';
    return;
  }

  try {
    const status = await Promise.all([
      client.call('hybrid.status', {}),
      client.call('hybrid.listBridges', { ed2kHash }),
    ]);
    const [hybridStatus, bridgesRes] = status;
    const rateMap = {};
    (hybridStatus?.activeFusions || []).forEach((f) => {
      if (f.downloadId) rateMap[f.downloadId] = Number(f.btDownloadRateBps || 0);
    });
    window.hybridActiveRatesByDownloadId = rateMap;

    const activeFusion = (hybridStatus?.activeFusions || []).find(
      (f) => f.downloadId === snap.id || (f.ed2kHash || '').toLowerCase() === ed2kHash.toLowerCase()
    );
    const telemetry = (hybridStatus?.downloadTelemetry || []).find(
      (t) => t.downloadId === snap.id || (t.ed2kHash || '').toLowerCase() === ed2kHash.toLowerCase()
    );
    const bridges = bridgesRes?.bridges || [];

    if (activeFusion) {
      const verifyLabel = activeFusion.verified ? '✔ Verificado MD4' : '⏳ Verificando MD4';
      statusEl.innerHTML =
        `<span style="color: #34d399; font-weight: 600;">⚡ Fusión BT activa (${verifyLabel})</span> · ` +
        `<code>${escapeHtml(shortHybridHash(activeFusion.btInfoHash))}</code> · ` +
        `${formatBytes(activeFusion.bytesFused || 0)} fusionados · ${formatSpeed(activeFusion.btDownloadRateBps || 0)}`;
    } else if (bridges.length > 0) {
      const top = bridges[0];
      const retryBtn =
        (top.status || '').toLowerCase() !== 'verified'
          ? ` <button type="button" class="btn btn-sm" onclick="triggerHybridDiscovery('${escapeHtml(snap.id)}', this)" style="padding: 1px 7px; font-size: 0.72rem; margin-left: 6px;">🔄 Reintentar</button>`
          : '';
      statusEl.innerHTML =
        `${formatHybridBridgeBadge(top.status)} · <code>${escapeHtml(shortHybridHash(top.btInfoHash))}</code> ` +
        `(${escapeHtml(formatHybridOriginLabel(top.origin))})${retryBtn}`;
    } else if (telemetry && telemetry.statusSummary) {
      let eqBtn = '';
      if (telemetry.bestEquivalent && telemetry.bestEquivalent.magnet) {
        const magEnc = encodeURIComponent(telemetry.bestEquivalent.magnet);
        const nameEnc = encodeURIComponent(telemetry.bestEquivalent.name || 'Torrent equivalente');
        eqBtn = ` <button type="button" class="btn btn-sm btn-primary" onclick="addEquivalentHybridTorrent(decodeURIComponent('${magEnc}'), decodeURIComponent('${nameEnc}'))" style="padding: 1px 7px; font-size: 0.72rem; margin-left: 6px;">⬇ Añadir equivalente BT</button>`;
      }
      statusEl.innerHTML =
        `<span>${escapeHtml(telemetry.statusSummary)}</span>${eqBtn}` +
        ` <button type="button" class="btn btn-sm" onclick="triggerHybridDiscovery('${escapeHtml(snap.id)}', this)" style="padding: 1px 7px; font-size: 0.72rem; margin-left: 6px;">🔄 Buscar ahora</button>`;
    } else {
      statusEl.innerHTML =
        '<span>Buscando automáticamente en catálogo DHT, Torznab, etiquetas .torrent, notas Kad y Rendezvous...</span>' +
        ` <button type="button" class="btn btn-sm" onclick="triggerHybridDiscovery('${escapeHtml(snap.id)}', this)" style="padding: 1px 7px; font-size: 0.72rem; margin-left: 6px;">🔄 Buscar ahora</button>`;
    }
  } catch (err) {
    console.warn('[PulseCore] Error consultando puente híbrido del inspector:', err);
  }
}

/**
 * Lanza una ronda manual inmediata de descubrimiento híbrido (DHT local + Torznab + Kad + Rendezvous).
 */
async function triggerHybridDiscovery(downloadId, btnEl = null) {
  if (!downloadId) return;
  const prevText = btnEl ? btnEl.textContent : '';
  if (btnEl) {
    btnEl.disabled = true;
    btnEl.textContent = '⏳ Buscando...';
  }
  try {
    const res = await client.call('hybrid.discoverForDownload', { downloadId });
    if (typeof appNotify === 'function') {
      const totalChecked = (res?.torznabCandidatesChecked || 0) + (res?.publicCandidatesChecked || 0);
      const msg = res?.activeFusionStarted
        ? 'Puente BitTorrent encontrado y fusión híbrida iniciada.'
        : `Búsqueda híbrida completada: ${totalChecked} candidatos BT revisados, ${res?.dhtExactMatches || 0} coincidencias exactas.`;
      appNotify(msg, res?.activeFusionStarted ? 'success' : 'info', '⚡');
    }
    if (typeof refreshInspector === 'function') {
      refreshInspector(true).catch((err) =>
        console.warn('[PulseCore] Error refrescando inspector tras búsqueda manual:', err)
      );
    } else if (typeof refreshInspectorHybridStatus === 'function' && window.selectedDownloadSnap) {
      refreshInspectorHybridStatus(window.selectedDownloadSnap, false).catch((err) =>
        console.warn('[PulseCore] Error refrescando inspector híbrido tras búsqueda manual:', err)
      );
    }
  } catch (err) {
    console.warn('[PulseCore] Error lanzando descubrimiento híbrido manual:', err);
    if (typeof appNotify === 'function') {
      appNotify('Error en búsqueda híbrida: ' + (err.message || String(err)), 'error', '❌');
    }
  } finally {
    if (btnEl) {
      btnEl.disabled = false;
      btnEl.textContent = prevText || '🔄 Buscar ahora';
    }
  }
}

async function addEquivalentHybridTorrent(magnet, name) {
  if (!magnet) return;
  try {
    await client.call('downloads.addEd2kLink', {
      link: magnet,
      categoryId: 'video',
      priority: 'auto',
    });
    if (typeof appNotify === 'function') {
      appNotify(`Torrent equivalente añadido: ${name || 'Vídeo BT'}`, 'success', '⚡');
    }
    if (typeof refreshDownloads === 'function') {
      refreshDownloads().catch((err) => console.warn('[PulseCore] refreshDownloads tras equivalente BT:', err));
    }
  } catch (err) {
    console.error('[PulseCore] Error añadiendo torrent equivalente BT:', err);
    if (typeof appNotify === 'function') {
      appNotify('No se pudo añadir el torrent equivalente: ' + (err.message || String(err)), 'error', '❌');
    }
  }
}
