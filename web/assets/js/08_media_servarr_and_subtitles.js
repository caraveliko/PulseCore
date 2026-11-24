// Pulse Core WebUI — Operaciones visuales de Cine y Series y gestión de subtítulos (Bazarr nativo).
// Extraído de web/index.html.

// === GESTOR DE OPERACIONES VISUALES Y PROGRESO EN CINE & SERIES ===
function startMediaOperation(title, desc, icon) {
  const banner = document.getElementById('media-op-banner');
  const titleEl = document.getElementById('media-op-title');
  const descEl = document.getElementById('media-op-desc');
  const iconEl = document.getElementById('media-op-icon');
  const badgeEl = document.getElementById('media-op-badge');
  const bar = document.getElementById('media-op-progressbar');

  if (banner) {
    banner.style.display = 'block';
    banner.style.borderColor = '#3b82f6';
  }
  if (titleEl) titleEl.textContent = title;
  if (descEl) descEl.textContent = desc;
  if (iconEl) iconEl.textContent = icon;
  if (badgeEl) {
    badgeEl.textContent = 'En curso';
    badgeEl.style.color = '#60a5fa';
    badgeEl.style.background = 'rgba(59, 130, 246, 0.2)';
    badgeEl.style.borderColor = 'rgba(59, 130, 246, 0.3)';
  }
  if (bar) {
    bar.style.width = '25%';
    bar.style.background = 'linear-gradient(90deg, #3b82f6, #06b6d4)';
  }

  showToast(title, 'info', icon);
}

function updateMediaOperation(percent, desc) {
  const descEl = document.getElementById('media-op-desc');
  const bar = document.getElementById('media-op-progressbar');
  if (descEl && desc) descEl.textContent = desc;
  if (bar) bar.style.width = `${percent}%`;
}

function finishMediaOperation(success, title, desc, icon) {
  const banner = document.getElementById('media-op-banner');
  const titleEl = document.getElementById('media-op-title');
  const descEl = document.getElementById('media-op-desc');
  const iconEl = document.getElementById('media-op-icon');
  const badgeEl = document.getElementById('media-op-badge');
  const bar = document.getElementById('media-op-progressbar');

  if (titleEl) titleEl.textContent = title;
  if (descEl) descEl.textContent = desc;
  if (iconEl) iconEl.textContent = icon;

  if (bar) {
    bar.style.width = '100%';
    bar.style.background = success ? 'linear-gradient(90deg, #10b981, #22c55e)' : '#ef4444';
  }
  if (banner) {
    banner.style.borderColor = success ? '#22c55e' : '#ef4444';
  }
  if (badgeEl) {
    badgeEl.textContent = success ? 'Completado' : 'Error';
    badgeEl.style.color = success ? '#22c55e' : '#ef4444';
    badgeEl.style.background = success ? 'rgba(34, 197, 94, 0.2)' : 'rgba(239, 68, 68, 0.2)';
    badgeEl.style.borderColor = success ? 'rgba(34, 197, 94, 0.3)' : 'rgba(239, 68, 68, 0.3)';
  }

  showToast(title, success ? 'success' : 'error', icon);
  clearTimeout(window.__mediaOpTimer);
  window.__mediaOpTimer = setTimeout(() => {
    dismissMediaOpBanner();
  }, 5500);
}

function dismissMediaOpBanner() {
  const banner = document.getElementById('media-op-banner');
  if (banner) banner.style.display = 'none';
}

async function triggerScanDisk() {
  const btn = document.getElementById('btn-media-scan');
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '<span>⏳</span> <span>Escaneando...</span>'; }

  startMediaOperation('Escaneando carpetas locales...', 'Buscando archivos de vídeo en /Peliculas y /Series del disco/NAS...', '📁');
  updateMediaOperation(35, 'Analizando nombres de ficheros e identificando temporadas y capítulos...');

  try {
    const res = await client.call('media.scanDisk', {});
    updateMediaOperation(85, 'Sincronizando biblioteca SQLite con los archivos encontrados...');
    const msg = `${res.moviesScanned || 0} películas y ${res.episodesScanned || 0} episodios escaneados (${res.itemsSynchronized || 0} sincronizados, ${res.itemsEnriched || 0} enriquecidos)`;
    finishMediaOperation(true, 'Escaneo de disco completado', msg, '✅');
    addLog('success', `Escaneo de disco completado: ${msg}`);
    await loadMediaLibrary();
  } catch (e) {
    finishMediaOperation(false, 'Error escaneando disco', e.message, '❌');
    addLog('error', `Error en escaneo de disco: ${e.message}`);
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
}

async function triggerRefreshMetadata() {
  const btn = document.getElementById('btn-media-metadata');
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '<span>⏳</span> <span>Actualizando...</span>'; }

  startMediaOperation('Descargando metadatos...', 'Consultando bases de datos de TMDB y Sonarr...', '🔄');
  updateMediaOperation(45, 'Descargando pósters, sinopsis y fechas de emisión...');

  try {
    const res = await client.call('media.refreshMetadata', {});
    updateMediaOperation(90, 'Guardando portadas y actualizando catálogo local...');
    const count = res.itemsEnriched || 0;
    const msg = `${count} títulos enriquecidos con portadas y sinopsis oficiales`;
    finishMediaOperation(true, 'Metadatos actualizados', msg, '✅');
    addLog('success', `Metadatos: ${msg}`);
    await loadMediaLibrary();
  } catch (e) {
    finishMediaOperation(false, 'Error actualizando metadatos', e.message, '❌');
    addLog('error', `Error en metadatos: ${e.message}`);
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
}

async function triggerAutoGrab() {
  const btn = document.getElementById('btn-media-autograb');
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '<span>⏳</span> <span>Buscando...</span>'; }

  startMediaOperation('Ejecutando Auto-Grab...', 'Buscando lanzamientos en Kademlia DHT e indexadores Torznab...', '⚡');
  updateMediaOperation(35, 'Comprobando películas deseadas y episodios emitidos...');

  try {
    const res = await client.call('media.autoGrab', {});
    updateMediaOperation(85, 'Puntuando versiones con filtro Anti-Fake...');
    const moviesCount = (res.grabbedMovies || []).length;
    const epsCount = (res.grabbedEpisodes || []).length;
    const msg = `${moviesCount} películas y ${epsCount} episodios capturados y añadidos a descargas`;
    finishMediaOperation(true, 'Ronda de Auto-Grab finalizada', msg, '⚡');
    addLog('info', `Auto-Grab: ${msg}`);
    await loadMediaLibrary();
    try { refreshDownloads(); } catch (e) { console.warn('[PulseCore] error al refrescar descargas:', e); }
  } catch (e) {
    finishMediaOperation(false, 'Error en Auto-Grab', e.message, '❌');
    addLog('error', `Error en Auto-Grab: ${e.message}`);
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
}

async function triggerOrganize() {
  const btn = document.getElementById('btn-media-organize');
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '<span>⏳</span> <span>Organizando...</span>'; }

  startMediaOperation('Organizando biblioteca...', 'Revisando descargas completadas y creando enlaces duros (hardlinks)...', '🔗');
  updateMediaOperation(50, 'Estructurando carpetas para Jellyfin sin duplicar espacio en disco...');

  try {
    const res = await client.call('media.organize', {});
    const count = (res.organized || []).length;
    const msg = `${count} archivos organizados y enlazados con éxito en la biblioteca`;
    finishMediaOperation(true, 'Organizador completado', msg, '🔗');
    addLog('success', `Organizador: ${msg}`);
    await loadMediaLibrary();
  } catch (e) {
    finishMediaOperation(false, 'Error en organizador', e.message, '❌');
    addLog('error', `Error en organizador: ${e.message}`);
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
}

async function triggerCleanLibrary() {
  if (!confirm('¿Deseas ejecutar la limpieza de duplicados y archivos basura en la biblioteca y descargas? Los archivos repetidos de menor calidad y los residuos de incoming serán eliminados permanentemente.')) {
    return;
  }
  const btn = document.getElementById('btn-media-clean');
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '<span>⏳</span> <span>Limpiando...</span>'; }

  startMediaOperation('Limpiando duplicados y residuos...', 'Analizando series y películas para detectar versiones repetidas y archivos temporales huérfanos...', '🧹');
  updateMediaOperation(40, 'Comparando calidad y eliminando copias duplicadas y basura de incoming...');

  try {
    const res = await client.call('media.clean', { dryRun: false });
    const summary = res.summary || 'Limpieza finalizada con éxito';
    const mbFreed = Math.round((res.bytesFreed || 0) / (1024 * 1024));
    finishMediaOperation(true, 'Limpieza completada', `${summary} (${mbFreed} MB liberados)`, '🧹');
    addLog('success', `Limpieza de biblioteca: ${summary}`);
    await loadMediaLibrary();
  } catch (e) {
    finishMediaOperation(false, 'Error en limpieza', e.message, '❌');
    addLog('error', `Error en limpieza: ${e.message}`);
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
}

    window.analyzeDuplicatesPreview = async function() {
  const btn = document.getElementById('btn-analyze-duplicates');
  const box = document.getElementById('dup-preview-box');
  if (!box) return;
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '⏳ Analizando...'; }

  try {
    const res = await client.call('media.clean', { dryRun: true });
    const pairs = Array.isArray(res.duplicatePairs) ? res.duplicatePairs : [];
    const mbFreed = ((res.bytesFreed || 0) / (1024 * 1024)).toFixed(0);
    box.style.display = 'block';

    if (pairs.length === 0) {
      box.innerHTML = `
        <div style="display: flex; align-items: center; justify-content: space-between; gap: 8px;">
          <span style="color: #34d399; font-weight: 600;">✓ No se han detectado archivos duplicados en la Filmoteca.</span>
          <span style="font-size: 0.75rem; color: var(--text-muted);">${escapeHtml(res.summary || '')}</span>
        </div>
      `;
      return;
    }

    box.innerHTML = `
      <div style="display: flex; align-items: center; justify-content: space-between; gap: 10px; flex-wrap: wrap; margin-bottom: 10px;">
        <div>
          <strong style="color: #fbbf24;">⚠️ ${pairs.length} ${pairs.length === 1 ? 'duplicado detectado' : 'duplicados detectados'}</strong>
          <span style="font-size: 0.76rem; color: var(--text-muted); margin-left: 8px;">Espacio recuperable: ${mbFreed} MB</span>
        </div>
        <button type="button" class="btn btn-sm btn-danger" onclick="executeDuplicatesCleanup()">🗑 Eliminar ${pairs.length} ${pairs.length === 1 ? 'duplicado' : 'duplicados'}</button>
      </div>
      <div style="display: flex; flex-direction: column; gap: 8px; max-height: 280px; overflow-y: auto;">
        ${pairs.map(p => `
          <div class="dup-pair-card">
            <div style="font-weight: 700; font-size: 0.8rem; color: #fff; margin-bottom: 4px;">
              ${p.kind === 'episode' ? '📺' : '🎬'} ${escapeHtml(p.groupLabel || '')}
            </div>
            <div class="dup-file-kept">
              <span>✓ Conservar (${p.keptScore} pts · ${formatBytes(p.keptSize || 0)}):</span>
              <code>${escapeHtml(p.keptPath || '')}</code>
            </div>
            <div class="dup-file-removed">
              <span>✕ Eliminar (${p.removedScore} pts · ${formatBytes(p.removedSize || 0)}):</span>
              <code>${escapeHtml(p.removedPath || '')}</code>
            </div>
          </div>
        `).join('')}
      </div>
    `;
  } catch (e) {
    showToast(`Error analizando duplicados: ${e.message || e}`, 'error', '❌');
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
};

window.executeDuplicatesCleanup = async function() {
  const box = document.getElementById('dup-preview-box');
  try {
    const res = await client.call('media.clean', { dryRun: false });
    const mbFreed = Math.round((res.bytesFreed || 0) / (1024 * 1024));
    showToast(`Duplicados eliminados (${mbFreed} MB liberados)`, 'success', '🧹');
    if (box) {
      box.innerHTML = `<div style="color: #34d399; font-weight: 600;">✓ ${escapeHtml(res.summary || 'Limpieza completada')}</div>`;
    }
    if (typeof loadMediaLibrary === 'function') await loadMediaLibrary();
    if (typeof refreshShares === 'function') await refreshShares();
  } catch (e) {
    showToast(`Error al eliminar duplicados: ${e.message || e}`, 'error', '❌');
  }
};

