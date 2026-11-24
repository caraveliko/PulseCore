// Pulse Core WebUI — Núcleo de la biblioteca multimedia Cine y Series (Servarr nativo).
// Extraído de web/index.html.

// ==========================================
// GESTIÓN DE CINE & SERIES (SERVARR NATIVO)
// ==========================================
let mediaLibraryItems = [];
let currentMediaTypeFilter = 'all';
let currentMediaStatusFilter = 'all';
let mediaLibraryLoaded = false;
let mediaLibraryLoading = false;
let mediaLibraryReqSeq = 0;
let mediaLibraryRetryTimer = null;

async function loadMediaLibrary() {
  const reqSeq = ++mediaLibraryReqSeq;
  mediaLibraryLoading = true;
  if (mediaLibraryRetryTimer) {
    clearTimeout(mediaLibraryRetryTimer);
    mediaLibraryRetryTimer = null;
  }
  try {
    const res = await client.call('media.list', {});
    if (reqSeq !== mediaLibraryReqSeq) return;
    mediaLibraryItems = Array.isArray(res) ? res : (res && res.items ? res.items : []);
    mediaLibraryLoaded = true;
    updateMediaCounters();
    updateMediaDiskSpace();
    if (currentMediaSubView === 'calendar') {
      loadCalendarView();
    } else {
      renderMediaCards();
    }
  } catch (err) {
    console.error('Error cargando biblioteca:', err);
    if (!mediaLibraryLoaded && client.ws && client.ws.readyState === WebSocket.OPEN && !mediaLibraryRetryTimer) {
      mediaLibraryRetryTimer = setTimeout(() => {
        mediaLibraryRetryTimer = null;
        loadMediaLibrary();
      }, 1500);
    }
  } finally {
    if (reqSeq === mediaLibraryReqSeq) {
      mediaLibraryLoading = false;
    }
  }
}

function updateMediaCounters() {
  const total = mediaLibraryItems.length;
  const movies = mediaLibraryItems.filter(i => i.mediaType === 'movie').length;
  const tv = mediaLibraryItems.filter(i => i.mediaType === 'tv').length;
  const completed = mediaLibraryItems.filter(i => i.status === 'completed').length;
  const downloading = mediaLibraryItems.filter(i => i.status === 'downloading').length;
  const wanted = mediaLibraryItems.filter(i => i.status === 'wanted').length;

  const setEl = (id, val) => { const el = document.getElementById(id); if (el) el.textContent = `(${val})`; };
  const setBadge = (id, val) => { const el = document.getElementById(id); if (el) el.textContent = String(val); };
  setBadge('count-media-all', total);
  setBadge('count-media-movie', movies);
  setBadge('count-media-tv', tv);
  setEl('count-media-completed', completed);
  setEl('count-media-downloading', downloading);
  setEl('count-media-wanted', wanted);
}

function filterMediaByType(type) {
  currentMediaTypeFilter = type;
  document.querySelectorAll('#media-type-filters button').forEach(b => {
    const active = b.dataset.mtype === type;
    b.classList.toggle('active', active);
    b.setAttribute('aria-pressed', active ? 'true' : 'false');
  });
  renderMediaCards();
}

function filterMediaByStatus(status) {
  currentMediaStatusFilter = status;
  document.querySelectorAll('#media-status-filters button').forEach(b => {
    b.classList.toggle('active', b.dataset.mstatus === status);
  });
  renderMediaCards();
}

function filterMediaCards() {
  renderMediaCards();
}

// Sub-Vistas de Filmoteca (Cartelera vs Cola vs Calendario)
let currentMediaSubView = 'library';
let calendarDate = new Date();
let liveDownloadsStripCollapsed = false;

function switchMediaSubView(view) {
  currentMediaSubView = view;
  document.getElementById('btn-subview-library')?.classList.toggle('active', view === 'library');
  document.getElementById('btn-subview-queue')?.classList.toggle('active', view === 'queue');
  document.getElementById('btn-subview-calendar')?.classList.toggle('active', view === 'calendar');

  const libEl = document.getElementById('media-library-view-wrap');
  const queueEl = document.getElementById('tab-downloads');
  const calEl = document.getElementById('media-calendar-view-wrap');
  if (libEl) libEl.style.display = view === 'library' ? 'block' : 'none';
  if (queueEl) queueEl.style.display = view === 'queue' ? 'flex' : 'none';
  if (calEl) calEl.style.display = view === 'calendar' ? 'block' : 'none';

  if (view !== 'library' && typeof window.clearMediaAmbientBackdrop === 'function') {
    window.clearMediaAmbientBackdrop();
  }

  if (view === 'calendar') {
    loadCalendarView();
  } else if (view === 'queue') {
    renderDownloadsTable();
  } else if (!mediaLibraryLoaded) {
    loadMediaLibrary();
  } else {
    renderMediaCards();
  }
}