// Modal Búsqueda de Títulos
function openMediaSearchModal() {
  const m = document.getElementById('modal-media-search');
  if (m) {
    m.classList.add('active');
    document.getElementById('media-search-input').focus();
  }
}

async function execMediaSearch() {
  const q = document.getElementById('media-search-input')?.value.trim();
  const type = document.getElementById('media-search-type')?.value || 'movie';
  const container = document.getElementById('media-search-results');
  if (!q || !container) return;

  container.innerHTML = `<div style="text-align:center; padding: 40px; color: var(--text-muted);"><div style="font-size: 1.5rem; margin-bottom:8px;">⏳</div>Buscando metadatos para "${escapeHtml(q)}"...</div>`;

  try {
    const items = await client.call('media.search', { query: q, type });
    if (!items || items.length === 0) {
      container.innerHTML = `<div style="text-align:center; padding: 40px; color: var(--text-muted);">No se encontraron títulos para "${escapeHtml(q)}".</div>`;
      return;
    }

    window.__searchResultItems = items;
    container.innerHTML = items.slice(0, 15).map((it, idx) => {
      const posterUrl = it.posterUrl || it.posterPath;
      const isTv = it.mediaType === 'tv';
      const typeBadge = isTv ? '<span class="badge" style="background: rgba(14, 165, 233, 0.2); color: #38bdf8; font-size: 0.68rem; margin-right: 6px;">📺 Serie TV</span>' : '<span class="badge" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; font-size: 0.68rem; margin-right: 6px;">🎬 Película</span>';
      const poster = posterUrl ? `<img src="${posterUrl}" style="width: 60px; height: 90px; object-fit: cover; border-radius: 4px; border: 1px solid var(--border);">` : `<div style="width: 60px; height: 90px; background: #1f2937; border-radius: 4px; display:flex; align-items:center; justify-content:center;">${isTv ? '📺' : '🎬'}</div>`;
      const year = it.year ? `(${it.year})` : '';
      const overview = it.overview ? it.overview.slice(0, 140) + '...' : 'Sin sinopsis disponible.';

      const seasonScopeSelect = isTv
        ? `<select id="search-season-scope-${idx}" class="form-control" style="width: auto; padding: 4px 8px; font-size: 0.74rem; height: 29px;" title="Elige qué temporadas descargar automáticamente">
             <option value="all">Todas las temporadas</option>
             <option value="latest">Solo última temporada</option>
             <option value="season_1">Solo Temporada 1</option>
           </select>`
        : '';

      return `
        <div style="background: rgba(255,255,255,0.02); border: 1px solid var(--border); border-radius: 6px; padding: 10px; display: flex; gap: 12px; align-items: center; flex-wrap: wrap;">
          ${poster}
          <div style="flex: 1; min-width: 180px;">
            <div style="font-weight: 700; font-size: 0.95rem; color: var(--text);">${typeBadge}${escapeHtml(it.title)} <span style="font-weight: normal; color: var(--text-muted); font-size: 0.85rem;">${year}</span></div>
            <div style="font-size: 0.78rem; color: var(--text-muted); line-height: 1.3; margin-top: 4px;">${escapeHtml(overview)}</div>
          </div>
          <div style="display: flex; align-items: center; gap: 6px; flex-shrink: 0;">
            ${seasonScopeSelect}
            <button class="btn btn-primary btn-sm" onclick="addMediaSearchResult(${idx}, this)">
              ➕ Añadir
            </button>
          </div>
        </div>
      `;
    }).join('');
  } catch (e) {
    container.innerHTML = `<div style="text-align:center; padding: 40px; color: #f85149;">Error buscando metadatos: ${escapeHtml(e.message)}</div>`;
  }
}

async function addMediaSearchResult(idx, btnEl) {
  const item = (window.__searchResultItems || [])[idx];
  if (!item) return;
  if (btnEl) {
    btnEl.disabled = true;
    btnEl.textContent = '⏳ Añadiendo...';
  }
  item.posterPath = item.posterPath || item.posterUrl;
  item.backdropPath = item.backdropPath || item.backdropUrl;
  const scopeEl = document.getElementById(`search-season-scope-${idx}`);
  const payload = Object.assign({}, item);
  if (scopeEl && scopeEl.value) {
    payload.seasonScope = scopeEl.value;
  }
  try {
    await client.call('media.add', payload);
    showToast(`Añadido "${item.title}" a la biblioteca`, 'success', '🎬');
    addLog('success', `Añadido título a biblioteca: ${item.title}`);
    closeModal('modal-media-search');
    await loadMediaLibrary();
  } catch (e) {
    showToast('Error añadiendo título: ' + e.message, 'error', '❌');
    if (btnEl) {
      btnEl.disabled = false;
      btnEl.textContent = '➕ Añadir';
    }
  }
}

let pendingDeleteMediaId = null;
let pendingDeleteMediaTitle = '';

function deleteMediaItem(id, fallbackTitle) {
  pendingDeleteMediaId = id;
  const item = (typeof mediaLibraryItems !== 'undefined') ? mediaLibraryItems.find(m => m.id === id) : null;
  const title = (item && item.title) ? item.title : (fallbackTitle || 'Título');
  pendingDeleteMediaTitle = title;

  const titleEl = document.getElementById('media-del-title');
  if (titleEl) titleEl.textContent = title;
  const metaEl = document.getElementById('media-del-meta');
  const posterEl = document.getElementById('media-del-poster');
  if (item) {
    const typeStr = (item.mediaType === 'tv' || item.media_type === 'tv') ? 'Serie de TV' : 'Película';
    const yearStr = item.year ? `(${item.year})` : '';
    if (metaEl) metaEl.textContent = `${typeStr} ${yearStr}`;
    const pPath = item.posterPath || item.poster_path;
    if (posterEl) {
      if (pPath) {
        posterEl.src = pPath;
        posterEl.style.display = 'block';
      } else {
        posterEl.style.display = 'none';
      }
    }
  } else {
    if (metaEl) metaEl.textContent = 'Filmoteca';
    if (posterEl) posterEl.style.display = 'none';
  }

  const chkCancel = document.getElementById('media-del-cancel-dls');
  if (chkCancel) chkCancel.checked = true;
  const chkDisk = document.getElementById('media-del-disk-files');
  if (chkDisk) chkDisk.checked = false;

  const promptEl = document.getElementById('media-del-prompt');
  if (promptEl) promptEl.style.display = 'block';
  const progWrap = document.getElementById('media-del-progress-wrap');
  if (progWrap) progWrap.style.display = 'none';
  const bar = document.getElementById('del-progress-bar');
  if (bar) {
    bar.style.width = '0%';
    bar.style.background = 'linear-gradient(90deg, #ef4444, #f97316)';
  }
  const pct = document.getElementById('del-progress-pct');
  if (pct) pct.textContent = '0%';
  const icon = document.getElementById('del-icon-status');
  if (icon) icon.textContent = '⏳';
  const progTitle = document.getElementById('del-progress-title');
  if (progTitle) progTitle.textContent = 'Eliminando título...';
  const progStep = document.getElementById('del-progress-step');
  if (progStep) progStep.textContent = 'Iniciando proceso...';

  const m = document.getElementById('modal-media-delete');
  if (m) m.classList.add('active');
}

async function executeMediaDeletion() {
  if (!pendingDeleteMediaId) return;
  const id = pendingDeleteMediaId;
  const title = pendingDeleteMediaTitle;
  const cancelDownloads = document.getElementById('media-del-cancel-dls').checked;
  const deleteFiles = document.getElementById('media-del-disk-files').checked;

  document.getElementById('media-del-prompt').style.display = 'none';
  const progressWrap = document.getElementById('media-del-progress-wrap');
  progressWrap.style.display = 'block';

  const bar = document.getElementById('del-progress-bar');
  const pct = document.getElementById('del-progress-pct');
  const step = document.getElementById('del-progress-step');
  const detail = document.getElementById('del-progress-detail');
  const icon = document.getElementById('del-icon-status');

  function updateProgress(percent, stepText, detailText) {
    bar.style.width = `${percent}%`;
    pct.textContent = `${percent}%`;
    step.textContent = stepText;
    if (detailText) detail.textContent = detailText;
  }

  try {
    updateProgress(15, 'Preparando desvinculación de la biblioteca...', 'Identificando');
    await new Promise(r => setTimeout(r, 220));

    updateProgress(35, cancelDownloads ? 'Cancelando descargas activas y colas asociadas...' : 'Verificando enjambres...', 'Descargas');
    await new Promise(r => setTimeout(r, 250));

    updateProgress(65, deleteFiles ? 'Eliminando registros y borrando archivos de vídeo del disco/NAS...' : 'Eliminando registros de episodios y biblioteca SQLite...', 'Base de datos');

    const res = await client.call('media.delete', {
      id,
      cancelDownloads,
      deleteFiles
    });

    updateProgress(90, 'Actualizando catálogo y sincronizando biblioteca...', 'Finalizando');
    await loadMediaLibrary();
    try { refreshDownloads(); } catch (e) { console.warn('[PulseCore] error al refrescar descargas:', e); }

    updateProgress(100, `¡"${title}" ha sido eliminado con éxito!`, 'Completado');
    bar.style.background = 'linear-gradient(90deg, #10b981, #22c55e)';
    icon.textContent = '✅';
    document.getElementById('del-progress-title').textContent = 'Eliminado correctamente';
    addLog('info', `Eliminado de la biblioteca: "${title}" (descargas canceladas: ${cancelDownloads}, archivos borrados: ${deleteFiles})`);

    await new Promise(r => setTimeout(r, 1100));
    closeModal('modal-media-delete');
    showToastNotification(`Eliminado "${title}" con éxito`, 'info', '🗑');
  } catch (err) {
    console.error('Error deleting media:', err);
    updateProgress(100, `Error durante la eliminación: ${err.message || err}`, 'Fallo');
    bar.style.background = '#dc2626';
    icon.textContent = '❌';
    document.getElementById('del-progress-title').textContent = 'Error al eliminar';
    setTimeout(() => {
      alert('Error eliminando título: ' + (err.message || JSON.stringify(err)));
      closeModal('modal-media-delete');
    }, 1500);
  } finally {
    pendingDeleteMediaId = null;
  }
}
window.deleteMediaItem = deleteMediaItem;
window.executeMediaDeletion = executeMediaDeletion;

if (document.getElementById('btn-exec-media-del')) {
  document.getElementById('btn-exec-media-del').onclick = executeMediaDeletion;
}

// Modal Lanzamientos (Grabber)
let currentSearchingMediaId = null;
let currentSearchingIsMovie = true;
let currentSearchingEpisodeId = null;
let currentSearchingSeason = null;
let currentSearchingEpisode = null;
let currentSearchingStrictCastilian = false;
let currentSearchingBlacklistCurrent = false;
let currentSearchingLabel = '';

function onStrictCastilianToggle() {
  const cb = document.getElementById('releases-strict-castilian');
  currentSearchingStrictCastilian = !!(cb && cb.checked);
  executeReleasesSearch();
}
window.onStrictCastilianToggle = onStrictCastilianToggle;

async function openReleasesModal(mediaId, title, isMovie, options = {}) {
  currentSearchingMediaId = mediaId;
  currentSearchingIsMovie = !!isMovie;
  currentSearchingEpisodeId = options.episodeId || null;
  currentSearchingSeason = Number.isFinite(options.seasonNumber) ? options.seasonNumber : null;
  currentSearchingEpisode = Number.isFinite(options.episodeNumber) ? options.episodeNumber : null;
  currentSearchingStrictCastilian = !!options.strictCastilian;
  currentSearchingBlacklistCurrent = !!options.blacklistCurrent;
  currentSearchingLabel = title || '';

  const m = document.getElementById('modal-media-releases');
  const titleEl = document.getElementById('releases-modal-title');
  const queryInput = document.getElementById('releases-custom-query');
  const strictCb = document.getElementById('releases-strict-castilian');
  const replaceBanner = document.getElementById('releases-replace-banner');

  if (titleEl) {
    titleEl.textContent = currentSearchingBlacklistCurrent
      ? `Siguiente en Castellano para: ${title}`
      : `Lanzamientos para: ${title}`;
  }
  if (queryInput) queryInput.value = options.customQuery || '';
  if (strictCb) strictCb.checked = currentSearchingStrictCastilian;
  if (replaceBanner) {
    replaceBanner.style.display = currentSearchingBlacklistCurrent ? 'flex' : 'none';
  }
  if (m) m.classList.add('active');

  await executeReleasesSearch();
}