window.openDownloadInQueue = function(id) {
  const tabBtn = document.querySelector('.nav-btn[data-tab="tab-media"]');
  if (tabBtn && !tabBtn.classList.contains('active')) {
    tabBtn.click();
  }
  switchMediaSubView('queue');
  if (id) {
    setTimeout(() => {
      if (typeof selectDownload === 'function') selectDownload(id);
    }, 80);
  }
};

window.toggleLiveDownloadsStrip = function() {
  liveDownloadsStripCollapsed = !liveDownloadsStripCollapsed;
  const strip = document.getElementById('media-live-downloads-strip');
  const btn = document.getElementById('btn-toggle-live-strip');
  if (strip) strip.classList.toggle('collapsed', liveDownloadsStripCollapsed);
  if (btn) btn.textContent = liveDownloadsStripCollapsed ? '▸' : '▾';
};

function renderLiveDownloadsStrip(allDownloads) {
  const list = Array.isArray(allDownloads) ? allDownloads : [];
  const activeOrPending = list.filter(item => {
    const dl = item.snapshot || item;
    const st = (dl.status || '').toLowerCase();
    return st === 'downloading' || st === 'verifying' || st === 'completing' || st === 'waiting' || st === 'queued' || st === 'paused' || Boolean(dl.completionStage || dl.completion_stage);
  });
  const downloadingNow = activeOrPending.filter(item => {
    const dl = item.snapshot || item;
    const st = (dl.status || '').toLowerCase();
    return st === 'downloading' || st === 'verifying' || st === 'completing' || Boolean(dl.completionStage || dl.completion_stage);
  });

  const queueCountEl = document.getElementById('subview-queue-count');
  if (queueCountEl) queueCountEl.textContent = String(list.length);

  const navBadge = document.getElementById('nav-media-active-badge');
  if (navBadge) {
    if (downloadingNow.length > 0) {
      navBadge.textContent = String(downloadingNow.length);
      navBadge.style.display = 'inline-flex';
    } else {
      navBadge.style.display = 'none';
    }
  }

  const strip = document.getElementById('media-live-downloads-strip');
  const grid = document.getElementById('media-live-downloads-grid');
  const summary = document.getElementById('live-dl-strip-summary');
  if (!strip || !grid) return;

  if (activeOrPending.length === 0) {
    strip.style.display = 'none';
    grid.innerHTML = '';
    return;
  }

  strip.style.display = 'block';
  const totalRate = downloadingNow.reduce((acc, item) => {
    const dl = item.snapshot || item;
    return acc + (dl.downloadRateBps || dl.download_rate_bps || 0);
  }, 0);

  if (summary) {
    summary.textContent = totalRate > 0
      ? `${activeOrPending.length} en curso · ${formatSpeed(totalRate)}`
      : `${activeOrPending.length} en cola`;
  }

  // Priorizar las que descargan ahora
  const sorted = [...activeOrPending].sort((a, b) => {
    const da = a.snapshot || a;
    const db = b.snapshot || b;
    return (db.downloadRateBps || db.download_rate_bps || 0) - (da.downloadRateBps || da.download_rate_bps || 0);
  });

  const topItems = sorted.slice(0, 6);
  const nextIds = topItems.map(item => String((item.snapshot || item).id || item.id || ''));
  const existingCards = Array.from(grid.querySelectorAll('.live-dl-card'));
  const canPatchInPlace = existingCards.length === nextIds.length &&
    existingCards.every((card, idx) => (card.dataset.downloadId || '') === nextIds[idx]);

  if (canPatchInPlace) {
    topItems.forEach((item, idx) => {
      const dl = item.snapshot || item;
      const card = existingCards[idx];
      const rawName = dl.fileName || dl.file_name || 'Archivo';
      const total = dl.totalBytes || dl.total_bytes || 0;
      const received = dl.receivedBytes || dl.received_bytes || 0;
      const pct = total > 0 ? Math.min(100, (received / total) * 100) : (dl.progress ? Math.min(100, dl.progress * 100) : 0);
      const speed = dl.downloadRateBps || dl.download_rate_bps || 0;
      const sources = dl.sourceCount || dl.source_count || 0;
      const st = (dl.status || '').toLowerCase();
      const stageMeta = typeof getDownloadCompletionStageMeta === 'function' ? getDownloadCompletionStageMeta(dl) : { active: false };
      const leftStatusText = stageMeta.active
        ? stageMeta.pipelineTitle.replace(/^Paso \d\/\d · /, '')
        : (speed > 0 ? formatSpeed(speed) : (st === 'paused' ? 'En pausa' : 'Esperando fuentes'));

      card.classList.toggle('is-paused', st === 'paused');
      const pctEl = card.querySelector('.live-dl-pct');
      if (pctEl) pctEl.textContent = `${pct.toFixed(1)}%`;
      const barFill = card.querySelector('.live-dl-bar-fill');
      if (barFill) {
        barFill.style.width = `${pct.toFixed(1)}%`;
        barFill.classList.toggle('completing', !!(stageMeta.active || st === 'completing'));
        barFill.classList.toggle('paused', st === 'paused' && !stageMeta.active);
      }
      const statusEl = card.querySelector('.live-dl-status-text');
      if (statusEl) statusEl.textContent = leftStatusText;
      const sourcesEl = card.querySelector('.live-dl-sources-text');
      if (sourcesEl) sourcesEl.textContent = `${sources} ${sources === 1 ? 'fuente' : 'fuentes'}`;
    });
    return;
  }

  grid.innerHTML = topItems.map(item => {
    const dl = item.snapshot || item;
    const id = dl.id || item.id || '';
    const rawName = dl.fileName || dl.file_name || 'Archivo';
    const displayName = fixMojibake(rawName.replace(/^⚡ \[Torrent\]\s*/i, '').replace(/^\[Torrent\]\s*/i, ''));
    const total = dl.totalBytes || dl.total_bytes || 0;
    const received = dl.receivedBytes || dl.received_bytes || 0;
    const pct = total > 0 ? Math.min(100, (received / total) * 100) : (dl.progress ? Math.min(100, dl.progress * 100) : 0);
    const speed = dl.downloadRateBps || dl.download_rate_bps || 0;
    const sources = dl.sourceCount || dl.source_count || 0;
    const st = (dl.status || '').toLowerCase();
    const stageMeta = typeof getDownloadCompletionStageMeta === 'function' ? getDownloadCompletionStageMeta(dl) : { active: false };
    const isTorrent = dl.protocol === 'torrent' || item.protocol === 'torrent' || rawName.startsWith('⚡ [Torrent]');
    const protoTag = isTorrent ? 'BT' : 'eD2k';
    const leftStatusText = stageMeta.active
      ? stageMeta.pipelineTitle.replace(/^Paso \d\/\d · /, '')
      : (speed > 0 ? formatSpeed(speed) : (st === 'paused' ? 'En pausa' : 'Esperando fuentes'));

    return `
      <button type="button" class="live-dl-card ${st === 'paused' ? 'is-paused' : ''}" data-media-action="open-download" data-download-id="${escapeHtml(id)}" title="${escapeHtml(displayName)}">
        <div class="live-dl-card-top">
          <span class="live-dl-proto ${isTorrent ? 'is-bt' : 'is-ed2k'}">${protoTag}</span>
          <span class="live-dl-name">${escapeHtml(displayName)}</span>
          <span class="live-dl-pct tabular-nums">${pct.toFixed(1)}%</span>
        </div>
        <div class="live-dl-bar"><div class="live-dl-bar-fill ${stageMeta.active || st === 'completing' ? 'completing' : (st === 'paused' ? 'paused' : '')}" style="width: ${pct.toFixed(1)}%"></div></div>
        <div class="live-dl-card-meta tabular-nums">
          <span class="live-dl-status-text">${escapeHtml(leftStatusText)}</span>
          <span class="live-dl-sources-text">${sources} ${sources === 1 ? 'fuente' : 'fuentes'}</span>
        </div>
      </button>
    `;
  }).join('');
}

function changeCalendarMonth(delta) {
  calendarDate.setMonth(calendarDate.getMonth() + delta);
  loadCalendarView();
}

function resetCalendarToCurrentMonth() {
  calendarDate = new Date();
  loadCalendarView();
}

const MONTH_NAMES_ES = [
  'Enero', 'Febrero', 'Marzo', 'Abril', 'Mayo', 'Junio',
  'Julio', 'Agosto', 'Septiembre', 'Octubre', 'Noviembre', 'Diciembre'
];
const DAY_NAMES_ES = [
  'Domingo', 'Lunes', 'Martes', 'Miércoles', 'Jueves', 'Viernes', 'Sábado'
];

async function loadCalendarView() {
  const year = calendarDate.getFullYear();
  const month = calendarDate.getMonth();
  const monthLabelEl = document.getElementById('calendar-current-month-label');
  if (monthLabelEl) {
    monthLabelEl.textContent = `${MONTH_NAMES_ES[month]} ${year}`;
  }

  const container = document.getElementById('calendar-events-container');
  const emptyMsg = document.getElementById('calendar-empty-msg');
  if (!container) return;

  container.innerHTML = `
    <div class="skeleton-stack">
      <div class="skeleton-row"><div class="skeleton-thumb"></div><div class="skeleton-lines"><div class="skeleton-line w-60"></div><div class="skeleton-line w-40"></div></div></div>
      <div class="skeleton-row"><div class="skeleton-thumb"></div><div class="skeleton-lines"><div class="skeleton-line w-80"></div><div class="skeleton-line w-40"></div></div></div>
      <div class="skeleton-row"><div class="skeleton-thumb"></div><div class="skeleton-lines"><div class="skeleton-line w-60"></div><div class="skeleton-line w-40"></div></div></div>
      <div class="skeleton-caption">Cargando emisiones para ${MONTH_NAMES_ES[month]}...</div>
    </div>
  `;

  const endDay = new Date(year, month + 1, 0);
  const pad = (n) => String(n).padStart(2, '0');
  const startDateStr = `${year}-${pad(month + 1)}-01`;
  const endDateStr = `${year}-${pad(month + 1)}-${pad(endDay.getDate())}`;

  const todayObj = new Date();
  const todayStr = `${todayObj.getFullYear()}-${pad(todayObj.getMonth() + 1)}-${pad(todayObj.getDate())}`;

  try {
    const events = await client.call('media.calendar', { startDate: startDateStr, endDate: endDateStr });
    const list = Array.isArray(events) ? events : [];

    if (list.length === 0) {
      container.innerHTML = '';
      if (emptyMsg) emptyMsg.style.display = 'block';
      return;
    }
    if (emptyMsg) emptyMsg.style.display = 'none';

    // Agrupar por airDate
    const groups = {};
    for (const item of list) {
      const d = item.episode.airDate || 'Sin fecha';
      if (!groups[d]) groups[d] = [];
      groups[d].push(item);
    }

    const sortedDates = Object.keys(groups).sort();

    container.innerHTML = sortedDates.map(dateKey => {
      const parts = dateKey.split('-');
      let dayHeader = dateKey;
      let isToday = false;
      let isPast = false;

      if (parts.length === 3) {
        const dObj = new Date(parseInt(parts[0]), parseInt(parts[1]) - 1, parseInt(parts[2]));
        const dayName = DAY_NAMES_ES[dObj.getDay()] || '';
        isToday = dateKey === todayStr;
        isPast = dateKey < todayStr;
        const tag = isToday ? '<span class="badge badge-dl" style="margin-left:8px;">★ HOY</span>' : (isPast ? '<span class="badge badge-wait" style="margin-left:8px; opacity:0.8;">Emitido</span>' : '<span class="badge badge-ed2k" style="margin-left:8px;">Próximo estreno</span>');
        dayHeader = `${dayName}, ${parseInt(parts[2])} de ${MONTH_NAMES_ES[parseInt(parts[1]) - 1]} ${tag}`;
      }

      const dayBorder = isToday ? 'border: 1px solid var(--accent); box-shadow: 0 0 12px rgba(245, 158, 11, 0.2);' : 'border: 1px solid var(--border);';

      const episodesHtml = groups[dateKey].map(calItem => {
        const ep = calItem.episode;
        const epCode = `S${String(ep.seasonNumber).padStart(2, '0')}E${String(ep.episodeNumber).padStart(2, '0')}`;
        const posterSrc = calItem.seriesPosterPath || '';
        const posterHtml = posterSrc 
          ? `<img src="${posterSrc}" style="width: 44px; height: 64px; object-fit: cover; border-radius: 4px; border: 1px solid var(--border);">`
          : `<div style="width: 44px; height: 64px; background: #1e293b; border-radius: 4px; display:flex; align-items:center; justify-content:center; font-size:1.2rem;">📺</div>`;

        let badgeHtml = '<span class="badge badge-wait">⚪ Próximamente</span>';
        if (ep.status === 'completed') {
          badgeHtml = '<span class="badge badge-dl">🟢 En Disco</span>';
        } else if (ep.status === 'downloading') {
          badgeHtml = '<span class="badge badge-ed2k">⬇ Descargando</span>';
        } else if (isPast || isToday) {
          badgeHtml = '<span class="badge badge-wait">⚡ Emitido (Buscando)</span>';
        }

        return `
          <div style="display: flex; align-items: center; gap: 12px; background: rgba(255,255,255,0.02); padding: 8px 12px; border-radius: 6px; border: 1px solid rgba(255,255,255,0.05);">
            ${posterHtml}
            <div style="flex: 1; min-width: 0;">
              <div style="font-weight: 700; font-size: 0.92rem; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">
                ${escapeHtml(calItem.seriesTitle)} <span style="color: var(--accent); font-family: var(--font-mono); font-weight: 600;">${epCode}</span>
              </div>
              <div style="font-size: 0.8rem; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">
                ${escapeHtml(ep.title)}
              </div>
            </div>
            <div style="display: flex; align-items: center; gap: 8px; flex-shrink: 0;">
              ${badgeHtml}
              <button class="btn btn-sm" data-media-action="episode-releases" data-media-id="${escapeHtml(ep.mediaId)}" data-title="${escapeHtml(calItem.seriesTitle)}" data-season="${ep.seasonNumber}" data-episode="${ep.episodeNumber}" title="Buscar fuentes">🔍 Fuentes</button>
              ${ep.status === 'completed' ? `<button class="btn btn-sm" data-media-action="open-subtitles" data-media-id="${escapeHtml(ep.mediaId)}" data-title="${escapeHtml(calItem.seriesTitle + ' ' + epCode)}" data-is-movie="0" data-season="${ep.seasonNumber}" data-episode="${ep.episodeNumber}" data-path="${escapeHtml(ep.filePath || '')}" title="Subtítulos">💬 Subs</button>` : ''}
            </div>
          </div>
        `;
      }).join('');

      return `
        <div style="background: var(--card-bg); ${dayBorder} border-radius: 8px; padding: 12px 14px;">
          <div style="font-weight: 700; font-size: 0.92rem; color: var(--text); margin-bottom: 10px; display: flex; align-items: center;">
            📅 ${dayHeader}
          </div>
          <div style="display: flex; flex-direction: column; gap: 8px;">
            ${episodesHtml}
          </div>
        </div>
      `;
    }).join('');

  } catch (err) {
    container.innerHTML = `<div style="text-align:center; padding: 40px; color: #f85149;">Error cargando calendario: ${escapeHtml(err.message)}</div>`;
  }
}