function openCastilianReplacementModal(mediaId, title, isMovie, episodeId = null, seasonNumber = null, episodeNumber = null) {
  return openReleasesModal(mediaId, title, isMovie, {
    episodeId,
    seasonNumber,
    episodeNumber,
    strictCastilian: true,
    blacklistCurrent: true
  });
}
window.openCastilianReplacementModal = openCastilianReplacementModal;

async function autoGrabRecommendedCastilianFromModal() {
  if (!currentSearchingMediaId) return;
  closeModal('modal-media-releases');
  await triggerReplaceWrongLanguage(
    currentSearchingMediaId,
    currentSearchingEpisodeId,
    currentSearchingSeason,
    currentSearchingLabel,
    true,
    true
  );
}
window.autoGrabRecommendedCastilianFromModal = autoGrabRecommendedCastilianFromModal;

async function executeReleasesSearch() {
  if (!currentSearchingMediaId) return;
  const loadingEl = document.getElementById('releases-loading-msg');
  const tableEl = document.getElementById('releases-table');
  const tbody = document.getElementById('releases-table-body');
  const queryInput = document.getElementById('releases-custom-query');
  const strictCb = document.getElementById('releases-strict-castilian');
  const customQuery = queryInput ? queryInput.value.trim() : '';
  currentSearchingStrictCastilian = !!(strictCb && strictCb.checked);

  if (loadingEl) {
    loadingEl.innerHTML = `<div style="font-size: 1.5rem; margin-bottom:8px;">⏳</div>Buscando en enjambres eD2k y trackers${currentSearchingStrictCastilian ? ' (solo Castellano ES)' : ''}${customQuery ? ' (' + escapeHtml(customQuery) + ')' : ''}...`;
    loadingEl.style.display = 'block';
  }
  if (tableEl) tableEl.style.display = 'none';
  if (tbody) tbody.innerHTML = '';

  try {
    const payload = {
      mediaId: currentSearchingMediaId,
      isMovie: currentSearchingIsMovie,
      strictCastilian: currentSearchingStrictCastilian
    };
    if (currentSearchingEpisodeId) payload.episodeId = currentSearchingEpisodeId;
    if (currentSearchingSeason !== null) payload.seasonNumber = currentSearchingSeason;
    if (currentSearchingEpisode !== null) payload.episodeNumber = currentSearchingEpisode;
    if (customQuery) {
      payload.query = customQuery;
    }
    const releases = await client.call('media.searchReleases', payload);
    if (loadingEl) loadingEl.style.display = 'none';

    if (!releases || releases.length === 0) {
      if (tbody) tbody.innerHTML = `<tr><td colspan="6" style="text-align:center; padding: 30px; color: var(--text-muted);">No se encontraron lanzamientos en Kad ni en trackers. Prueba con otro término arriba.</td></tr>`;
      if (tableEl) tableEl.style.display = 'table';
      return;
    }

    const firstValidIdx = releases.findIndex(r => !r.isRejected && !r.isBlacklisted);

    if (tbody) {
      tbody.innerHTML = releases.map((r, idx) => {
        const protoBadge = r.protocol === 'torrent' ? '<span class="proto-badge proto-torrent">⚡ Torrent</span>' : '<span class="proto-badge proto-ed2k">🫏 eD2k</span>';
        const sizeMb = (r.sizeBytes / (1024 * 1024)).toFixed(1) + ' MB';
        const scoreClass = r.score >= 50 ? 'color: #22c55e;' : (r.score > 0 ? 'color: #eab308;' : 'color: #f85149;');
        const rejectedNote = r.isRejected ? `<div style="font-size: 0.72rem; color: #f85149;">❌ ${escapeHtml(r.rejectionReason || 'Rechazado')}</div>` : '';
        const blacklistedBadge = r.isBlacklisted ? `<span class="badge" style="background: rgba(248,81,73,0.16); color: #f85149; font-size: 0.7rem;">⛔ Descartada</span>` : '';
        const recommendedBadge = idx === firstValidIdx ? `<span class="badge" style="background: rgba(34,197,94,0.2); color: #4ade80; font-size: 0.7rem; margin-right: 4px;">★ Recomendada</span>` : '';
        let confBadge = '';
        if (r.castilianConfidence === 'verified') {
          confBadge = `<span class="badge" style="background: rgba(34,197,94,0.18); color: #4ade80; font-size: 0.7rem;">✅ Castellano ES</span>`;
        } else if (r.castilianConfidence === 'dual') {
          confBadge = `<span class="badge" style="background: rgba(56,189,248,0.18); color: #38bdf8; font-size: 0.7rem;">🇪🇸 Dual ES/EN</span>`;
        } else if (r.castilianConfidence === 'possible') {
          confBadge = `<span class="badge" style="background: rgba(234,179,8,0.18); color: #fde047; font-size: 0.7rem;">🟡 Posible ES</span>`;
        }
        const actionLabel = currentSearchingBlacklistCurrent ? '🔄 Reemplazar' : '⬇ Descargar';
        const rowHighlight = idx === firstValidIdx ? 'background: rgba(34,197,94,0.06);' : '';

        return `
          <tr class="release-table-row" style="${r.isRejected ? 'opacity: 0.6;' : rowHighlight}">
            <td>
              <div style="font-weight: 600; font-size: 0.84rem; word-break: break-all;">${recommendedBadge}${escapeHtml(r.title)}</div>
              ${rejectedNote}
              <div style="font-size: 0.72rem; color: var(--text-muted); display: flex; gap: 6px; flex-wrap: wrap; align-items: center; margin-top: 2px;">
                <span>${escapeHtml(r.indexerName || 'Indexer')} • Fuentes/Seeds: ${r.sources || 0}</span>
                ${confBadge}
                ${blacklistedBadge}
              </div>
            </td>
            <td>${protoBadge}</td>
            <td style="font-family: var(--font-mono); font-size: 0.8rem;">${sizeMb}</td>
            <td>
              <span class="badge" style="background: rgba(255,255,255,0.05); font-size: 0.72rem;">${escapeHtml(r.quality || 'HD')}</span>
              <span class="badge" style="background: rgba(255,255,255,0.05); font-size: 0.72rem;">${escapeHtml(r.language || 'Multi')}</span>
            </td>
            <td style="font-weight: 700; ${scoreClass}">${r.score} pts</td>
            <td>
              <button class="btn btn-sm btn-primary" onclick='grabSpecificRelease(${idx})' ${r.isRejected ? 'disabled' : ''}>
                ${actionLabel}
              </button>
            </td>
          </tr>
        `;
      }).join('');
    }
    window.__currentReleases = releases;
    if (tableEl) tableEl.style.display = 'table';
  } catch (e) {
    if (loadingEl) loadingEl.innerHTML = `<div style="color: #f85149; text-align:center; padding: 40px;">Error buscando lanzamientos: ${escapeHtml(e.message)}</div>`;
  }
}

async function grabSpecificRelease(idx) {
  const release = (window.__currentReleases || [])[idx];
  if (!release || !currentSearchingMediaId) return;

  try {
    const payload = {
      mediaId: currentSearchingMediaId,
      release: release,
      blacklistCurrent: currentSearchingBlacklistCurrent
    };
    if (currentSearchingEpisodeId) {
      payload.episodeId = currentSearchingEpisodeId;
    }
    await client.call('media.grabRelease', payload);
    showToastNotification(`Lanzamiento capturado: ${release.title}`, 'success', '⚡');
    addLog('success', `Lanzamiento añadido a descargas: ${release.title}`);
    closeModal('modal-media-releases');
    if (typeof loadMediaLibrary === 'function') await loadMediaLibrary();
    if (currentMediaDetailState && currentMediaDetailState.item && currentMediaDetailState.item.id === currentSearchingMediaId) {
      await openMediaDetail(currentSearchingMediaId);
    }
  } catch (e) {
    showToastNotification('Error capturando lanzamiento: ' + e.message, 'error', '❌');
  }
}

async function triggerReplaceWrongLanguage(mediaId, episodeId, seasonNumber, label, autoGrab = true, skipConfirm = false) {
  const isSeasonScope = seasonNumber !== null && seasonNumber !== undefined && !episodeId;
  if (isSeasonScope && !skipConfirm) {
    const ok = confirm(`¿Descartar la versión actual de "${label}" y buscar la siguiente recomendación en Castellano de España? Los archivos actuales se mantendrán hasta que terminen los nuevos.`);
    if (!ok) return;
  }

  showToastNotification(`🇪🇸 Buscando siguiente recomendación en Castellano para "${label}"...`, 'info', '🔄');
  try {
    const payload = {
      mediaId,
      autoGrab: Boolean(autoGrab),
      cancelActiveDownload: true
    };
    if (episodeId) payload.episodeId = episodeId;
    if (seasonNumber !== null && seasonNumber !== undefined) payload.seasonNumber = seasonNumber;

    const res = await client.call('media.replaceWrongLanguage', payload);
    if (res && res.success) {
      if (res.grabbed) {
        showToastNotification(
          `✅ Reemplazo iniciado: ${res.grabbed.title} [${res.grabbed.quality} · ${res.grabbed.language} · ${res.grabbed.score} pts]`,
          'success',
          '🇪🇸'
        );
      } else if (res.grabbedEpisodes && res.grabbedEpisodes.length > 0) {
        showToastNotification(
          `✅ Reemplazo iniciado para ${res.grabbedEpisodes.length} episodio(s) en Castellano`,
          'success',
          '🇪🇸'
        );
      } else {
        showToastNotification('Búsqueda de reemplazo completada', 'info', '🔎');
      }
    } else {
      showToastNotification(
        (res && res.message) || 'No se encontró ninguna alternativa verificada en Castellano de España. El archivo actual se mantiene intacto.',
        'warning',
        '⚠️'
      );
    }
    await openMediaDetail(mediaId);
    if (typeof loadMediaLibrary === 'function') loadMediaLibrary();
  } catch (e) {
    showToastNotification(`Error cambiando a Castellano: ${e.message || e}`, 'error', '❌');
  }
}
window.triggerReplaceWrongLanguage = triggerReplaceWrongLanguage;

// Estado en memoria de la serie abierta actualmente para cambio instantáneo de temporada
let currentMediaDetailState = {
  item: null,
  episodes: [],
  details: null,
  subtitles: [],
  selectedSeason: 1
};

function formatRuntimeMinutes(mins) {
  if (!mins || mins <= 0) return '';
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h > 0 && m > 0) return `${h}h ${m}m`;
  if (h > 0) return `${h}h`;
  return `${m} min`;
}

function getInitials(name) {
  if (!name) return '?';
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map(part => part[0].toUpperCase())
    .join('');
}

function renderPersonCardHtml(person, highlightClass = '') {
  const initials = escapeHtml(getInitials(person.name));
  const avatarInner = person.photoUrl
    ? `<img src="${escapeHtml(person.photoUrl)}" alt="${escapeHtml(person.name)}" loading="lazy" onerror="this.style.display='none'; this.parentElement.textContent='${initials}';">`
    : initials;
  const roleText = person.role ? escapeHtml(person.role) : (person.department === 'acting' ? 'Reparto' : 'Equipo');
  return `
    <div class="credit-person-card ${highlightClass}">
      <div class="credit-avatar">${avatarInner}</div>
      <div style="min-width: 0; flex: 1;">
        <div style="font-weight: 700; font-size: 0.88rem; color: #f8fafc; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;" title="${escapeHtml(person.name)}">
          ${escapeHtml(person.name)}
        </div>
        <div style="font-size: 0.76rem; color: #94a3b8; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;" title="${roleText}">
          ${roleText}
        </div>
      </div>
    </div>
  `;
}