async function updateMediaDiskSpace() {
  try {
    const res = await client.call('system.diskSpace', {});
    if (res && res.freeBytes) {
      const freeGb = (res.freeBytes / (1024 * 1024 * 1024)).toFixed(1);
      const totalGb = (res.totalBytes / (1024 * 1024 * 1024)).toFixed(1);
      const textEl = document.getElementById('media-disk-space-text');
      if (textEl) {
        textEl.textContent = `${freeGb} GB libres de ${totalGb} GB`;
      }
    }
  } catch (e) { console.warn('[PulseCore] error al cargar medios:', e); }
}

const mediaFlipDetailsCache = new Map();

async function prefetchFlipCardMeta(mediaId) {
  if (!mediaId) return;
  const existing = mediaFlipDetailsCache.get(mediaId);
  if (existing === 'loading') return;
  if (existing && typeof existing === 'object') {
    const extraEl = document.getElementById(`flip-extra-${mediaId}`);
    if (extraEl && existing.extraHtml) extraEl.innerHTML = existing.extraHtml;
    const synEl = document.getElementById(`flip-synopsis-${mediaId}`);
    if (synEl && existing.overview && synEl.dataset.fallback === '1') {
      synEl.textContent = existing.overview;
      synEl.dataset.fallback = '0';
    }
    return;
  }

  mediaFlipDetailsCache.set(mediaId, 'loading');
  try {
    const res = await client.call('media.get', { id: mediaId });
    const item = res?.item || {};
    const details = res?.details || {};
    const episodes = Array.isArray(res?.episodes) ? res.episodes : [];
    const chips = [];

    const tmdb = details.ratings?.tmdb;
    const imdb = details.ratings?.imdb;
    if (tmdb && tmdb > 0) {
      chips.push(`<span class="media-flip-chip is-rating">★ ${Number(tmdb).toFixed(1)}</span>`);
    } else if (imdb && imdb > 0) {
      chips.push(`<span class="media-flip-chip is-rating">★ ${Number(imdb).toFixed(1)}</span>`);
    }

    if (details.runtimeMins && details.runtimeMins > 0) {
      chips.push(`<span class="media-flip-chip">⏱ ${details.runtimeMins}m</span>`);
    }
    if (item.mediaType === 'tv' && episodes.length > 0) {
      const doneCount = episodes.filter(e => e.status === 'completed').length;
      chips.push(`<span class="media-flip-chip">${doneCount}/${episodes.length} ep.</span>`);
    }
    if (Array.isArray(details.genres)) {
      for (const g of details.genres.slice(0, 2)) {
        if (g) chips.push(`<span class="media-flip-chip">${escapeHtml(String(g))}</span>`);
      }
    }

    const extraHtml = chips.join('');
    const overview = (details.spanishOverview || item.overview || '').trim();
    mediaFlipDetailsCache.set(mediaId, { extraHtml, overview });

    const extraEl = document.getElementById(`flip-extra-${mediaId}`);
    if (extraEl && extraHtml) {
      extraEl.innerHTML = extraHtml;
    }
    const synEl = document.getElementById(`flip-synopsis-${mediaId}`);
    if (synEl && overview) {
      synEl.textContent = overview;
      synEl.dataset.fallback = '0';
    }
    const libEntry = (Array.isArray(mediaLibraryItems) ? mediaLibraryItems : []).find(x => x.id === mediaId);
    if (libEntry && overview) {
      libEntry.overview = overview;
    }
  } catch (err) {
    mediaFlipDetailsCache.delete(mediaId);
    console.debug('[PulseCore] flip metadata:', err);
  }
}

function renderMediaCards() {
  const container = document.getElementById('media-grid-container');
  const emptyMsg = document.getElementById('media-empty-msg');
  if (!container) return;

  const textFilter = (document.getElementById('media-search-filter')?.value || '').toLowerCase().trim();

  const filtered = mediaLibraryItems.filter(item => {
    if (currentMediaTypeFilter !== 'all' && item.mediaType !== currentMediaTypeFilter) return false;
    if (currentMediaStatusFilter !== 'all' && item.status !== currentMediaStatusFilter) return false;
    if (textFilter) {
      const title = (item.title || '').toLowerCase();
      const orig = (item.originalTitle || '').toLowerCase();
      if (!title.includes(textFilter) && !orig.includes(textFilter)) return false;
    }
    return true;
  });

  if (filtered.length === 0) {
    container.innerHTML = '';
    if (emptyMsg) emptyMsg.style.display = 'block';
    return;
  }
  if (emptyMsg) emptyMsg.style.display = 'none';

function renderCardQualityLogo(qualityProfile) {
  const raw = String(qualityProfile || '1080p');
  const lower = raw.toLowerCase();
  let label = 'HD';
  if (lower.includes('2160') || lower.includes('4k') || lower.includes('uhd')) {
    label = '4K';
  } else if (lower.includes('sd') || lower.includes('480')) {
    label = 'SD';
  }
  return `<span class="badge-quality-outline badge-quality-logo" title="Perfil: ${escapeHtml(raw)}" aria-label="${label}">
    <svg viewBox="0 0 28 16" width="26" height="15" role="img" aria-hidden="true">
      <rect x="0.75" y="0.75" width="26.5" height="14.5" rx="3" fill="rgba(56, 189, 248, 0.1)" stroke="currentColor" stroke-width="1.25"></rect>
      <text x="14" y="11.5" text-anchor="middle" fill="currentColor" font-size="9.5" font-weight="800" font-family="ui-sans-serif, system-ui, -apple-system, sans-serif" letter-spacing="0.4">${label}</text>
    </svg>
  </span>`;
}

  container.innerHTML = filtered.map(item => {
    const isMovie = item.mediaType === 'movie';
    const yearStr = item.year ? item.year : '';
    const qualityStr = item.qualityProfile || '1080p';

    // 1. Estado en carátula: solo se muestran insignias cuando hay descarga o búsqueda activa
    let statusBadge = '';
    let backStatusLabel = '🟢 En disco';
    let backStatusClass = 'is-done';
    if (item.status === 'downloading') {
      statusBadge = `<span class="badge badge-dl media-exception-badge">⬇ Descargando</span>`;
      backStatusLabel = '⬇ Descargando';
      backStatusClass = 'is-dl';
    } else if (item.status === 'wanted') {
      statusBadge = `<span class="badge badge-wait media-exception-badge">🟡 Buscada</span>`;
      backStatusLabel = '🟡 Buscada';
      backStatusClass = 'is-wait';
    }

    const safeTitleData = escapeHtml(item.title);
    const safeFilePathData = escapeHtml(item.filePath || '');
    const isMovieFlag = isMovie ? '1' : '0';

    // 2. Botones de acción directos en la cara trasera (Ficha, Fuentes, Carpeta, Eliminar)
    const quickActions = `
      <div class="media-flip-actions">
        <button class="btn-icon-overlay" data-media-action="open-detail" data-media-id="${escapeHtml(item.id)}" data-stop-prop="1" title="Ver ficha técnica">
          <svg viewBox="0 0 24 24" width="13" height="13" stroke="currentColor" stroke-width="2" fill="none"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="16" x2="12" y2="12"></line><line x1="12" y1="8" x2="12.01" y2="8"></line></svg>
          <span>Ficha</span>
        </button>
        <button class="btn-icon-overlay" data-media-action="open-releases" data-media-id="${escapeHtml(item.id)}" data-title="${safeTitleData}" data-is-movie="${isMovieFlag}" data-stop-prop="1" title="Buscar fuentes en Kad y trackers">
          <svg viewBox="0 0 24 24" width="13" height="13" stroke="currentColor" stroke-width="2" fill="none"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line></svg>
          <span>Fuentes</span>
        </button>
        <button class="btn-icon-overlay" data-media-action="open-folder" data-media-id="${escapeHtml(item.id)}" data-path="${safeFilePathData}" data-title="${safeTitleData}" data-stop-prop="1" title="Abrir carpeta de destino">
          <svg viewBox="0 0 24 24" width="13" height="13" stroke="currentColor" stroke-width="2" fill="none"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path></svg>
          <span>Carpeta</span>
        </button>
        <button class="btn-icon-overlay btn-danger-overlay" data-media-action="delete-media" data-media-id="${escapeHtml(item.id)}" data-stop-prop="1" title="Eliminar de biblioteca">
          <svg viewBox="0 0 24 24" width="13" height="13" stroke="currentColor" stroke-width="2" fill="none"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path></svg>
          <span>Eliminar</span>
        </button>
      </div>
    `;

    // 3. Icono diminuto vectorial según tipo junto al año
    const typeIcon = isMovie
      ? `<svg viewBox="0 0 24 24" width="13" height="13" stroke="currentColor" stroke-width="2" fill="none"><rect x="2" y="2" width="20" height="20" rx="2.18" ry="2.18"></rect><line x1="7" y1="2" x2="7" y2="22"></line><line x1="17" y1="2" x2="17" y2="22"></line><line x1="2" y1="12" x2="22" y2="12"></line><line x1="2" y1="7" x2="7" y2="7"></line><line x1="2" y1="17" x2="7" y2="17"></line><line x1="17" y1="17" x2="22" y2="17"></line><line x1="17" y1="7" x2="22" y2="7"></line></svg>`
      : `<svg viewBox="0 0 24 24" width="13" height="13" stroke="currentColor" stroke-width="2" fill="none"><rect x="2" y="7" width="20" height="15" rx="2" ry="2"></rect><polyline points="17 2 12 7 7 2"></polyline></svg>`;

    const posterHtml = item.posterPath
      ? `<img class="media-poster-img movie-poster-img" src="${escapeHtml(item.posterPath)}" alt="${escapeHtml(item.title)}" loading="lazy" onerror="this.onerror=null;this.style.display='none';if(this.nextElementSibling)this.nextElementSibling.style.display='flex';">
         <div class="media-poster-fallback" style="display: none;">${typeIcon}<br><strong style="margin-top:8px;color:#f1f5f9;">${escapeHtml(item.title)}</strong><small style="color:var(--text-muted);margin-top:4px;">${yearStr}</small></div>`
      : `<div class="media-poster-fallback">${typeIcon}<br><strong style="margin-top:8px;color:#f1f5f9;">${escapeHtml(item.title)}</strong><small style="color:var(--text-muted);margin-top:4px;">${yearStr}</small></div>`;

    const bgUrl = (item.backdropPath || item.posterPath || '').replace(/'/g, "\\'");
    const bgStyle = bgUrl ? `style="background-image: url('${escapeHtml(bgUrl)}');"` : '';
    const cachedMeta = mediaFlipDetailsCache.get(item.id);
    const cachedExtraHtml = (cachedMeta && typeof cachedMeta === 'object' && cachedMeta.extraHtml)
      ? cachedMeta.extraHtml
      : `<span class="media-flip-chip">${escapeHtml(qualityStr)}</span>`;
    const rawOverview = (item.overview || (cachedMeta && typeof cachedMeta === 'object' ? cachedMeta.overview : '') || '').trim();
    const hasOverview = rawOverview.length > 0;
    const synopsisText = hasOverview
      ? rawOverview
      : 'Haz clic para abrir la ficha completa, reparto y fuentes P2P.';
    const origHtml = (item.originalTitle && item.originalTitle.toLowerCase() !== (item.title || '').toLowerCase())
      ? `<div class="media-flip-orig">${escapeHtml(item.originalTitle)}</div>`
      : '';

    return `
      <div class="media-card movie-card" id="media-card-${escapeHtml(item.id)}" data-media-action="open-detail" data-media-id="${escapeHtml(item.id)}" data-media-context="folder" data-path="${safeFilePathData}" data-title="${safeTitleData}" onmouseenter="prefetchFlipCardMeta('${escapeHtml(item.id)}')" title="Clic para ver ficha · Clic derecho para abrir carpeta">
        <div class="media-poster-box movie-poster-box">
          <div class="media-flip-inner">
            <div class="media-flip-face media-flip-front">
              ${statusBadge}
              ${posterHtml}
            </div>
            <div class="media-flip-face media-flip-back">
              <div class="media-flip-back-bg" ${bgStyle}></div>
              <div class="media-flip-header">
                <div class="media-flip-top-row">
                  <span class="media-flip-type">${isMovie ? '🎬' : '📺'} ${yearStr || '—'}</span>
                  <span class="media-flip-status ${backStatusClass}">${backStatusLabel}</span>
                </div>
                <div class="media-flip-back-title">${escapeHtml(item.title)}</div>
                ${origHtml}
                <div class="media-flip-extra" id="flip-extra-${item.id}">${cachedExtraHtml}</div>
              </div>
              <div class="media-flip-synopsis" id="flip-synopsis-${item.id}" data-fallback="${hasOverview ? '0' : '1'}">${escapeHtml(synopsisText)}</div>
              ${quickActions}
            </div>
          </div>
        </div>
        <div class="media-card-body movie-info">
          <div class="media-card-title movie-title">${escapeHtml(item.title)}</div>
          <div class="media-card-meta movie-meta">
            <span class="media-meta-left" title="${isMovie ? 'Película' : 'Serie TV'}">
              ${typeIcon}
              <span>${yearStr}</span>
            </span>
            ${renderCardQualityLogo(qualityStr)}
          </div>
        </div>
      </div>
    `;
  }).join('');
}

// Delegación centralizada para acciones de biblioteca multimedia (evita escapes frágiles en onclick)
document.addEventListener('click', (e) => {
  const el = e.target && e.target.closest ? e.target.closest('[data-media-action]') : null;
  if (!el) return;
  if (el.dataset.stopProp === '1') {
    e.stopPropagation();
  }
  if (el.dataset.closeDetail === '1' && typeof closeModal === 'function') {
    closeModal('modal-media-detail');
  }

  const d = el.dataset;
  switch (d.mediaAction) {
    case 'open-download':
      if (typeof openDownloadInQueue === 'function') openDownloadInQueue(d.downloadId);
      break;
    case 'open-detail':
      if (typeof openMediaDetail === 'function') openMediaDetail(d.mediaId);
      break;
    case 'open-releases':
      if (typeof openReleasesModal === 'function') openReleasesModal(d.mediaId, d.title || '', d.isMovie === '1');
      break;
    case 'open-folder':
      if (typeof openDestinationFolder === 'function') {
        openDestinationFolder({
          mediaId: d.mediaId || undefined,
          path: d.path || '',
          title: d.title || ''
        });
      }
      break;
    case 'delete-media':
      if (typeof deleteMediaItem === 'function') deleteMediaItem(d.mediaId);
      break;
    case 'single-autograb':
      if (typeof triggerSingleAutoGrab === 'function') triggerSingleAutoGrab(d.mediaId, d.episodeId || null, d.title || '');
      break;
    case 'replace-modal':
      if (typeof openCastilianReplacementModal === 'function') {
        openCastilianReplacementModal(
          d.mediaId,
          d.title || '',
          d.isMovie === '1',
          d.episodeId || null,
          d.season ? Number(d.season) : null,
          d.episode ? Number(d.episode) : null
        );
      }
      break;
    case 'replace-auto':
      if (typeof triggerReplaceWrongLanguage === 'function') {
        triggerReplaceWrongLanguage(
          d.mediaId,
          d.episodeId || null,
          d.season ? Number(d.season) : null,
          d.title || '',
          true
        );
      }
      break;
    case 'open-subtitles':
      if (typeof openSubtitlesModal === 'function') {
        openSubtitlesModal(
          d.mediaId,
          d.title || '',
          d.isMovie === '1',
          d.season ? Number(d.season) : undefined,
          d.episode ? Number(d.episode) : undefined,
          d.path || undefined
        );
      }
      break;
    case 'season-autograb':
      if (typeof triggerSeasonAutoGrab === 'function') triggerSeasonAutoGrab(d.mediaId, Number(d.season), d.title || '');
      break;
    case 'season-only':
      if (typeof setOnlySeasonMonitored === 'function') setOnlySeasonMonitored(d.mediaId, Number(d.season));
      break;
    case 'season-toggle':
      if (typeof toggleSeasonMonitored === 'function') toggleSeasonMonitored(d.mediaId, Number(d.season), d.monitored === '1');
      break;
    case 'season-releases':
      if (typeof openSeasonReleaseSearch === 'function') openSeasonReleaseSearch(d.mediaId, d.title || '', Number(d.season));
      break;
    case 'episode-releases':
      if (typeof openEpisodeReleaseSearch === 'function') {
        openEpisodeReleaseSearch(d.mediaId, d.title || '', Number(d.season), Number(d.episode), d.episodeId || undefined);
      }
      break;
    case 'delete-indexer':
      if (typeof deleteIndexer === 'function') deleteIndexer(d.indexerId, d.indexerName || '');
      break;
  }
});

document.addEventListener('contextmenu', (e) => {
  const card = e.target && e.target.closest ? e.target.closest('[data-media-context="folder"]') : null;
  if (!card) return;
  e.preventDefault();
  if (typeof openDestinationFolder === 'function') {
    openDestinationFolder({
      mediaId: card.dataset.mediaId || undefined,
      path: card.dataset.path || '',
      title: card.dataset.title || ''
    });
  }
});