// Modal Detalle de Película o Serie (Vista Panorámica 1080p)
async function openMediaDetail(mediaId) {
  const m = document.getElementById('modal-media-detail');
  if (m) m.classList.add('active');

  const loadingEl = document.getElementById('media-sheet-loading');
  const scrollContainer = document.getElementById('media-sheet-scroll-container');
  const titleLoadingEl = document.getElementById('media-loading-title');
  const subLoadingEl = document.getElementById('media-loading-subtitle');
  const iconLoadingEl = document.getElementById('media-spinner-icon');

  // Limpiar rastro visual del título y póster previo
  const posterWrapEl = document.getElementById('series-detail-poster-wrap');
  const posterEl = document.getElementById('series-detail-poster');
  const backdropEl = document.getElementById('media-sheet-backdrop');
  const titleEl = document.getElementById('series-detail-title');
  if (posterWrapEl) posterWrapEl.style.display = 'none';
  if (posterEl) posterEl.src = '';
  if (backdropEl) backdropEl.style.backgroundImage = 'none';
  if (titleEl) titleEl.textContent = '';

  // Precargar datos del elemento si ya están en memoria en mediaLibraryItems
  const cachedItem = (Array.isArray(mediaLibraryItems) ? mediaLibraryItems : []).find(x => x.id === mediaId);
  if (cachedItem) {
    if (titleLoadingEl) titleLoadingEl.textContent = cachedItem.title || 'Cargando información multimedia...';
    if (subLoadingEl) {
      subLoadingEl.textContent = cachedItem.mediaType === 'tv'
        ? 'Cargando temporadas, episodios y ficha técnica...'
        : 'Cargando ficha técnica y metadatos...';
    }
    if (iconLoadingEl) iconLoadingEl.textContent = cachedItem.mediaType === 'tv' ? '📺' : '🎬';
  } else {
    if (titleLoadingEl) titleLoadingEl.textContent = 'Cargando información multimedia...';
    if (subLoadingEl) subLoadingEl.textContent = 'Consultando catálogo y metadatos técnicos';
    if (iconLoadingEl) iconLoadingEl.textContent = '🎬';
  }

  // Ocultar scroll container y desplegar el círculo de carga elegante
  if (scrollContainer) {
    scrollContainer.style.opacity = '0';
    scrollContainer.style.display = 'none';
    scrollContainer.scrollTop = 0;
  }
  if (loadingEl) loadingEl.style.display = 'flex';

  const minDisplayTime = new Promise(r => setTimeout(r, 450));

  try {
    const [res] = await Promise.all([
      client.call('media.get', { id: mediaId }),
      minDisplayTime
    ]);
    const item = res.item;
    const episodes = res.episodes || [];
    const details = res.details || {};
    const subtitles = res.subtitles || [];
    const mediaProbe = res.mediaProbe || null;
    const episodeProbes = res.episodeProbes || {};
    const autoGrab = res.autoGrab || {};
    const isMovie = item.mediaType === 'movie';

    currentMediaDetailState = {
      item,
      episodes,
      details,
      subtitles,
      mediaProbe,
      episodeProbes,
      autoGrab,
      selectedSeason: 1
    };

    // 1. Backdrop / Fanart
    const backdropEl = document.getElementById('media-sheet-backdrop');
    const posterUrl = details.posterUrl || item.posterPath || (cachedItem ? cachedItem.posterPath : '');
    const backdropUrl = details.backdropUrl || item.backdropPath || (cachedItem ? cachedItem.backdropPath : '') || posterUrl;
    if (backdropEl) {
      if (backdropUrl) {
        backdropEl.style.backgroundImage = `url('${backdropUrl.replace(/'/g, "\\'")}')`;
        backdropEl.style.display = 'block';
      } else {
        backdropEl.style.backgroundImage = 'none';
        backdropEl.style.display = 'none';
      }
    }

    // 2. Póster
    const posterWrapEl = document.getElementById('series-detail-poster-wrap');
    const posterEl = document.getElementById('series-detail-poster');
    if (posterEl && posterWrapEl) {
      if (posterUrl) {
        posterEl.style.display = 'block';
        posterEl.src = posterUrl;
        posterWrapEl.style.display = 'block';

        // Precarga en memoria para asegurar que la portada esté lista
        if (!posterEl.complete) {
          await Promise.race([
            new Promise(r => { posterEl.onload = r; posterEl.onerror = r; }),
            new Promise(r => setTimeout(r, 500))
          ]);
        }
      } else {
        posterWrapEl.style.display = 'none';
      }
    }

    // 3. Badges Superiores Diferenciados (Película vs Serie)
    const topBadgesEl = document.getElementById('media-sheet-top-badges');
    if (topBadgesEl) {
      const badges = [];
      if (isMovie) {
        badges.push(`<span class="media-type-pill movie"><span id="series-detail-icon">🎬</span><span>PELÍCULA DE CINE</span></span>`);
      } else {
        badges.push(`<span class="media-type-pill series"><span id="series-detail-icon">📺</span><span>SERIE DE TV</span></span>`);
      }

      if (details.studioOrNetwork) {
        badges.push(`<span class="media-meta-chip">🏛️ ${escapeHtml(details.studioOrNetwork)}</span>`);
      }
      if (details.certification) {
        badges.push(`<span class="media-meta-chip" style="font-weight:700; color:#fbbf24;">🔞 ${escapeHtml(details.certification)}</span>`);
      }
      if (details.runtimeMins) {
        badges.push(`<span class="media-meta-chip">⏱️ ${formatRuntimeMinutes(details.runtimeMins)}${isMovie ? '' : ' / ep'}</span>`);
      }
      if (!isMovie && details.showStatus) {
        const stMap = {
          'Continuing': '🟢 En emisión',
          'Ended': '🏁 Finalizada',
          'Upcoming': '⏳ Próximamente'
        };
        badges.push(`<span class="media-meta-chip">${stMap[details.showStatus] || escapeHtml(details.showStatus)}</span>`);
      }

      if (isMovie) {
        const stBadge = item.status === 'completed'
          ? '<span class="badge badge-dl" style="font-size:0.78rem; padding:4px 10px;">🟢 En Biblioteca</span>'
          : (item.status === 'downloading'
            ? '<span class="badge badge-ed2k" style="font-size:0.78rem; padding:4px 10px;">⬇ Descargando</span>'
            : '<span class="badge badge-wait" style="font-size:0.78rem; padding:4px 10px;">🟡 Buscada</span>');
        badges.push(stBadge);
      } else {
        badges.push(`<span class="media-meta-chip">${item.monitored ? '🔔 Monitoreada' : '🔕 No monitoreada'}</span>`);
      }
      topBadgesEl.innerHTML = badges.join('');
    }

    // 4. Título y Título Original
    const yearSuffix = item.year ? ` (${item.year})` : '';
    const displayTitle = (yearSuffix && !item.title.endsWith(yearSuffix))
      ? `${item.title}${yearSuffix}`
      : item.title;
    document.getElementById('series-detail-title').textContent = displayTitle;
    const origTitleEl = document.getElementById('media-sheet-original-title');
    if (origTitleEl) {
      if (item.originalTitle && item.originalTitle.toLowerCase() !== item.title.toLowerCase()) {
        origTitleEl.textContent = `Título original: ${item.originalTitle}`;
        origTitleEl.style.display = 'block';
      } else {
        origTitleEl.style.display = 'none';
      }
    }

    // 5. Puntuaciones Oficiales y Géneros
    const ratingsGenresEl = document.getElementById('media-sheet-ratings-genres');
    if (ratingsGenresEl) {
      const rgHtml = [];
      const ratings = details.ratings || {};
      if (ratings.tmdb && ratings.tmdb > 0) {
        rgHtml.push(`<span class="media-rating-badge" style="color:#38bdf8;" title="Puntuación TMDB / TVDB">⭐ ${Number(ratings.tmdb).toFixed(1)} <span style="font-size:0.7rem; color:#94a3b8; font-weight:500;">TMDB</span></span>`);
      }
      if (ratings.imdb && ratings.imdb > 0) {
        rgHtml.push(`<span class="media-rating-badge" style="color:#facc15;" title="Puntuación IMDb">🟡 ${Number(ratings.imdb).toFixed(1)} <span style="font-size:0.7rem; color:#94a3b8; font-weight:500;">IMDb</span></span>`);
      }
      if (ratings.rottenTomatoes && ratings.rottenTomatoes > 0) {
        rgHtml.push(`<span class="media-rating-badge" style="color:#f87171;" title="Rotten Tomatoes">🍅 ${Math.round(ratings.rottenTomatoes)}%</span>`);
      }
      if (ratings.metacritic && ratings.metacritic > 0) {
        rgHtml.push(`<span class="media-rating-badge" style="color:#4ade80;" title="Metacritic">🟩 ${Math.round(ratings.metacritic)} <span style="font-size:0.7rem; color:#94a3b8; font-weight:500;">Meta</span></span>`);
      }
      (details.genres || []).forEach(g => {
        rgHtml.push(`<span class="media-genre-tag">${escapeHtml(g)}</span>`);
      });
      ratingsGenresEl.innerHTML = rgHtml.join('');
      ratingsGenresEl.style.display = rgHtml.length > 0 ? 'flex' : 'none';
    }

    // 6. Sinopsis
    document.getElementById('series-detail-overview').textContent = item.overview || 'Sin descripción disponible todavía.';

    // 7. Estadísticas Rápidas / Barra de progreso y Botones de Acción
    const statsEl = document.getElementById('series-detail-stats');
    const actionsEl = document.getElementById('movie-detail-actions');
    const movieSpecificEl = document.getElementById('media-sheet-movie-specific');
    const tableWrapEl = document.getElementById('series-episodes-table-wrap');

    const safeTitleAttr = escapeHtml(item.title).replace(/'/g, "\\'");

    if (isMovie) {
      if (tableWrapEl) tableWrapEl.style.display = 'none';
      if (movieSpecificEl) movieSpecificEl.style.display = 'block';

      if (statsEl) {
        const directorsNames = (details.directors || [])
          .filter(d => d.department === 'directing')
          .map(d => escapeHtml(d.name))
          .join(', ');
        statsEl.innerHTML = directorsNames
          ? `<div style="font-size: 0.88rem; color: #e2e8f0;">🎬 Dirigida por <strong style="color:#fbbf24;">${directorsNames}</strong> • Perfil de calidad: <strong>${escapeHtml(item.qualityProfile || '1080p')}</strong></div>`
          : `<div style="font-size: 0.88rem; color: #e2e8f0;">Perfil de calidad objetivo: <strong>${escapeHtml(item.qualityProfile || '1080p')}</strong></div>`;
      }

      if (actionsEl) {
        const safeFilePathAttr = escapeHtml(item.filePath || '').replace(/'/g, "\\'");
        actionsEl.style.display = 'flex';
        actionsEl.innerHTML = `
          <button class="btn" style="padding: 8px 14px; background: rgba(56,189,248,0.16); border-color: rgba(56,189,248,0.4); color: #38bdf8; font-weight: 600;" onclick="triggerSingleAutoGrab('${item.id}', null, '${safeTitleAttr}')" title="Ejecutar AutoGrabber de precisión ahora">${item.status === 'completed' ? '🔄 Buscar Upgrade (Auto-Grab)' : '⚡ Auto-Grab Ahora'}</button>
          ${(item.status === 'completed' || item.status === 'downloading') ? `<button class="btn" style="padding: 8px 14px; background: rgba(234,179,8,0.14); border-color: rgba(234,179,8,0.38); color: #fde047; font-weight: 600;" onclick="closeModal('modal-media-detail'); openCastilianReplacementModal('${item.id}', '${safeTitleAttr}', true)" title="Si el archivo no está en Castellano de España, descartarlo y elegir o bajar la siguiente recomendación en español">🇪🇸 Cambiar versión (ES)</button>` : ''}
          <button class="btn" style="padding: 8px 14px;" onclick="closeModal('modal-media-detail'); openReleasesModal('${item.id}', '${safeTitleAttr}', true)" title="Buscar lanzamientos manualmente en Kad y trackers">🔍 Buscar Fuentes</button>
          <button class="btn" style="padding: 8px 14px;" onclick="openDestinationFolder({ mediaId: '${item.id}', path: '${safeFilePathAttr}', title: '${safeTitleAttr}' })" title="Abrir carpeta de destino en el servidor">📂 Abrir carpeta</button>
          ${details.trailerYoutubeId ? `<a class="btn" style="padding: 8px 14px; text-decoration: none;" href="https://www.youtube.com/watch?v=${encodeURIComponent(details.trailerYoutubeId)}" target="_blank" rel="noopener">🎞️ Ver Tráiler</a>` : ''}
          ${item.status === 'completed' ? `<button class="btn" style="padding: 8px 14px;" onclick="closeModal('modal-media-detail'); openSubtitlesModal('${item.id}', '${safeTitleAttr}', true)" title="Descargar subtítulos (Bazarr)">💬 Subtítulos (Bazarr)</button>` : ''}
          <button class="btn" style="padding: 8px 14px; color:#f85149; border-color: rgba(248,81,73,0.3);" onclick="closeModal('modal-media-detail'); deleteMediaItem('${item.id}')">🗑 Eliminar Película</button>
        `;
      }

      // Poblar paneles exclusivos de Película (Ficha Técnica + Archivo Local + AutoGrabber)
      renderMovieSpecificPanels(item, details, subtitles, safeTitleAttr, autoGrab, mediaProbe);
    } else {
      if (movieSpecificEl) movieSpecificEl.style.display = 'none';
      if (tableWrapEl) tableWrapEl.style.display = 'block';

      const completedCount = episodes.filter(e => e.status === 'completed').length;
      const totalCount = episodes.length;
      const pct = totalCount > 0 ? Math.round((completedCount / totalCount) * 100) : 0;
      const seasonsSet = new Set(episodes.map(e => e.seasonNumber));

      if (statsEl) {
        statsEl.innerHTML = `
          <div style="display: flex; flex-direction: column; gap: 10px; max-width: 820px;">
            <div style="background: rgba(15, 23, 42, 0.72); border: 1px solid rgba(255,255,255,0.1); border-radius: 10px; padding: 10px 16px;">
              <div style="display: flex; justify-content: space-between; align-items: center; font-size: 0.82rem; margin-bottom: 6px;">
                <span>📦 <strong>${seasonsSet.size}</strong> Temporada(s) • <strong>${completedCount} / ${totalCount}</strong> episodios en biblioteca</span>
                <span style="font-weight: 700; color: ${pct === 100 ? '#4ade80' : '#38bdf8'};">${pct}% completado</span>
              </div>
              <div style="width: 100%; height: 7px; background: rgba(255,255,255,0.08); border-radius: 999px; overflow: hidden;">
                <div style="width: ${pct}%; height: 100%; background: linear-gradient(90deg, #38bdf8, #22c55e); border-radius: 999px; transition: width 0.3s ease;"></div>
              </div>
            </div>
            ${renderAutoGrabDiagnosticBanner(item.id, autoGrab)}
          </div>
        `;
      }

      if (actionsEl) {
        actionsEl.style.display = 'flex';
        actionsEl.innerHTML = `
          <button class="btn" style="padding: 8px 16px; background: rgba(56,189,248,0.16); border-color: rgba(56,189,248,0.4); color: #38bdf8; font-weight: 600;" onclick="triggerSingleAutoGrab('${item.id}', null, '${safeTitleAttr}')" title="Ejecutar AutoGrabber sobre capítulos pendientes">⚡ Auto-Grab Serie Ahora</button>
          <button class="btn btn-primary" style="padding: 8px 16px;" onclick="closeModal('modal-media-detail'); openReleasesModal('${item.id}', '${safeTitleAttr}', false)" title="Buscar lanzamientos de la serie">🔍 Buscar Fuentes de la Serie</button>
          <button class="btn" style="padding: 8px 14px;" onclick="openDestinationFolder({ mediaId: '${item.id}', title: '${safeTitleAttr}' })" title="Abrir carpeta de la serie en el servidor">📂 Abrir carpeta</button>
          <button class="btn" style="padding: 8px 14px; color:#f85149; border-color: rgba(248,81,73,0.3);" onclick="closeModal('modal-media-detail'); deleteMediaItem('${item.id}')">🗑 Eliminar Serie</button>
        `;
      }
    }

    // 8. Poblar Sección de Dirección / Creadores y Reparto con Fotografías
    renderCreditsSection(isMovie, details);

    // 9. Si es Serie, construir Pestañas de Temporadas y Tarjetas de Episodios 1080p
    if (!isMovie) {
      initSeriesSeasonsAndEpisodes();
    }

    // Transición suave: ocultar spinner y mostrar contenido cargado
    if (loadingEl) loadingEl.style.display = 'none';
    if (scrollContainer) {
      scrollContainer.style.display = 'block';
      requestAnimationFrame(() => {
        scrollContainer.style.transition = 'opacity 0.22s ease-out';
        scrollContainer.style.opacity = '1';
      });
    }
  } catch (e) {
    if (titleLoadingEl) titleLoadingEl.textContent = 'No se pudo cargar la información';
    if (subLoadingEl) subLoadingEl.textContent = e.message || 'Error consultando los detalles multimedia';
    showToastNotification('Error cargando detalles: ' + e.message, 'error', '❌');
  }
}
const openSeriesDetail = openMediaDetail;

function renderCreditsSection(isMovie, details) {
  const sectionEl = document.getElementById('media-sheet-credits-section');
  const dirBlockEl = document.getElementById('media-sheet-directors-block');
  const dirTitleEl = document.getElementById('media-sheet-directors-title');
  const dirListEl = document.getElementById('media-sheet-directors-list');
  const castBlockEl = document.getElementById('media-sheet-cast-block');
  const castListEl = document.getElementById('media-sheet-cast-list');

  const directors = (details && details.directors) || [];
  const cast = (details && details.cast) || [];

  if (directors.length === 0 && cast.length === 0) {
    if (sectionEl) sectionEl.style.display = 'none';
    return;
  }
  if (sectionEl) sectionEl.style.display = 'block';

  if (directors.length > 0 && dirBlockEl && dirListEl) {
    dirBlockEl.style.display = 'block';
    if (dirTitleEl) {
      dirTitleEl.innerHTML = isMovie
        ? '🎬 Dirección y Equipo Creativo'
        : '🎬 Creadores, Showrunners y Producción Ejecutiva';
    }
    const highlightClass = isMovie ? 'director-highlight' : 'creator-highlight';
    dirListEl.innerHTML = directors
      .map(person => renderPersonCardHtml(person, highlightClass))
      .join('');
  } else if (dirBlockEl) {
    dirBlockEl.style.display = 'none';
  }

  if (cast.length > 0 && castBlockEl && castListEl) {
    castBlockEl.style.display = 'block';
    castListEl.innerHTML = cast
      .map(actor => renderPersonCardHtml(actor, ''))
      .join('');
  } else if (castBlockEl) {
    castBlockEl.style.display = 'none';
  }
}

function renderAutoGrabDiagnosticBanner(mediaId, autoGrab) {
  if (!autoGrab) return '';
  const diag = autoGrab.diagnostic;
  const blacklistedCount = autoGrab.totalBlacklisted || 0;
  if (!diag && blacklistedCount === 0) return '';

  let statusLabel = '🤖 AutoGrabber Activo';
  let borderCol = 'rgba(56, 189, 248, 0.3)';
  let bgCol = 'rgba(15, 23, 42, 0.78)';
  let textCol = '#e2e8f0';

  if (diag) {
    if (diag.lastStatus === 'grabbed' || diag.lastStatus === 'upgrade_grabbed') {
      statusLabel = diag.lastStatus === 'upgrade_grabbed'
        ? '🔄 Auto-Upgrade en Descarga'
        : '✅ Seleccionada por AutoGrabber';
      borderCol = 'rgba(34, 197, 94, 0.35)';
      textCol = '#86efac';
    } else if (diag.lastStatus === 'stalled_failover') {
      statusLabel = '🛡️ Auto-Curación (Failover Aplicado)';
      borderCol = 'rgba(251, 191, 36, 0.45)';
      textCol = '#fde047';
    } else if (diag.lastStatus === 'rejected_all' || diag.lastStatus === 'no_results') {
      statusLabel = '⏳ Smart Backoff (Esperando Release Válida)';
      borderCol = 'rgba(248, 113, 113, 0.35)';
      textCol = '#fca5a5';
    }
  }

  const lastSearchStr = (diag && diag.lastSearchMs)
    ? new Date(diag.lastSearchMs).toLocaleTimeString('es-ES', { hour: '2-digit', minute: '2-digit' })
    : '';

  return `
    <div style="background: ${bgCol}; border: 1px solid ${borderCol}; border-radius: 9px; padding: 10px 14px; font-size: 0.8rem;">
      <div style="display: flex; justify-content: space-between; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 4px;">
        <span style="font-weight: 700; color: ${textCol};">${statusLabel}${lastSearchStr ? ` <span style="font-weight:400; color:#94a3b8; font-size:0.74rem;">(última pasada ${lastSearchStr})</span>` : ''}</span>
        <div style="display: flex; align-items: center; gap: 8px;">
          ${blacklistedCount > 0 ? `
            <span class="badge" style="background: rgba(248,81,73,0.18); color: #fca5a5; font-size: 0.72rem;">🚫 ${blacklistedCount} en Lista Negra</span>
            <button class="btn btn-sm" style="padding: 2px 8px; font-size: 0.7rem;" onclick="clearMediaBlacklist('${mediaId}', '${mediaId}')" title="Limpiar hashes descartados y reiniciar Backoff">🧹 Limpiar</button>
          ` : ''}
        </div>
      </div>
      ${diag && diag.summaryMessage ? `<div style="color: #cbd5e1; line-height: 1.4;">${escapeHtml(diag.summaryMessage)}</div>` : ''}
    </div>
  `;
}

async function triggerSingleAutoGrab(mediaId, episodeId, label) {
  showToastNotification(`⚡ Ejecutando AutoGrabber de precisión para "${label}"...`, 'info', '🤖');
  try {
    const res = await client.call('media.autoGrab', {
      mediaId,
      episodeId: episodeId || undefined
    });
    const grabbedM = (res && res.grabbedMovies) || [];
    const grabbedE = (res && res.grabbedEpisodes) || [];
    const healed = (res && res.healedDownloads) || [];

    if (grabbedM.length > 0) {
      const g = grabbedM[0];
      showToastNotification(`✅ Auto-Grab: "${g.release}" [${g.quality} · ${g.language} · ${g.score} pts]`, 'success', '🎬');
    } else if (grabbedE.length > 0) {
      const g = grabbedE[0];
      showToastNotification(`✅ Auto-Grab S${String(g.season).padStart(2,'0')}E${String(g.episode).padStart(2,'0')}: "${g.release}"`, 'success', '📺');
    } else if (healed.length > 0) {
      showToastNotification(`🛡️ Auto-Curación: descartada descarga estancada (${healed[0].discardedRelease})`, 'warning', '🛡️');
    } else if (res && res.diagnostic && res.diagnostic.summaryMessage) {
      showToastNotification(res.diagnostic.summaryMessage, 'info', '🔎');
    } else {
      showToastNotification('Búsqueda completada: ninguna release superó los filtros estrictos todavía.', 'info', '🔎');
    }
    await openMediaDetail(mediaId);
    if (typeof loadMediaLibrary === 'function') loadMediaLibrary();
  } catch (e) {
    showToastNotification('Error en AutoGrabber: ' + e.message, 'error', '❌');
  }
}
window.triggerSingleAutoGrab = triggerSingleAutoGrab;

async function clearMediaBlacklist(targetId, mediaId) {
  try {
    const res = await client.call('media.autoGrab', { clearBlacklistId: targetId });
    showToastNotification(`🧹 Lista negra limpiada (${res.clearedCount || 0} hashes restaurados)`, 'success', '✨');
    await openMediaDetail(mediaId);
  } catch (e) {
    showToastNotification('Error limpiando lista negra: ' + e.message, 'error', '❌');
  }
}
window.clearMediaBlacklist = clearMediaBlacklist;

function renderMovieSpecificPanels(item, details, subtitles, safeTitleAttr, autoGrab, mediaProbe = null) {
  const specsGridEl = document.getElementById('movie-tech-specs-grid');
  const fileBoxEl = document.getElementById('movie-file-details-box');

  if (specsGridEl) {
    const directors = (details.directors || []).filter(d => d.department === 'directing').map(d => d.name).join(', ') || 'No especificado';
    const writers = (details.directors || []).filter(d => d.department === 'writing').map(d => d.name).join(', ') || '—';
    const music = (details.directors || []).filter(d => d.department === 'music' || d.department === 'camera').map(d => `${d.name} (${d.role})`).join(', ') || '—';
    const extLinks = [];
    if (details.tmdbId || item.tmdbId) {
      const tid = details.tmdbId || item.tmdbId;
      extLinks.push(`<a href="https://www.themoviedb.org/movie/${tid}" target="_blank" rel="noopener" style="color:#38bdf8; text-decoration:none;">TMDB #${tid} ↗</a>`);
    }
    if (details.imdbId || item.imdbId) {
      const iid = details.imdbId || item.imdbId;
      extLinks.push(`<a href="https://www.imdb.com/title/${escapeHtml(iid)}" target="_blank" rel="noopener" style="color:#facc15; text-decoration:none;">IMDb (${escapeHtml(iid)}) ↗</a>`);
    }

    specsGridEl.innerHTML = `
      <div class="media-kv-item">
        <div class="label">Dirección</div>
        <div class="value" style="color:#fbbf24;">${escapeHtml(directors)}</div>
      </div>
      <div class="media-kv-item">
        <div class="label">Guion</div>
        <div class="value">${escapeHtml(writers)}</div>
      </div>
      <div class="media-kv-item">
        <div class="label">Estudio / Productora</div>
        <div class="value">${escapeHtml(details.studioOrNetwork || '—')}</div>
      </div>
      <div class="media-kv-item">
        <div class="label">Año / Duración</div>
        <div class="value">${item.year || '—'}${details.runtimeMins ? ' • ' + formatRuntimeMinutes(details.runtimeMins) : ''}</div>
      </div>
      <div class="media-kv-item">
        <div class="label">Música / Fotografía</div>
        <div class="value">${escapeHtml(music)}</div>
      </div>
      <div class="media-kv-item">
        <div class="label">Fichas Externas</div>
        <div class="value" style="display:flex; gap:10px; flex-wrap:wrap;">${extLinks.join(' ') || '—'}</div>
      </div>
    `;
  }

  if (fileBoxEl) {
    const subBadges = (subtitles && subtitles.length > 0)
      ? subtitles.map(lang => `<span class="badge badge-dl" style="font-size:0.75rem;">✓ Subtítulo ${escapeHtml(lang.toUpperCase())}</span>`).join(' ')
      : `<span class="badge badge-wait" style="font-size:0.75rem;">⚠️ Sin subtítulos locales (.srt)</span>`;

    const audioLangsList = (mediaProbe && Array.isArray(mediaProbe.audioLanguages) && mediaProbe.audioLanguages.length > 0)
      ? mediaProbe.audioLanguages.join(', ')
      : 'desconocido';
    const audioChip = mediaProbe
      ? (mediaProbe.hasSpanishAudio
        ? `<span class="badge badge-dl" style="font-size:0.75rem;">🔊 Audio: Castellano / ES (${escapeHtml(audioLangsList)})</span>`
        : `<span class="badge" style="background: rgba(248,81,73,0.18); color: #fca5a5; border: 1px solid rgba(248,81,73,0.4); font-size:0.75rem;">⚠️ Audio detectado: ${escapeHtml(audioLangsList)}</span>`)
      : '';
    const wrongLangBanner = (item.filePath && mediaProbe && !mediaProbe.hasSpanishAudio) ? `
      <div style="background: rgba(248, 81, 73, 0.1); border: 1px solid rgba(248, 81, 73, 0.35); border-radius: 8px; padding: 10px 12px; display: flex; justify-content: space-between; align-items: center; gap: 10px; flex-wrap: wrap;">
        <div style="font-size: 0.82rem; color: #fecaca;">
          <strong>⚠️ Idioma no detectado en Castellano de España:</strong> las pistas reales del archivo son <code>${escapeHtml(audioLangsList)}</code>.
        </div>
        <div style="display: flex; gap: 6px; flex-wrap: wrap;">
          <button type="button" class="btn btn-sm btn-primary" onclick="triggerReplaceWrongLanguage('${item.id}', null, null, '${safeTitleAttr}', true)">🇪🇸 Siguiente en Castellano (1 clic)</button>
          <button type="button" class="btn btn-sm" onclick="closeModal('modal-media-detail'); openCastilianReplacementModal('${item.id}', '${safeTitleAttr}', true)">🎯 Elegir otra en Castellano</button>
        </div>
      </div>
    ` : '';

    const fileNameOnly = item.filePath ? item.filePath.split('/').pop() : null;
    fileBoxEl.innerHTML = `
      <div style="display: flex; flex-direction: column; gap: 12px;">
        <div style="display: flex; align-items: center; gap: 10px; flex-wrap: wrap;">
          <span class="media-meta-chip">🎞️ Perfil: <strong>${escapeHtml(item.qualityProfile || '1080p')}</strong></span>
          ${audioChip}
          ${subBadges}
        </div>
        ${wrongLangBanner}
        ${item.filePath ? `
          <div style="background: rgba(0,0,0,0.35); border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; padding: 10px 12px;">
            <div style="font-size: 0.72rem; color: var(--text-muted); margin-bottom: 4px;">ARCHIVO CATALOGADO EN DISCO:</div>
            <div style="font-family: var(--font-mono); font-size: 0.82rem; color: #e2e8f0; word-break: break-all;">🎬 ${escapeHtml(fileNameOnly || item.filePath)}</div>
            <div style="font-family: var(--font-mono); font-size: 0.72rem; color: #64748b; margin-top: 4px; word-break: break-all;">📂 ${escapeHtml(item.filePath)}</div>
          </div>
        ` : `
          <div style="background: rgba(234, 179, 8, 0.08); border: 1px solid rgba(234, 179, 8, 0.25); border-radius: 8px; padding: 12px; font-size: 0.84rem; color: #fde047;">
            🟡 Esta película aún no tiene un archivo completado en la biblioteca. Puedes pulsar <strong>⚡ Auto-Grab Ahora</strong> o <strong>🔍 Buscar Fuentes</strong> para elegir una versión en eD2K o BitTorrent.
          </div>
        `}
        ${renderAutoGrabDiagnosticBanner(item.id, autoGrab)}
      </div>
    `;
  }
}

function initSeriesSeasonsAndEpisodes() {
  const episodes = currentMediaDetailState.episodes || [];
  const tabsEl = document.getElementById('series-season-tabs');
  const filterEl = document.getElementById('series-episode-filter');
  if (filterEl) filterEl.value = 'all';

  const seasonMap = new Map();
  episodes.forEach(ep => {
    const s = ep.seasonNumber || 1;
    if (!seasonMap.has(s)) {
      seasonMap.set(s, { total: 0, completed: 0, monitoredCount: 0 });
    }
    const entry = seasonMap.get(s);
    entry.total += 1;
    if (ep.status === 'completed') entry.completed += 1;
    if (ep.monitored !== false) entry.monitoredCount += 1;
  });

  const seasons = Array.from(seasonMap.keys()).sort((a, b) => a - b);
  if (seasons.length > 0 && !seasons.includes(currentMediaDetailState.selectedSeason)) {
    currentMediaDetailState.selectedSeason = seasons[0];
  }

  if (tabsEl) {
    tabsEl.innerHTML = seasons.map(s => {
      const st = seasonMap.get(s);
      const isComplete = st.total > 0 && st.completed === st.total;
      const isMuted = st.monitoredCount === 0 && !isComplete;
      return `
        <button class="season-tab-btn ${s === currentMediaDetailState.selectedSeason ? 'active' : ''} ${isMuted ? 'is-muted' : ''}"
                id="season-tab-btn-${s}"
                onclick="selectSeriesSeason(${s})">
          <span>${isMuted ? '🔕 ' : ''}Temporada ${s}</span>
          <span class="season-tab-count">${st.completed}/${st.total}${isComplete ? ' ✓' : ''}</span>
        </button>
      `;
    }).join('');
  }

  renderCurrentSeriesSeason();
}

async function triggerSeasonAutoGrab(mediaId, seasonNumber, title) {
  showToastNotification(`⚡ Buscando y descargando Temporada ${seasonNumber} de "${title}"...`, 'info', '📺');
  try {
    const res = await client.call('media.autoGrab', { mediaId, seasonNumber });
    const grabbedE = (res && res.grabbedEpisodes) || [];
    if (grabbedE.length > 0) {
      showToastNotification(`✅ Temporada ${seasonNumber}: ${grabbedE.length} elemento(s) añadidos a la cola`, 'success', '📺');
    } else {
      showToastNotification(`Búsqueda de Temporada ${seasonNumber} completada`, 'info', '🔎');
    }
    await openMediaDetail(mediaId);
    if (typeof loadMediaLibrary === 'function') loadMediaLibrary();
  } catch (e) {
    showToastNotification(`Error descargando Temporada ${seasonNumber}: ${e.message || e}`, 'error', '❌');
  }
}

async function toggleSeasonMonitored(mediaId, seasonNumber, nextMonitored) {
  try {
    await client.call('media.setSeasonMonitored', {
      mediaId,
      seasonNumber,
      monitored: Boolean(nextMonitored),
      onlyThisSeason: false
    });
    showToastNotification(
      nextMonitored
        ? `🔔 Temporada ${seasonNumber} activada para descarga automática`
        : `🔕 Temporada ${seasonNumber} omitida (ya vista / sin descarga automática)`,
      'success',
      nextMonitored ? '🔔' : '👁️'
    );
    await openMediaDetail(mediaId);
  } catch (e) {
    showToastNotification(`Error actualizando temporada: ${e.message || e}`, 'error', '❌');
  }
}

async function setOnlySeasonMonitored(mediaId, seasonNumber) {
  try {
    await client.call('media.setSeasonMonitored', {
      mediaId,
      seasonNumber,
      monitored: true,
      onlyThisSeason: true
    });
    showToastNotification(`🎯 Solo Temporada ${seasonNumber} activa (temporadas anteriores omitidas)`, 'success', '🎯');
    await openMediaDetail(mediaId);
  } catch (e) {
    showToastNotification(`Error aislando temporada: ${e.message || e}`, 'error', '❌');
  }
}

function openSeasonReleaseSearch(mediaId, seriesTitle, seasonNumber) {
  const sTag = `S${String(seasonNumber).padStart(2, '0')}`;
  openReleasesModal(mediaId, `${seriesTitle} ${sTag}`, false);
}

window.triggerSeasonAutoGrab = triggerSeasonAutoGrab;
window.toggleSeasonMonitored = toggleSeasonMonitored;
window.setOnlySeasonMonitored = setOnlySeasonMonitored;
window.openSeasonReleaseSearch = openSeasonReleaseSearch;

function selectSeriesSeason(seasonNumber) {
  currentMediaDetailState.selectedSeason = seasonNumber;
  const buttons = document.querySelectorAll('#series-season-tabs .season-tab-btn');
  buttons.forEach(btn => btn.classList.remove('active'));
  const activeBtn = document.getElementById(`season-tab-btn-${seasonNumber}`);
  if (activeBtn) activeBtn.classList.add('active');
  renderCurrentSeriesSeason();
}
window.selectSeriesSeason = selectSeriesSeason;

function renderCurrentSeriesSeason() {
  const { item, episodes, details, selectedSeason } = currentMediaDetailState;
  const container = document.getElementById('series-episodes-body');
  const summaryEl = document.getElementById('series-season-summary');
  const filterVal = document.getElementById('series-episode-filter')?.value || 'all';
  if (!container || !item) return;

  const epMetaMap = (details && details.episodeMeta) || {};
  const fallbackThumb = (details && (details.backdropUrl || details.posterUrl)) || item.backdropPath || item.posterPath || '';

  let seasonEps = episodes
    .filter(ep => (ep.seasonNumber || 1) === selectedSeason)
    .sort((a, b) => a.episodeNumber - b.episodeNumber);

  const completedInSeason = seasonEps.filter(e => e.status === 'completed').length;
  const monitoredInSeason = seasonEps.filter(e => e.monitored !== false).length;
  const isSeasonMonitored = monitoredInSeason > 0;
  if (summaryEl) {
    summaryEl.textContent = `Temporada ${selectedSeason}: ${completedInSeason} de ${seasonEps.length} completados`;
  }

  const safeSeriesTitle = escapeHtml(item.title).replace(/'/g, "\\'");
  const smartBarEl = document.getElementById('series-season-smart-bar');
  if (smartBarEl) {
    const pendingInSeason = seasonEps.filter(e => e.status !== 'completed').length;
    const replaceableInSeason = seasonEps.some(e => e.status === 'completed' || e.status === 'downloading');
    smartBarEl.innerHTML = `
      <div class="season-smart-status">
        <span class="badge ${isSeasonMonitored ? 'badge-dl' : 'badge-pause'}">
          ${isSeasonMonitored ? `🔔 Siguiendo Temporada ${selectedSeason}` : `👁️ Temporada ${selectedSeason} omitida (ya vista)`}
        </span>
        <span style="font-size: 0.76rem; color: var(--text-muted);">
          ${pendingInSeason > 0 ? `${pendingInSeason} episodio(s) pendientes` : 'Temporada completa en disco'}
        </span>
      </div>
      <div class="season-smart-actions">
        ${pendingInSeason > 0 ? `<button type="button" class="btn btn-sm btn-primary" onclick="triggerSeasonAutoGrab('${item.id}', ${selectedSeason}, '${safeSeriesTitle}')" title="Buscar pack o episodios solo de la Temporada ${selectedSeason}">⚡ Descargar Temporada ${selectedSeason}</button>` : ''}
        ${replaceableInSeason ? `<button type="button" class="btn btn-sm" style="background:rgba(234,179,8,0.14); color:#fde047; border-color:rgba(234,179,8,0.35);" onclick="triggerReplaceWrongLanguage('${item.id}', null, ${selectedSeason}, '${safeSeriesTitle} T.${selectedSeason}', true)" title="Descartar la versión actual de toda la temporada y buscar la siguiente en Castellano de España">🇪🇸 Cambiar T.${selectedSeason} (ES)</button>` : ''}
        <button type="button" class="btn btn-sm" onclick="setOnlySeasonMonitored('${item.id}', ${selectedSeason})" title="Omitir automáticamente el resto de temporadas y seguir solo la Temporada ${selectedSeason}">🎯 Solo seguir T.${selectedSeason}</button>
        <button type="button" class="btn btn-sm" onclick="toggleSeasonMonitored('${item.id}', ${selectedSeason}, ${!isSeasonMonitored})" title="${isSeasonMonitored ? 'Omitir esta temporada para no descargarla automáticamente' : 'Volver a activar la descarga automática de esta temporada'}">
          ${isSeasonMonitored ? `👁️ Omitir T.${selectedSeason} (Ya vista)` : `🔔 Activar T.${selectedSeason}`}
        </button>
        <button type="button" class="btn btn-sm" onclick="closeModal('modal-media-detail'); openSeasonReleaseSearch('${item.id}', '${safeSeriesTitle}', ${selectedSeason})" title="Buscar manualmente pack o episodios de la Temporada ${selectedSeason}">🔍 Pack / Fuentes T.${selectedSeason}</button>
      </div>
    `;
  }

  if (filterVal === 'completed') {
    seasonEps = seasonEps.filter(e => e.status === 'completed');
  } else if (filterVal === 'pending') {
    seasonEps = seasonEps.filter(e => e.status !== 'completed');
  }

  if (seasonEps.length === 0) {
    container.innerHTML = `
      <div style="grid-column: 1 / -1; text-align: center; padding: 36px; color: var(--text-muted); background: rgba(255,255,255,0.02); border-radius: 10px; border: 1px dashed rgba(255,255,255,0.1);">
        No hay capítulos que coincidan con el filtro seleccionado en la Temporada ${selectedSeason}.
      </div>
    `;
    return;
  }

  const episodeProbes = currentMediaDetailState.episodeProbes || {};
  container.innerHTML = seasonEps.map(ep => {
    const epCode = `S${String(ep.seasonNumber).padStart(2, '0')}E${String(ep.episodeNumber).padStart(2, '0')}`;
    const rich = epMetaMap[epCode] || {};
    const epProbe = episodeProbes[ep.id] || null;
    const epAudioLangs = (epProbe && Array.isArray(epProbe.audioLanguages) && epProbe.audioLanguages.length > 0)
      ? epProbe.audioLanguages.join(', ')
      : 'desconocido';
    const wrongLangEpBadge = (ep.status === 'completed' && epProbe && !epProbe.hasSpanishAudio)
      ? `<span class="badge" style="background: rgba(248,81,73,0.18); color: #fca5a5; border: 1px solid rgba(248,81,73,0.4); font-size: 0.7rem; margin-right: 4px;" title="Audio real detectado: ${escapeHtml(epAudioLangs)}">⚠️ Sin audio ES</span>`
      : '';

    let statusBadge = '<span class="badge badge-wait">🟡 Buscado</span>';
    let cardStateClass = 'wanted';
    if (ep.status === 'completed') {
      statusBadge = '<span class="badge badge-dl">🟢 Completado</span>';
      cardStateClass = 'completed';
    } else if (ep.status === 'downloading') {
      statusBadge = '<span class="badge badge-ed2k">⬇ Descargando</span>';
      cardStateClass = 'downloading';
    }

    const thumbUrl = rich.imageUrl || fallbackThumb;
    const runtimeBadge = rich.runtimeMins ? `<span class="episode-runtime-badge">${rich.runtimeMins}m</span>` : '';
    const airDateText = ep.airDate || rich.airDate || '';
    const overviewText = rich.overview || '';
    const cleanFileName = ep.filePath ? ep.filePath.split('/').pop() : '';
    const safeFilePathAttr = escapeHtml(ep.filePath || '').replace(/'/g, "\\'");

    return `
      <div class="episode-card ${cardStateClass}">
        <div class="episode-thumb-wrap">
          ${thumbUrl
            ? `<img src="${escapeHtml(thumbUrl)}" alt="${epCode}" loading="lazy" onerror="this.style.display='none'">`
            : `<div style="width:100%; height:100%; display:flex; align-items:center; justify-content:center; color:#475569; font-size:1.4rem;">📺</div>`
          }
          <span class="episode-code-badge">${epCode}</span>
          ${runtimeBadge}
        </div>

        <div style="flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: space-between; gap: 6px;">
          <div>
            <div style="display: flex; justify-content: space-between; align-items: flex-start; gap: 8px; margin-bottom: 4px;">
              <div style="font-weight: 700; font-size: 0.96rem; color: #f8fafc; line-height: 1.25;">
                ${ep.episodeNumber}. ${escapeHtml(ep.title || rich.title || 'Episodio ' + ep.episodeNumber)}
              </div>
              <div style="flex-shrink: 0;">${wrongLangEpBadge}${statusBadge}</div>
            </div>

            ${airDateText ? `<div style="font-size: 0.74rem; color: #94a3b8; margin-bottom: 4px;">📅 Emisión: ${escapeHtml(airDateText)}</div>` : ''}

            ${overviewText ? `
              <div style="font-size: 0.8rem; color: #cbd5e1; line-height: 1.42; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;" title="${escapeHtml(overviewText)}">
                ${escapeHtml(overviewText)}
              </div>
            ` : ''}
          </div>

          <div style="display: flex; justify-content: space-between; align-items: center; gap: 10px; padding-top: 6px; border-top: 1px solid rgba(255,255,255,0.06); margin-top: 2px;">
            <div style="min-width: 0; flex: 1; font-family: var(--font-mono); font-size: 0.73rem; color: #94a3b8; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;" title="${escapeHtml(cleanFileName || ((currentMediaDetailState.autoGrab && currentMediaDetailState.autoGrab.episodeDiagnostics && currentMediaDetailState.autoGrab.episodeDiagnostics[ep.id] && currentMediaDetailState.autoGrab.episodeDiagnostics[ep.id].summaryMessage) || 'Sin archivo en disco'))}">
              ${cleanFileName ? `📂 ${escapeHtml(cleanFileName)}` : `<span style="color:#64748b;">${escapeHtml((currentMediaDetailState.autoGrab && currentMediaDetailState.autoGrab.episodeDiagnostics && currentMediaDetailState.autoGrab.episodeDiagnostics[ep.id] && currentMediaDetailState.autoGrab.episodeDiagnostics[ep.id].summaryMessage) || 'Pendiente de descarga')}</span>`}
            </div>

            <div style="display: flex; gap: 6px; flex-shrink: 0; align-items: center;">
              ${ep.status === 'completed' ? `<span class="badge badge-completed" style="font-size: 0.72rem; padding: 2px 7px;">✔ En disco</span>` : `<button class="btn btn-sm" style="background:rgba(56,189,248,0.16); color:#38bdf8; border-color:rgba(56,189,248,0.35);" onclick="triggerSingleAutoGrab('${item.id}', '${ep.id}', '${safeSeriesTitle} ${epCode}')" title="Auto-Grab automático para ${epCode}">⚡ Auto</button>`}
              ${(ep.status === 'completed' || ep.status === 'downloading') ? `<button class="btn btn-sm" style="background:rgba(234,179,8,0.14); color:#fde047; border-color:rgba(234,179,8,0.35);" onclick="closeModal('modal-media-detail'); openCastilianReplacementModal('${item.id}', '${safeSeriesTitle} ${epCode}', false, '${ep.id}', ${ep.seasonNumber}, ${ep.episodeNumber})" title="Idioma incorrecto: buscar siguiente recomendación en Castellano de España">🇪🇸</button>` : ''}
              <button class="btn btn-sm" onclick="openEpisodeReleaseSearch('${item.id}', '${safeSeriesTitle}', ${ep.seasonNumber}, ${ep.episodeNumber}, '${ep.id}')" title="Buscar fuentes para ${epCode}">🔍 Fuentes</button>
              ${ep.status === 'completed' && ep.filePath ? `<button class="btn btn-sm" onclick="openDestinationFolder({ path: '${safeFilePathAttr}', title: '${safeSeriesTitle} ${epCode}' })" title="Abrir carpeta de destino">📂</button>` : ''}
              ${ep.status === 'completed' ? `<button class="btn btn-sm" onclick="openSubtitlesModal('${item.id}', '${safeSeriesTitle} ${epCode}', false, ${ep.seasonNumber}, ${ep.episodeNumber}, '${safeFilePathAttr}')" title="Subtítulos (Bazarr)">💬 Subs</button>` : ''}
            </div>
          </div>
        </div>
      </div>
    `;
  }).join('');
}
window.renderCurrentSeriesSeason = renderCurrentSeriesSeason;

function openEpisodeReleaseSearch(mediaId, seriesTitle, season, episode, episodeId = null) {
  const epCode = `S${String(season).padStart(2, '0')}E${String(episode).padStart(2, '0')}`;
  openReleasesModal(mediaId, `${seriesTitle} ${epCode}`, false, {
    episodeId,
    seasonNumber: season,
    episodeNumber: episode
  });
}

// Modal Indexadores
async function openIndexersModal() {
  const m = document.getElementById('modal-indexers');
  if (m) m.classList.add('active');
  await loadIndexersList();
}

async function loadIndexersList() {
  const listEl = document.getElementById('indexers-list');
  if (!listEl) return;
  try {
    const indexers = await client.call('indexer.list', {});
    if (!indexers || indexers.length === 0) {
      listEl.innerHTML = `<div style="text-align:center; padding: 20px; color: var(--text-muted);">No hay indexadores Torznab configurados. (Se usa Kademlia DHT por defecto).</div>`;
      return;
    }

    listEl.innerHTML = indexers.map(idx => `
      <div style="background: rgba(255,255,255,0.02); border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; display: flex; justify-content: space-between; align-items: center;">
        <div>
          <div style="font-weight: 700; font-size: 0.9rem;">${escapeHtml(idx.name)} ${idx.enabled ? '🟢' : '⚪'}</div>
          <div style="font-size: 0.75rem; color: var(--text-muted); font-family: var(--font-mono);">${escapeHtml(idx.baseUrl)}</div>
        </div>
        <button class="btn btn-sm" style="color: #f85149;" onclick="deleteIndexer('${idx.id}', '${escapeHtml(idx.name)}')">🗑</button>
      </div>
    `).join('');
  } catch (e) {
    listEl.innerHTML = `<div style="color: #f85149;">Error cargando indexadores: ${escapeHtml(e.message)}</div>`;
  }
}

async function addExternalIndexer() {
  const name = document.getElementById('indexer-name')?.value.trim();
  const baseUrl = document.getElementById('indexer-url')?.value.trim();
  const apiKey = document.getElementById('indexer-key')?.value.trim();
  if (!name || !baseUrl) return;

  try {
    await client.call('indexer.add', { name, baseUrl, apiKey, enabled: true });
    showToastNotification(`Indexador "${name}" añadido`, 'success', '🌐');
    document.getElementById('indexer-name').value = '';
    document.getElementById('indexer-url').value = '';
    document.getElementById('indexer-key').value = '';
    await loadIndexersList();
  } catch (e) {
    showToastNotification('Error añadiendo indexador: ' + e.message, 'error', '❌');
  }
}

async function deleteIndexer(id, name) {
  if (!confirm(`¿Eliminar indexador "${name}"?`)) return;
  try {
    await client.call('indexer.delete', { id });
    showToastNotification(`Indexador "${name}" eliminado`, 'info', '🗑');
    await loadIndexersList();
  } catch (e) {
    showToastNotification('Error eliminando indexador: ' + e.message, 'error', '❌');
  }
}

function openModal(modalId) {
  const m = document.getElementById(modalId);
  if (m) m.classList.add('active');
}
window.openModal = openModal;

function closeModal(modalId) {
  const m = document.getElementById(modalId);
  if (m) m.classList.remove('active');
}
window.closeModal = closeModal;

// ==========================================
// GESTIÓN DE SUBTÍTULOS (BAZARR NATIVO)
// ==========================================
let currentSubtitleMediaId = null;
let currentSubtitleContext = null;

async function openSubtitlesModal(mediaId, title, isMovie, season, episode, filePath) {
  currentSubtitleMediaId = mediaId;
  currentSubtitleContext = { mediaId, title, isMovie, season, episode, filePath };

  const m = document.getElementById('modal-subtitles');
  const titleEl = document.getElementById('subtitles-modal-title');
  const badgesEl = document.getElementById('subtitles-detected-badges');
  const tableEl = document.getElementById('subtitles-table');
  const tbody = document.getElementById('subtitles-table-body');
  const msgEl = document.getElementById('subtitles-loading-msg');

  if (titleEl) titleEl.textContent = `Subtítulos: ${title}`;
  if (badgesEl) badgesEl.innerHTML = '<span style="color:var(--text-muted);">Comprobando disco...</span>';
  if (tableEl) tableEl.style.display = 'none';
  if (tbody) tbody.innerHTML = '';
  if (msgEl) {
    msgEl.style.display = 'block';
    msgEl.textContent = 'Pulsa "Buscar" para consultar subtítulos en OpenSubtitles.';
  }
  if (m) m.classList.add('active');

  // Check existing companion subtitles and render quick sync controls
  try {
    const check = await client.call('subtitles.check', { mediaId });
    const syncBar = `
      <span style="display:inline-flex; gap:4px; margin-left:8px; vertical-align:middle;">
        <button class="btn btn-sm btn-secondary" onclick="syncCurrentSubtitle('auto')" title="Alinear contra pista embebida">⚡ Auto-sync</button>
        <button class="btn btn-sm btn-secondary" onclick="syncCurrentSubtitle('manual', -500)">-500ms</button>
        <button class="btn btn-sm btn-secondary" onclick="syncCurrentSubtitle('manual', 500)">+500ms</button>
      </span>
    `;
    if (check && check.subtitles && check.subtitles.length > 0) {
      badgesEl.innerHTML = check.subtitles.map(l => `<span class="badge badge-dl" style="font-size:0.75rem; margin-right:4px;">✓ ${l.toUpperCase()} detectado</span>`).join('') + syncBar;
    } else {
      badgesEl.innerHTML = '<span class="badge badge-wait" style="font-size:0.75rem;">⚠️ Sin subtítulos locales</span>';
    }
  } catch (e) {
    badgesEl.innerHTML = '<span style="color:var(--text-muted);">-</span>';
  }

  // Auto-trigger search
  await searchSubtitlesForCurrentMedia();
}

async function searchSubtitlesForCurrentMedia() {
  if (!currentSubtitleContext) return;
  const lang = document.getElementById('subtitles-lang-select')?.value || 'spa';
  const msgEl = document.getElementById('subtitles-loading-msg');
  const tableEl = document.getElementById('subtitles-table');
  const tbody = document.getElementById('subtitles-table-body');

  if (msgEl) {
    msgEl.innerHTML = `<div style="font-size: 1.5rem; margin-bottom:8px;">⏳</div>Buscando en OpenSubtitles (${lang.toUpperCase()})...`;
    msgEl.style.display = 'block';
  }
  if (tableEl) tableEl.style.display = 'none';

  try {
    const params = {
      mediaId: currentSubtitleContext.mediaId,
      lang: lang
    };
    if (currentSubtitleContext.season && currentSubtitleContext.episode) {
      params.season = currentSubtitleContext.season;
      params.episode = currentSubtitleContext.episode;
      params.isMovie = false;
    }

    const candidates = await client.call('subtitles.search', params);
    if (msgEl) msgEl.style.display = 'none';

    if (!candidates || candidates.length === 0) {
      if (tbody) tbody.innerHTML = `<tr><td colspan="5" style="text-align:center; padding: 30px; color: var(--text-muted);">No se encontraron subtítulos en OpenSubtitles para este título en ${lang.toUpperCase()}.</td></tr>`;
      if (tableEl) tableEl.style.display = 'table';
      return;
    }

    window.__currentSubCandidates = candidates;
    if (tbody) {
      tbody.innerHTML = candidates.map((c, idx) => {
        const isHash = c.matchedBy === 'moviehash';
        const matchBadge = isHash
          ? '<span class="badge badge-dl" style="font-weight:700;">⚡ Hash exacto (100% sync)</span>'
          : `<span class="badge" style="background:rgba(255,255,255,0.06); font-size:0.75rem;">${escapeHtml(c.matchedBy || 'título')}</span>`;

        const scoreBadge = `<span class="badge badge-dl" style="font-size:0.7rem; margin-left:4px;">${c.score ?? 0} pts</span>`;
        const fpsBadge = c.fpsMilli ? `<span class="badge" style="background:rgba(56,189,248,0.12); color:#38bdf8; font-size:0.7rem; margin-left:4px;">${(c.fpsMilli / 1000).toFixed(3)} fps</span>` : '';
        const variantBadge = c.variant ? `<span class="badge" style="background:rgba(34,197,94,0.12); color:#22c55e; font-size:0.7rem; margin-left:4px;">${escapeHtml(c.variant)}</span>` : '';
        const hiBadge = c.isHi ? '<span class="badge" style="background:rgba(234,179,8,0.15); color:#eab308; font-size:0.7rem; margin-left:4px;">HI</span>' : '';
        const autoBadge = c.isAutoTranslated ? '<span class="badge" style="background:rgba(248,81,73,0.15); color:#f85149; font-size:0.7rem; margin-left:4px;">Auto-trad</span>' : '';

        return `
          <tr class="release-table-row">
            <td>
              <div style="font-weight:600; font-size:0.84rem; word-break:break-all;">${escapeHtml(c.title)}</div>
              <div style="font-size:0.72rem; color:var(--text-muted);">Calificación: ${c.rating || '0.0'}★ ${scoreBadge}${variantBadge}${fpsBadge}${hiBadge}${autoBadge}</div>
            </td>
            <td><span class="badge" style="font-size:0.75rem;">${escapeHtml(c.language.toUpperCase())}</span></td>
            <td>${matchBadge}</td>
            <td style="font-family:var(--font-mono); font-size:0.8rem;">${c.downloads || 0}</td>
            <td style="display:flex; gap:4px;">
              <button class="btn btn-sm btn-primary" onclick="downloadSubtitleCandidate(${idx})">
                ⬇ Guardar
              </button>
              <button class="btn btn-sm btn-secondary" onclick="blacklistSubtitleCandidate(${idx})" title="Marcar como erróneo y ocultar">
                ✕
              </button>
            </td>
          </tr>
        `;
      }).join('');
    }
    if (tableEl) tableEl.style.display = 'table';
  } catch (e) {
    if (msgEl) msgEl.innerHTML = `<div style="color:#f85149; text-align:center; padding:40px;">Error buscando subtítulos: ${escapeHtml(e.message)}</div>`;
  }
}

async function downloadSubtitleCandidate(idx) {
  const candidate = (window.__currentSubCandidates || [])[idx];
  if (!candidate || !currentSubtitleMediaId) return;

  try {
    const payload = {
      mediaId: currentSubtitleMediaId,
      downloadLink: candidate.download_link || candidate.downloadLink,
      lang: candidate.language || 'es',
      fpsMilli: candidate.fpsMilli || null,
      matchedBy: candidate.matchedBy || 'query'
    };
    if (currentSubtitleContext && currentSubtitleContext.season && currentSubtitleContext.episode) {
      payload.season = currentSubtitleContext.season;
      payload.episode = currentSubtitleContext.episode;
    }
    await client.call('subtitles.download', payload);
    showToastNotification(`Subtítulo guardado: ${candidate.title}`, 'success', '💬');
    addLog('success', `Subtítulo descargado y sincronizado: ${candidate.title}`);
    closeModal('modal-subtitles');
    await loadMediaLibrary();
  } catch (e) {
    showToastNotification('Error descargando subtítulo: ' + e.message, 'error', '❌');
  }
}

async function syncCurrentSubtitle(mode, offsetMs = 0) {
  if (!currentSubtitleMediaId) return;
  try {
    const payload = { mediaId: currentSubtitleMediaId, mode, lang: 'es' };
    if (mode === 'manual') payload.offsetMs = offsetMs;
    if (currentSubtitleContext && currentSubtitleContext.season && currentSubtitleContext.episode) {
      payload.season = currentSubtitleContext.season;
      payload.episode = currentSubtitleContext.episode;
    }
    const res = await client.call('subtitles.sync', payload);
    const delta = res.appliedOffsetMs || 0;
    showToastNotification(`Sincronización aplicada (${delta >= 0 ? '+' : ''}${delta} ms)`, 'success', '⚡');
  } catch (e) {
    showToastNotification('Error sincronizando subtítulo: ' + e.message, 'error', '❌');
  }
}

async function blacklistSubtitleCandidate(idx) {
  const candidate = (window.__currentSubCandidates || [])[idx];
  if (!candidate || !currentSubtitleMediaId) return;
  try {
    const payload = {
      mediaId: currentSubtitleMediaId,
      provider: candidate.provider || 'opensubtitles',
      providerSubId: candidate.id,
      reason: 'rejected_by_user',
      lang: candidate.language || 'spa'
    };
    if (currentSubtitleContext && currentSubtitleContext.season && currentSubtitleContext.episode) {
      payload.season = currentSubtitleContext.season;
      payload.episode = currentSubtitleContext.episode;
    }
    await client.call('subtitles.blacklistAndRetry', payload);
    showToastNotification('Subtítulo descartado; buscando alternativa...', 'info', '🔄');
    await searchSubtitlesForCurrentMedia();
  } catch (e) {
    showToastNotification('Error descartando subtítulo: ' + e.message, 'error', '❌');
  }
}

async function triggerAutoSubtitles() {
  const btn = document.getElementById('btn-media-autosubs');
  const origHtml = btn ? btn.innerHTML : '';
  if (btn) { btn.disabled = true; btn.innerHTML = '<span>⏳</span> <span>Buscando subs...</span>'; }

  startMediaOperation('Buscando subtítulos (Bazarr)...', 'Conectando con OpenSubtitles para pistas en español de vídeos sin subs...', '💬');
  updateMediaOperation(35, 'Comprobando películas y capítulos de series completados...');

  try {
    const res = await client.call('subtitles.autoFetch', { lang: 'spa' });
    updateMediaOperation(85, 'Descargando y asociando pistas .srt al directorio de medios...');
    const list = res.downloaded || [];
    const msg = `${list.length} nuevos subtítulos descargados y sincronizados`;
    finishMediaOperation(true, 'Auto-Subtítulos finalizado', msg, '💬');
    addLog('success', `Auto-Subtítulos: ${msg}`);
    await loadMediaLibrary();
  } catch (e) {
    finishMediaOperation(false, 'Error en Auto-Subtítulos', e.message, '❌');
    addLog('error', `Error en Auto-Subtítulos: ${e.message}`);
  } finally {
    if (btn) { btn.disabled = false; btn.innerHTML = origHtml; }
  }
}

