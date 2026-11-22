// Pulse Core WebUI — Filtros y ordenación de descargas, gráficas sparkline, monitor de ancho de banda y tooltips.
// Extraído de web/index.html.

// === ESTADO DE FILTROS Y ORDENACIÓN DE DESCARGAS ===
let currentFilterStatus = 'all';
let currentFilterProto = 'all';
let currentSearchFilter = '';
let currentSortCol = 'progress';
let currentSortDir = 'desc';
let cachedDownloads = [];
let selectedDownloadId = null;
let inspectorActiveTab = 'insp-general';

function setupTableSorting() {
  document.querySelectorAll('th.sortable').forEach(th => {
    th.addEventListener('click', () => {
      const col = th.dataset.sort;
      if (currentSortCol === col) {
        currentSortDir = currentSortDir === 'asc' ? 'desc' : 'asc';
      } else {
        currentSortCol = col;
        currentSortDir = (col === 'progress' || col === 'size' || col === 'speed' || col === 'sources') ? 'desc' : 'asc';
      }
      lastRenderedDownloadIds = '';
      updateSortIcons();
      renderDownloadsTable();
    });
  });
  updateSortIcons();
}

function updateSortIcons() {
  document.querySelectorAll('th.sortable .sort-icon').forEach(icon => {
    icon.textContent = '';
  });
  const activeIcon = document.getElementById(`sort-icon-${currentSortCol}`);
  if (activeIcon) {
    activeIcon.textContent = currentSortDir === 'asc' ? ' ▲' : ' ▼';
  }
}


// === SPARKLINES, MONITOR DETALLADO DE ANCHO DE BANDA Y SINCRONIZACIÓN DE SIDEBAR ===
const MAX_BW_HISTORY = 300; // 10 minutos a intervalos de ~2 s
const BW_DEFAULT_WINDOW = 60; // 2 minutos por defecto
const BW_MIN_SCALE_BPS = 10240; // Escala mínima de 10 kB/s

let downHistory = new Array(15).fill(0);
let upHistory = new Array(15).fill(0);
let perfDownHistory = new Array(MAX_BW_HISTORY).fill(0);
let perfUpHistory = new Array(MAX_BW_HISTORY).fill(0);
let bwTimestamps = new Array(MAX_BW_HISTORY).fill(null);
let bwSampleCount = 0;
let sessionPeakDown = 0;
let sessionPeakUp = 0;
let lastProtoStats = { ed2kDown: 0, btDown: 0, ed2kUp: 0, btUp: 0 };
let cachedUploadsActive = [];
let cachedUploadsQueued = [];
let cachedLimitsInfo = null;
let latestStatsTick = null;
let fallbackSessionDownBytes = 0;
let fallbackSessionUpBytes = 0;
let lastBwSampleMs = null;
let totalSourcesCache = 0;
let bwWindowSamples = BW_DEFAULT_WINDOW;
let bwSeriesMode = 'both';
let bwHoverIndex = null;
let bwLastChartState = null;
let bwLimitsPollTimer = null;

function formatDurationShort(totalSec) {
  const s = Math.max(0, Math.round(totalSec));
  if (s === 0) return 'Ahora';
  const m = Math.floor(s / 60);
  const rem = s % 60;
  if (m === 0) return `-${rem}s`;
  return rem === 0 ? `-${m}m` : `-${m}m ${rem}s`;
}

function formatSampleClock(tsMs) {
  if (!tsMs) return '';
  const d = new Date(tsMs);
  return d.toLocaleTimeString('es-ES', { hour12: false });
}

async function refreshBandwidthLimits() {
  try {
    cachedLimitsInfo = await client.call('limits.get', {});
  } catch (e) {
    // Ignorar silenciosamente si aún no hay conexión RPC
  }
  const modal = document.getElementById('bandwidth-details-modal');
  if (modal && modal.classList.contains('active')) {
    renderBandwidthModal();
  }
}

function openBandwidthModal() {
  const modal = document.getElementById('bandwidth-details-modal');
  if (!modal) return;
  hideSmartTooltip();
  modal.classList.add('active');
  updateTurtleModeUI();
  renderBandwidthModal();
  refreshBandwidthLimits();
  refreshDownloads().catch(() => {});
  if (bwLimitsPollTimer) clearInterval(bwLimitsPollTimer);
  bwLimitsPollTimer = setInterval(() => {
    refreshBandwidthLimits();
  }, 4000);
}

function closeBandwidthModal() {
  const modal = document.getElementById('bandwidth-details-modal');
  if (modal) modal.classList.remove('active');
  if (bwLimitsPollTimer) {
    clearInterval(bwLimitsPollTimer);
    bwLimitsPollTimer = null;
  }
  clearBwChartHover();
}

function formatSmartReasonLabel(uss) {
  if (!uss) return 'Desactivado';
  if (uss.reason === 'streaming' || uss.streamingCapActive) {
    return `🎬 Recortando por streaming (${uss.activeStreams || 1} activo/s)`;
  }
  if (uss.reason === 'quota_eco' || uss.quotaEcoActive) {
    return '🌿 Modo eco por cuota diaria';
  }
  if (!uss.enabled) return 'Desactivado';
  switch (uss.reason) {
    case 'latency':
      return 'Recortando por latencia (USS/LEDBAT)';
    case 'idle_cap':
      return 'Modo reposo (sin descargas activas)';
    case 'baseline':
    case 'waiting':
      return uss.state === 'waiting' ? 'Esperando muestras RTT' : 'Calibrando línea base';
    case 'normal':
    default:
      return 'Activo (USS/LEDBAT++)';
  }
}

async function toggleSmartUploadMode() {
  const currentlyEnabled = Boolean(cachedLimitsInfo && cachedLimitsInfo.uss && cachedLimitsInfo.uss.enabled);
  const nextEnabled = !currentlyEnabled;
  try {
    if (nextEnabled && turtleModeActive) {
      turtleModeActive = false;
      localStorage.setItem('mule_turtle_mode', 'false');
      updateTurtleModeUI();
      await client.call('limits.set', { downloadLimit: 0, uploadLimit: 0, uss: { enabled: true } });
    } else {
      await client.call('limits.set', { uss: { enabled: nextEnabled } });
    }
    addLog(
      'info',
      nextEnabled
        ? '⚡ Control inteligente de subida activado (USS / LEDBAT++).'
        : '⚡ Control inteligente de subida desactivado.'
    );
    await refreshBandwidthLimits();
  } catch (e) {
    addLog('error', 'Error al conmutar el control inteligente de subida: ' + (e.message || JSON.stringify(e)));
  }
}

async function applySmartUploadControls() {
  const readKbs = (id) => {
    const el = document.getElementById(id);
    if (!el) return null;
    const raw = el.value.trim();
    if (raw === '') return 0;
    const n = parseInt(raw, 10);
    return Number.isNaN(n) || n < 0 ? 0 : n;
  };
  const maxUpKbs = readKbs('bw-smart-max-up');
  const idleCapKbs = readKbs('bw-smart-idle-cap');
  const streamingCapKbs = readKbs('bw-smart-streaming-cap');
  const slotTargetKbs = readKbs('bw-smart-slot-target');
  try {
    const payload = {
      uss: {
        maxUploadBytesPerSec: (maxUpKbs || 0) * 1024,
        idleUploadCapBytesPerSec: (idleCapKbs || 0) * 1024,
        streamingUploadCapBytesPerSec: (streamingCapKbs || 0) * 1024,
      },
      smartSlotTargetKbs: slotTargetKbs || 0,
    };
    if (!cachedLimitsInfo?.uss?.enabled) {
      payload.uploadLimit = maxUpKbs || 0;
    }
    await client.call('limits.set', payload);
    addLog(
      'info',
      `🧠 Ajustes inteligentes aplicados: techo ${maxUpKbs || 'ilimitado'} KB/s, reposo ${idleCapKbs || 'off'} KB/s, ranura ${slotTargetKbs || 'clásico'} KB/s.`
    );
    await refreshBandwidthLimits();
  } catch (e) {
    addLog('error', 'Error aplicando ajustes inteligentes de subida: ' + (e.message || JSON.stringify(e)));
  }
}

function setBandwidthWindow(samples) {
  bwWindowSamples = Math.max(15, Math.min(MAX_BW_HISTORY, Number(samples) || BW_DEFAULT_WINDOW));
  document.querySelectorAll('#bw-window-group .filter-btn').forEach(btn => {
    btn.classList.toggle('active', Number(btn.dataset.bwWindow) === bwWindowSamples);
  });
  clearBwChartHover();
  renderBandwidthModal();
}

function setBandwidthSeries(mode) {
  bwSeriesMode = (mode === 'down' || mode === 'up') ? mode : 'both';
  document.querySelectorAll('#bw-series-group .filter-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.bwSeries === bwSeriesMode);
  });
  clearBwChartHover();
  renderBandwidthModal();
}

function updateBwChartHover(idx) {
  if (!bwLastChartState) return;
  const { downSlice, upSlice, tsSlice, maxScale, svgW, padTop, plotH, showDown, showUp } = bwLastChartState;
  const len = downSlice.length;
  if (len === 0) return;

  const safeIdx = Math.max(0, Math.min(len - 1, idx));
  bwHoverIndex = safeIdx;
  const x = +((safeIdx / Math.max(1, len - 1)) * svgW).toFixed(1);
  const valDown = downSlice[safeIdx] || 0;
  const valUp = upSlice[safeIdx] || 0;
  const yDown = +(padTop + plotH - (Math.max(0, valDown) / maxScale) * plotH).toFixed(1);
  const yUp = +(padTop + plotH - (Math.max(0, valUp) / maxScale) * plotH).toFixed(1);

  const hoverLine = document.getElementById('bw-hover-line');
  const dotDown = document.getElementById('bw-hover-dot-down');
  const dotUp = document.getElementById('bw-hover-dot-up');
  const hoverInfo = document.getElementById('bw-chart-hover-info');

  if (hoverLine) {
    hoverLine.setAttribute('x1', x);
    hoverLine.setAttribute('x2', x);
    hoverLine.style.display = '';
  }
  if (dotDown) {
    if (showDown) {
      dotDown.setAttribute('cx', x);
      dotDown.setAttribute('cy', yDown);
      dotDown.style.display = '';
    } else {
      dotDown.style.display = 'none';
    }
  }
  if (dotUp) {
    if (showUp) {
      dotUp.setAttribute('cx', x);
      dotUp.setAttribute('cy', yUp);
      dotUp.style.display = '';
    } else {
      dotUp.style.display = 'none';
    }
  }
  if (hoverInfo) {
    const agoSec = (len - 1 - safeIdx) * 2;
    const tsStr = tsSlice[safeIdx] ? formatSampleClock(tsSlice[safeIdx]) : formatDurationShort(agoSec);
    const relStr = agoSec === 0 ? 'Ahora' : formatDurationShort(agoSec);
    const timeLabel = tsSlice[safeIdx] ? `${tsStr} (${relStr})` : relStr;
    hoverInfo.innerHTML = `<span style="color:#e2e8f0;">${timeLabel}</span> · <span style="color:#38bdf8;">↓ ${formatSpeed(valDown)}</span> · <span style="color:#34d399;">↑ ${formatSpeed(valUp)}</span>`;
  }
}

function handleBwChartMouseMove(event) {
  const svg = document.getElementById('bw-detail-svg');
  if (!svg || !bwLastChartState) return;
  const rect = svg.getBoundingClientRect();
  if (!rect.width) return;
  const relX = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  const idx = Math.round(relX * Math.max(0, bwLastChartState.downSlice.length - 1));
  updateBwChartHover(idx);
}

function clearBwChartHover() {
  bwHoverIndex = null;
  const hoverLine = document.getElementById('bw-hover-line');
  const dotDown = document.getElementById('bw-hover-dot-down');
  const dotUp = document.getElementById('bw-hover-dot-up');
  const hoverInfo = document.getElementById('bw-chart-hover-info');
  if (hoverLine) hoverLine.style.display = 'none';
  if (dotDown) dotDown.style.display = 'none';
  if (dotUp) dotUp.style.display = 'none';
  if (hoverInfo) hoverInfo.textContent = 'Pasa el cursor sobre la gráfica para inspeccionar un instante';
}

function renderBandwidthModal() {
  const modal = document.getElementById('bandwidth-details-modal');
  if (!modal || !modal.classList.contains('active')) return;

  const downSlice = perfDownHistory.slice(-bwWindowSamples);
  const upSlice = perfUpHistory.slice(-bwWindowSamples);
  const tsSlice = bwTimestamps.slice(-bwWindowSamples);

  const curDown = downSlice.length ? downSlice[downSlice.length - 1] : 0;
  const curUp = upSlice.length ? upSlice[upSlice.length - 1] : 0;

  const validCount = Math.min(bwSampleCount, bwWindowSamples);
  const avgDown = validCount > 0
    ? downSlice.slice(-validCount).reduce((acc, v) => acc + v, 0) / validCount
    : 0;
  const avgUp = validCount > 0
    ? upSlice.slice(-validCount).reduce((acc, v) => acc + v, 0) / validCount
    : 0;
  const winPeakDown = Math.max(...downSlice, 0);
  const winPeakUp = Math.max(...upSlice, 0);

  // Tarjetas KPI 1 y 2: Velocidades, picos, protocolos y límites
  const setText = (id, txt) => {
    const el = document.getElementById(id);
    if (el) el.textContent = txt;
  };
  setText('bw-cur-down', formatSpeed(curDown));
  setText('bw-peak-down', formatSpeed(sessionPeakDown));
  setText('bw-avg-down', formatSpeed(avgDown));
  setText('bw-proto-down', `eD2K: ${formatSpeed(lastProtoStats.ed2kDown)} | BT: ${formatSpeed(lastProtoStats.btDown)}`);

  setText('bw-cur-up', formatSpeed(curUp));
  setText('bw-peak-up', formatSpeed(sessionPeakUp));
  setText('bw-avg-up', formatSpeed(avgUp));
  setText('bw-proto-up', `eD2K: ${formatSpeed(lastProtoStats.ed2kUp)} | BT: ${formatSpeed(lastProtoStats.btUp)}`);

  const dlLimit = cachedLimitsInfo ? (cachedLimitsInfo.downloadBytesPerSec || 0) : (turtleModeActive ? 500 * 1024 : 0);
  const ulLimit = cachedLimitsInfo ? (cachedLimitsInfo.uploadBytesPerSec || 0) : (turtleModeActive ? 100 * 1024 : 0);
  const uss = cachedLimitsInfo && cachedLimitsInfo.uss ? cachedLimitsInfo.uss : null;

  setText('bw-limit-down', dlLimit > 0 ? `${formatSpeed(dlLimit)}${cachedLimitsInfo?.downloadRatioApplied ? ' (Ratio eD2K)' : ''}` : 'Sin límite');
  const upLimitText = (uss && uss.enabled && uss.currentLimitBytesPerSec > 0)
    ? `${formatSpeed(uss.currentLimitBytesPerSec)} (USS)`
    : (ulLimit > 0 ? formatSpeed(ulLimit) : 'Sin límite');
  const splitHint = (uss && uss.torrentLimitBytesPerSec > 0 && uss.ed2kLimitBytesPerSec > 0 && uss.ed2kLimitBytesPerSec !== uss.torrentLimitBytesPerSec)
    ? ` · eD2K ${formatSpeed(uss.ed2kLimitBytesPerSec)} / BT ${formatSpeed(uss.torrentLimitBytesPerSec)}`
    : '';
  setText('bw-limit-up', `${upLimitText}${splitHint}`);

  const modalBadge = document.getElementById('bw-modal-badge');
  if (modalBadge) {
    if (turtleModeActive) {
      modalBadge.className = 'badge badge-completing';
      modalBadge.textContent = '🐢 Modo Tortuga (500/100 kB/s)';
    } else if (uss && uss.enabled) {
      modalBadge.className = uss.reason === 'latency' ? 'badge badge-completing' : 'badge badge-dl';
      modalBadge.textContent = `⚡ ${formatSmartReasonLabel(uss)}`;
    } else if (dlLimit > 0 || ulLimit > 0) {
      modalBadge.className = 'badge badge-completing';
      modalBadge.textContent = '⚡ Límite Activo';
    } else {
      modalBadge.className = 'badge badge-completed';
      modalBadge.textContent = '🟢 Sin límites';
    }
  }

  const smartToggleBtn = document.getElementById('btn-bw-smart-toggle');
  if (smartToggleBtn) {
    const isSmartOn = Boolean(uss && uss.enabled);
    smartToggleBtn.classList.toggle('btn-primary', isSmartOn);
    smartToggleBtn.textContent = isSmartOn ? '⚡ Control Inteligente: ON' : '⚡ Control Inteligente: OFF';
  }

  const smartStatusEl = document.getElementById('bw-smart-status-text');
  if (smartStatusEl) {
    smartStatusEl.textContent = formatSmartReasonLabel(uss);
    smartStatusEl.style.color = (uss && uss.enabled)
      ? (uss.reason === 'latency' ? '#fbbf24' : '#34d399')
      : 'var(--text-muted)';
  }
  const smartRttEl = document.getElementById('bw-smart-rtt-text');
  if (smartRttEl) {
    if (uss && uss.enabled && uss.latencyMs > 0) {
      const qDelay = Number(uss.queuingDelayMs || 0);
      const targetStr = uss.pingTarget ? ` · ${uss.pingTarget}` : '';
      smartRttEl.textContent = `RTT ${uss.latencyMs} ms (base ${uss.lowestPingMs || uss.latencyMs} ms, +${qDelay} ms cola)${targetStr}`;
    } else {
      smartRttEl.textContent = '';
    }
  }
  const syncSmartInput = (id, kbsVal) => {
    const el = document.getElementById(id);
    if (el && document.activeElement !== el) {
      el.value = kbsVal > 0 ? String(Math.round(kbsVal)) : '';
    }
  };
  if (cachedLimitsInfo) {
    const cfgMaxUpBps = uss && uss.config ? (uss.config.maxUploadBps || 0) : (cachedLimitsInfo.uploadConfiguredBytesPerSec || 0);
    const cfgIdleBps = uss && uss.config ? (uss.config.idleUploadCapBps || 0) : 0;
    const cfgStreamingBps = uss && uss.config ? (uss.config.streamingUploadCapBps || 0) : 0;
    syncSmartInput('bw-smart-max-up', cfgMaxUpBps / 1024);
    syncSmartInput('bw-smart-idle-cap', cfgIdleBps / 1024);
    syncSmartInput('bw-smart-streaming-cap', cfgStreamingBps / 1024);
    syncSmartInput('bw-smart-slot-target', cachedLimitsInfo.smartSlotTargetKbs || 0);
  }

  // Tarjeta KPI 3: Volumen de sesión
  const sessDown = Math.max(latestStatsTick ? (latestStatsTick.sessionDownloadedBytes || 0) : 0, Math.round(fallbackSessionDownBytes));
  const sessUp = Math.max(latestStatsTick ? (latestStatsTick.sessionUploadedBytes || 0) : 0, Math.round(fallbackSessionUpBytes));
  const sessRatio = sessDown > 0 ? (sessUp / sessDown).toFixed(2) : (sessUp > 0 ? '∞' : '0.00');
  const capturedCount = Math.min(bwSampleCount, MAX_BW_HISTORY);
  const capturedSecs = capturedCount * 2;
  const capturedDur = capturedSecs >= 60 ? `${Math.floor(capturedSecs / 60)}m ${capturedSecs % 60}s` : `${capturedSecs}s`;

  setText('bw-sess-ratio', `Ratio ${sessRatio}`);
  setText('bw-sess-down', formatBytes(sessDown));
  setText('bw-sess-up', formatBytes(sessUp));
  setText('bw-sess-samples', `${capturedCount} muestras (${capturedDur})`);

  // Tarjeta KPI 4: Acumulado histórico y enjambre
  const lifeDown = latestStatsTick ? (latestStatsTick.lifetimeDownloadedBytes || 0) : null;
  const lifeUp = latestStatsTick ? (latestStatsTick.lifetimeUploadedBytes || 0) : null;
  const lifeRatio = (lifeDown !== null && lifeDown > 0)
    ? (lifeUp / lifeDown).toFixed(2)
    : (lifeUp > 0 ? '∞' : '0.00');
  const peersConn = latestStatsTick ? (latestStatsTick.peersConnected || 0) : 0;

  setText('bw-life-ratio', `Ratio ${lifeRatio}`);
  setText('bw-life-down', lifeDown !== null ? formatBytes(lifeDown) : 'Sincronizando...');
  setText('bw-life-up', lifeUp !== null ? formatBytes(lifeUp) : 'Sincronizando...');
  setText('bw-active-peers', `${peersConn} pares • ${totalSourcesCache} fuentes`);
  setText('bw-upload-slots', `${cachedUploadsActive.length} activas • ${cachedUploadsQueued.length} en cola`);

  // Gráfica SVG detallada
  const showDown = bwSeriesMode === 'both' || bwSeriesMode === 'down';
  const showUp = bwSeriesMode === 'both' || bwSeriesMode === 'up';
  const effUpLimitBps = (uss && uss.enabled && uss.currentLimitBytesPerSec > 0)
    ? uss.currentLimitBytesPerSec
    : ulLimit;
  const scaleCandidateLimit = (showUp && effUpLimitBps > 0 && effUpLimitBps <= Math.max(winPeakUp * 2, BW_MIN_SCALE_BPS))
    ? effUpLimitBps
    : 0;
  const maxScale = Math.max(showDown ? winPeakDown : 0, showUp ? winPeakUp : 0, scaleCandidateLimit, BW_MIN_SCALE_BPS);

  setText('bw-y-100', formatSpeed(maxScale));
  setText('bw-y-75', formatSpeed(maxScale * 0.75));
  setText('bw-y-50', formatSpeed(maxScale * 0.5));
  setText('bw-y-25', formatSpeed(maxScale * 0.25));
  setText('bw-y-0', '0 B/s');

  const totalWinSec = bwWindowSamples * 2;
  setText('bw-x-0', formatDurationShort(totalWinSec));
  setText('bw-x-1', formatDurationShort(totalWinSec * 0.75));
  setText('bw-x-2', formatDurationShort(totalWinSec * 0.5));
  setText('bw-x-3', formatDurationShort(totalWinSec * 0.25));
  setText('bw-x-4', 'Ahora');

  const svgW = 720;
  const padTop = 10;
  const plotH = 200;
  const baseY = padTop + plotH;

  function buildSeriesPaths(arr) {
    const len = arr.length;
    const pts = arr.map((val, idx) => {
      const x = +((idx / Math.max(1, len - 1)) * svgW).toFixed(1);
      const y = +(padTop + plotH - (Math.max(0, val) / maxScale) * plotH).toFixed(1);
      return `${x},${y}`;
    });
    return {
      line: pts.join(' '),
      fill: `M 0,${baseY} L ${pts.join(' L ')} L ${svgW},${baseY} Z`,
    };
  }

  const bwDownLine = document.getElementById('bw-down-line');
  const bwDownFill = document.getElementById('bw-down-fill');
  const bwUpLine = document.getElementById('bw-up-line');
  const bwUpFill = document.getElementById('bw-up-fill');

  if (showDown) {
    const dPaths = buildSeriesPaths(downSlice);
    if (bwDownLine) bwDownLine.setAttribute('points', dPaths.line);
    if (bwDownFill) bwDownFill.setAttribute('d', dPaths.fill);
  } else {
    if (bwDownLine) bwDownLine.setAttribute('points', '');
    if (bwDownFill) bwDownFill.setAttribute('d', '');
  }

  if (showUp) {
    const uPaths = buildSeriesPaths(upSlice);
    if (bwUpLine) bwUpLine.setAttribute('points', uPaths.line);
    if (bwUpFill) bwUpFill.setAttribute('d', uPaths.fill);
  } else {
    if (bwUpLine) bwUpLine.setAttribute('points', '');
    if (bwUpFill) bwUpFill.setAttribute('d', '');
  }

  const peakLineDown = document.getElementById('bw-peak-line-down');
  if (peakLineDown) {
    if (showDown && winPeakDown > 0) {
      const yPeakD = +(padTop + plotH - (winPeakDown / maxScale) * plotH).toFixed(1);
      peakLineDown.setAttribute('y1', yPeakD);
      peakLineDown.setAttribute('y2', yPeakD);
      peakLineDown.style.display = '';
    } else {
      peakLineDown.style.display = 'none';
    }
  }

  const peakLineUp = document.getElementById('bw-peak-line-up');
  if (peakLineUp) {
    if (showUp && winPeakUp > 0) {
      const yPeakU = +(padTop + plotH - (winPeakUp / maxScale) * plotH).toFixed(1);
      peakLineUp.setAttribute('y1', yPeakU);
      peakLineUp.setAttribute('y2', yPeakU);
      peakLineUp.style.display = '';
    } else {
      peakLineUp.style.display = 'none';
    }
  }

  const limitLineUp = document.getElementById('bw-limit-line-up');
  if (limitLineUp) {
    if (showUp && effUpLimitBps > 0 && effUpLimitBps <= maxScale) {
      const yLimU = +(padTop + plotH - (effUpLimitBps / maxScale) * plotH).toFixed(1);
      limitLineUp.setAttribute('y1', yLimU);
      limitLineUp.setAttribute('y2', yLimU);
      limitLineUp.style.display = '';
    } else {
      limitLineUp.style.display = 'none';
    }
  }

  bwLastChartState = { downSlice, upSlice, tsSlice, maxScale, svgW, padTop, plotH, showDown, showUp };
  if (bwHoverIndex !== null) {
    updateBwChartHover(bwHoverIndex);
  }

  // Desglose inferior: Descargas activas con tráfico
  const dlRows = cachedDownloads
    .map(item => {
      const dl = item.snapshot || item;
      const rawName = dl.fileName || dl.file_name || 'Archivo';
      const hash = dl.ed2kHash || dl.ed2k_hash || '';
      const total = dl.totalBytes || dl.total_bytes || 0;
      const received = dl.receivedBytes || dl.received_bytes || 0;
      const progress = total > 0 ? ((received / total) * 100).toFixed(1) : (dl.progress ? (dl.progress * 100).toFixed(1) : '0.0');
      const rate = dl.downloadRateBps || dl.download_rate_bps || 0;
      const sources = dl.sourceCount || dl.source_count || 0;
      const status = (dl.status || '').toLowerCase();
      const isTorrent = dl.protocol === 'torrent' || item.protocol === 'torrent' || rawName.startsWith('⚡ [Torrent]') || (hash.length === 40 && !item.ed2kHash && (!dl.parts || dl.parts.length === 0));
      const cleanName = fixMojibake(rawName.replace(/^⚡ \[Torrent\]\s*/i, '').replace(/^\[Torrent\]\s*/i, ''));
      return { cleanName, isTorrent, progress, rate, sources, status };
    })
    .filter(r => r.rate > 0 || r.status === 'downloading')
    .sort((a, b) => b.rate - a.rate || Number(b.progress) - Number(a.progress));

  setText('bw-active-downloads-count', `${dlRows.length} activas`);
  const dlListEl = document.getElementById('bw-active-downloads-list');
  if (dlListEl) {
    if (dlRows.length === 0) {
      dlListEl.innerHTML = '<div style="color: var(--text-muted); font-size: 0.78rem; padding: 12px 0; text-align: center;">Sin descargas transfiriendo en este instante.</div>';
    } else {
      dlListEl.innerHTML = dlRows.slice(0, 20).map(r => `
        <div class="bw-transfer-row">
          <div style="min-width: 0; flex: 1;">
            <div style="display: flex; align-items: center; gap: 6px; min-width: 0;">
              <span class="badge ${r.isTorrent ? 'badge-torrent' : 'badge-ed2k'}">${r.isTorrent ? 'BT' : 'eD2K'}</span>
              <span style="font-weight: 600; color: #f8fafc; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${escapeHtml(r.cleanName)}">${escapeHtml(r.cleanName)}</span>
            </div>
            <div style="font-size: 0.72rem; color: var(--text-muted); margin-top: 2px;">${r.progress}% completado • ${r.sources} fuentes</div>
          </div>
          <div style="font-family: var(--font-mono); font-weight: 700; color: #38bdf8; white-space: nowrap;">${formatSpeed(r.rate)}</div>
        </div>
      `).join('');
    }
  }

  // Desglose inferior: Subidas y ranuras activas
  const uploadRows = [];
  cachedUploadsActive.forEach(u => {
    const cleanName = fixMojibake(u.fileName || 'Archivo compartido');
    uploadRows.push({
      name: cleanName,
      proto: u.state === 'trickle' ? 'Goteo' : 'eD2K',
      isTrickle: u.state === 'trickle',
      isTorrent: false,
      subtext: `${u.clientAddress || 'Par remoto'} • ${formatBytes(u.transferredBytes || 0)} enviados`,
      rate: u.uploadRateBps || 0,
    });
  });
  cachedDownloads.forEach(item => {
    const dl = item.snapshot || item;
    const rate = dl.uploadRateBps || dl.upload_rate_bps || 0;
    if (rate <= 0) return;
    const rawName = dl.fileName || dl.file_name || 'Archivo';
    const hash = dl.ed2kHash || dl.ed2k_hash || '';
    const isTorrent = dl.protocol === 'torrent' || item.protocol === 'torrent' || rawName.startsWith('⚡ [Torrent]') || (hash.length === 40 && !item.ed2kHash && (!dl.parts || dl.parts.length === 0));
    if (!isTorrent) return; // Evita duplicar subidas eD2K ya contabilizadas en uploads.list
    const cleanName = fixMojibake(rawName.replace(/^⚡ \[Torrent\]\s*/i, '').replace(/^\[Torrent\]\s*/i, ''));
    uploadRows.push({
      name: cleanName,
      proto: 'BT',
      isTrickle: false,
      isTorrent: true,
      subtext: `Enjambre BitTorrent • ${formatBytes(dl.uploadedBytes || 0)} enviados`,
      rate,
    });
  });
  uploadRows.sort((a, b) => b.rate - a.rate);

  setText('bw-active-uploads-count', `${uploadRows.length} activas • ${cachedUploadsQueued.length} en cola`);
  const ulListEl = document.getElementById('bw-active-uploads-list');
  if (ulListEl) {
    if (uploadRows.length === 0) {
      ulListEl.innerHTML = `<div style="color: var(--text-muted); font-size: 0.78rem; padding: 12px 0; text-align: center;">Sin subidas activas en este instante (${cachedUploadsQueued.length} clientes en cola).</div>`;
    } else {
      ulListEl.innerHTML = uploadRows.slice(0, 20).map(u => {
        const badgeCls = u.isTorrent ? 'badge-torrent' : (u.isTrickle ? 'badge-completing' : 'badge-ed2k');
        return `
          <div class="bw-transfer-row">
            <div style="min-width: 0; flex: 1;">
              <div style="display: flex; align-items: center; gap: 6px; min-width: 0;">
                <span class="badge ${badgeCls}">${u.proto}</span>
                <span style="font-weight: 600; color: #f8fafc; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${escapeHtml(u.name)}">${escapeHtml(u.name)}</span>
              </div>
              <div style="font-size: 0.72rem; color: var(--text-muted); margin-top: 2px;">${escapeHtml(u.subtext)}</div>
            </div>
            <div style="font-family: var(--font-mono); font-weight: 700; color: #34d399; white-space: nowrap;">${formatSpeed(u.rate)}</div>
          </div>
        `;
      }).join('');
    }
  }
}

function updateSparklines(downSpeed, upSpeed) {
  const nowMs = Date.now();
  if (lastBwSampleMs !== null) {
    const dtSec = Math.max(0, Math.min(10, (nowMs - lastBwSampleMs) / 1000));
    fallbackSessionDownBytes += downSpeed * dtSec;
    fallbackSessionUpBytes += upSpeed * dtSec;
  }
  lastBwSampleMs = nowMs;
  bwSampleCount++;
  sessionPeakDown = Math.max(sessionPeakDown, downSpeed);
  sessionPeakUp = Math.max(sessionPeakUp, upSpeed);

  downHistory.push(downSpeed);
  downHistory.shift();
  upHistory.push(upSpeed);
  upHistory.shift();

  perfDownHistory.push(downSpeed);
  perfDownHistory.shift();
  perfUpHistory.push(upSpeed);
  perfUpHistory.shift();
  bwTimestamps.push(nowMs);
  bwTimestamps.shift();

  function pointsToSvg(arr, width = 50, height = 16) {
    const max = Math.max(...arr, 1024);
    return arr.map((val, idx) => {
      const x = Math.round((idx / (arr.length - 1)) * width);
      const y = Math.round(height - (val / max) * (height - 3) - 1);
      return `${x},${y}`;
    }).join(' ');
  }

  const downPoly = document.getElementById('sparkline-down-poly');
  if (downPoly) downPoly.setAttribute('points', pointsToSvg(downHistory));
  const upPoly = document.getElementById('sparkline-up-poly');
  if (upPoly) upPoly.setAttribute('points', pointsToSvg(upHistory));

  // Actualizar gráfica extendida del panel inspector (solo si la pestaña de rendimiento está visible)
  const inspPerf = document.getElementById('insp-perf');
  const isPerfVisible = inspPerf && inspPerf.style.display !== 'none';

  if (isPerfVisible) {
    const perfDownEl = document.getElementById('insp-perf-down');
    if (perfDownEl) perfDownEl.textContent = formatSpeed(downSpeed);
    const perfUpEl = document.getElementById('insp-perf-up');
    if (perfUpEl) perfUpEl.textContent = formatSpeed(upSpeed);

    const perfSvg = document.getElementById('insp-perf-svg');
    if (perfSvg) {
      const inspDownSlice = perfDownHistory.slice(-60);
      const inspUpSlice = perfUpHistory.slice(-60);
      const maxPerf = Math.max(...inspDownSlice, ...inspUpSlice, 10240);
      const w = 500;
      const h = 140;

      function makePathData(arr) {
        const pts = arr.map((val, idx) => {
          const x = Math.round((idx / (arr.length - 1)) * w);
          const y = Math.round(h - (val / maxPerf) * (h - 14) - 6);
          return `${x},${y}`;
        });
        const polylineStr = pts.join(' ');
        const fillD = `M 0,${h} L ${pts.join(' L ')} L ${w},${h} Z`;
        return { polylineStr, fillD };
      }

      const downData = makePathData(inspDownSlice);
      const upData = makePathData(inspUpSlice);

      const pDownLine = document.getElementById('perf-down-line');
      const pDownFill = document.getElementById('perf-down-fill');
      if (pDownLine) pDownLine.setAttribute('points', downData.polylineStr);
      if (pDownFill) pDownFill.setAttribute('d', downData.fillD);

      const pUpLine = document.getElementById('perf-up-line');
      const pUpFill = document.getElementById('perf-up-fill');
      if (pUpLine) pUpLine.setAttribute('points', upData.polylineStr);
      if (pUpFill) pUpFill.setAttribute('d', upData.fillD);
    }
  }

  renderBandwidthModal();
}

async function refreshDownloads() {
  try {
    const res = await client.call('downloads.list');
    const list = res.downloads || [];
    cachedDownloads = list;

    // Conteo para los botones de filtro
    let countDownloading = 0, countQueued = 0, countPaused = 0, countCompleted = 0, countCompleting = 0;
    let totalDown = 0, totalUp = 0, totalSources = 0;
    let ed2kDown = 0, btDown = 0, ed2kUp = 0, btUp = 0;
    let countBt = 0, countEd2k = 0;

    list.forEach(item => {
      const dl = item.snapshot || item;
      const rawName = dl.fileName || dl.file_name || '';
      const hash = dl.ed2kHash || dl.ed2k_hash || '';
      const status = (dl.status || '').toLowerCase();
      const total = dl.totalBytes || dl.total_bytes || 0;
      const transferred = dl.receivedBytes || dl.received_bytes || 0;
      const isComplete = (total > 0 && transferred >= total) || status === 'completed';

      const isTorrent = dl.protocol === 'torrent' || item.protocol === 'torrent' || rawName.startsWith('⚡ [Torrent]') || (hash.length === 40 && !item.ed2kHash && (!dl.parts || dl.parts.length === 0));
      if (isTorrent) countBt++; else countEd2k++;

      if (status === 'completing') {
        countCompleting++;
      } else if (status === 'completed' || isComplete) {
        countCompleted++;
      } else if (status === 'downloading') {
        countDownloading++;
      } else if (status === 'queued' || status === 'waiting_for_sources' || status === 'waitingforsources') {
        countQueued++;
      } else if (status === 'paused') {
        countPaused++;
      } else {
        countQueued++;
      }

      const dlRate = dl.downloadRateBps || dl.download_rate_bps || 0;
      const ulRate = dl.uploadRateBps || dl.upload_rate_bps || 0;
      totalDown += dlRate;
      totalUp += ulRate;
      if (isTorrent) {
        btDown += dlRate;
        btUp += ulRate;
      } else {
        ed2kDown += dlRate;
        ed2kUp += ulRate;
      }
      totalSources += (dl.sourceCount || dl.source_count || 0);
    });
    totalSourcesCache = totalSources;

    // Sumar también las subidas activas de archivos compartidos (eD2k/Kad)
    let activeUploads = [];
    let queuedUploads = [];
    try {
      const uploadsRes = await client.call('uploads.list', {});
      if (uploadsRes) {
        if (Array.isArray(uploadsRes.active)) {
          activeUploads = uploadsRes.active;
          uploadsRes.active.forEach(u => {
            const rate = u.uploadRateBps || 0;
            totalUp += rate;
            ed2kUp += rate;
          });
        }
        if (Array.isArray(uploadsRes.queued)) {
          queuedUploads = uploadsRes.queued;
        }
      }
    } catch (e) {
      // Ignorar silenciosamente si uploads.list no responde
    }
    cachedUploadsActive = activeUploads;
    cachedUploadsQueued = queuedUploads;
    lastProtoStats = { ed2kDown, btDown, ed2kUp, btUp };

    document.getElementById('cnt-all').textContent = `(${list.length})`;
    document.getElementById('cnt-downloading').textContent = `(${countDownloading})`;
    const cntQueued = document.getElementById('cnt-queued');
    if (cntQueued) cntQueued.textContent = `(${countQueued})`;
    const cntCompleting = document.getElementById('cnt-completing');
    if (cntCompleting) cntCompleting.textContent = `(${countCompleting})`;
    document.getElementById('cnt-completed').textContent = `(${countCompleted})`;
    document.getElementById('cnt-paused').textContent = `(${countPaused})`;

    // Sincronizar contadores en la barra lateral
    const setSidebarCnt = (id, val) => {
      const el = document.getElementById(id);
      if (el) el.textContent = val;
    };
    setSidebarCnt('count-proto-all', list.length);
    setSidebarCnt('count-proto-torrent', countBt);
    setSidebarCnt('count-proto-ed2k', countEd2k);
    setSidebarCnt('count-status-all', list.length);
    setSidebarCnt('count-status-downloading', countDownloading);
    setSidebarCnt('count-status-queued', countQueued);
    setSidebarCnt('count-status-completing', countCompleting);
    setSidebarCnt('count-status-completed', countCompleted);
    setSidebarCnt('count-status-paused', countPaused);
    document.getElementById('global-down').textContent = formatSpeed(totalDown);
    document.getElementById('global-up').textContent = formatSpeed(totalUp);
    updateSparklines(totalDown, totalUp);
    document.getElementById('downloads-summary').innerHTML =
      `<span style="color:#38bdf8; font-weight:600;">${countDownloading} descargando</span> | <span style="color:#cbd5e1;">${countQueued} en cola</span> | <span style="color:#34d399;">${countCompleted} completadas</span> (${totalSources} fuentes)`;

    renderDownloadsTable();
    renderLiveDownloadsStrip(cachedDownloads);

    if (selectedDownloadId) {
      refreshInspector(false);
    }
  } catch (e) {
    console.error('Error refreshing downloads:', e);
  }
}

// Multi-selección de descargas
let selectedIds = new Set();

function toggleRowSelection(id, checked) {
  if (checked) selectedIds.add(id);
  else selectedIds.delete(id);
  updateBatchActionBar();
}

function selectAllRows(checked) {
  const checkboxes = document.querySelectorAll('.row-checkbox');
  checkboxes.forEach(cb => {
    cb.checked = checked;
    const id = cb.dataset.id;
    if (checked) selectedIds.add(id);
    else selectedIds.delete(id);
  });
  updateBatchActionBar();
}

function updateBatchActionBar() {
  const pill = document.getElementById('batch-selection-pill');
  const countEl = document.getElementById('batch-selection-count');
  const btnDelete = document.getElementById('btn-action-delete');
  const btnResume = document.getElementById('btn-action-resume');
  const btnPause = document.getElementById('btn-action-pause');
  const btnRestart = document.getElementById('btn-action-restart');
  const thAll = document.getElementById('th-select-all');

  if (selectedIds.size > 0) {
    if (pill) pill.style.display = 'inline-flex';
    if (countEl) countEl.textContent = selectedIds.size;
    if (btnDelete) btnDelete.style.display = 'inline-flex';
    if (btnResume) btnResume.title = `Reanudar ${selectedIds.size} seleccionada(s)`;
    if (btnPause) btnPause.title = `Pausar ${selectedIds.size} seleccionada(s)`;
    if (btnRestart) btnRestart.title = `Reiniciar ${selectedIds.size} seleccionada(s)`;
  } else {
    if (pill) pill.style.display = 'none';
    if (btnDelete) btnDelete.style.display = 'none';
    if (btnResume) btnResume.title = 'Reanudar todas las descargas';
    if (btnPause) btnPause.title = 'Pausar todas las descargas';
    if (btnRestart) btnRestart.title = 'Reiniciar todas las descargas';
    if (thAll) thAll.checked = false;
  }
}

// Corrección automática de mojibake UTF-8 / Latin-1 en strings
function fixMojibake(str) {
  if (!str || typeof str !== 'string') return str;
  if (!str.includes('Ã') && !str.includes('Â')) return str;
  try {
    return decodeURIComponent(escape(str));
  } catch(e) {
    return str
      .replace(/Ã±/g, 'ñ').replace(/Ã‘/g, 'Ñ')
      .replace(/Ã¡/g, 'á').replace(/Ã©/g, 'é').replace(/Ã­/g, 'í').replace(/Ã³/g, 'ó').replace(/Ãº/g, 'ú')
      .replace(/Ã/g, 'Á').replace(/Ã‰/g, 'É').replace(/Ã/g, 'Í').replace(/Ã/g, 'Ó').replace(/Ãš/g, 'Ú')
      .replace(/Ã¼/g, 'ü').replace(/Ãœ/g, 'Ü')
      .replace(/Â¿/g, '¿').replace(/Â¡/g, '¡');
  }
}

function getDownloadStatusBadge(statusLower, isComplete, isTransferringToNas) {
  if (isTransferringToNas) {
    return '<span class="badge badge-completing"><span style="display:inline-block; animation: spin 2s linear infinite;">⏳</span> Finalizando...</span>';
  } else if (statusLower === 'verifying') {
    return '<span class="badge badge-verifying">Verificando</span>';
  } else if (statusLower === 'completed' || isComplete) {
    return '<span class="badge badge-completed"><span style="display: inline-block; width: 6px; height: 6px; border-radius: 50%; background: #22c55e; margin-right: 4px;"></span>Completado</span>';
  } else if (statusLower === 'downloading') {
    return '<span class="badge badge-dl">Descargando</span>';
  } else if (statusLower === 'waiting_for_sources' || statusLower === 'waitingforsources' || statusLower === 'connecting') {
    return '<span class="badge badge-wait">Buscando fuentes</span>';
  } else if (statusLower === 'queued') {
    return '<span class="badge badge-wait">En cola</span>';
  } else if (statusLower === 'paused') {
    return '<span class="badge badge-pause">En pausa</span>';
  } else if (statusLower === 'error') {
    return '<span class="badge badge-error">Error</span>';
  }
  return '<span class="badge badge-wait">En cola</span>';
}

let lastRenderedDownloadIds = '';

function renderDownloadsTable() {
  const tbody = document.getElementById('downloads-list');
  const filtered = cachedDownloads.filter(item => {
    const dl = item.snapshot || item;
    const name = (dl.fileName || dl.file_name || '').toLowerCase();
    const status = (dl.status || '').toLowerCase();
    const total = dl.totalBytes || dl.total_bytes || 0;
    const transferred = dl.receivedBytes || dl.received_bytes || 0;
    const isComplete = (total > 0 && transferred >= total) || status === 'completed';

    // Filtro texto
    if (currentSearchFilter && !name.includes(currentSearchFilter)) return false;

    // Ocultar completadas de la UI si el toggle está activo
    if (hideCompletedInUi && isComplete) return false;

    // Filtro estado (Píldoras)
    if (currentFilterStatus === 'downloading' && (status !== 'downloading' || isComplete || status === 'completing')) return false;
    if (currentFilterStatus === 'queued' && (status !== 'queued' && status !== 'waiting_for_sources' && status !== 'waitingforsources')) return false;
    if (currentFilterStatus === 'completing' && status !== 'completing') return false;
    if (currentFilterStatus === 'completed' && (!isComplete || status === 'completing')) return false;
    if (currentFilterStatus === 'paused' && (status !== 'paused' || isComplete)) return false;

    // Filtro protocolo (Segmented control)
    const isTorrent = name.startsWith('⚡ [torrent]') || ((dl.ed2kHash || '').length === 40 && !item.ed2kHash);
    if (currentFilterProto === 'ed2k' && isTorrent) return false;
    if (currentFilterProto === 'torrent' && !isTorrent) return false;

    return true;
  });

  // Ordenación interactiva
  filtered.sort((a, b) => {
    const dlA = a.snapshot || a;
    const dlB = b.snapshot || b;
    let valA, valB;

    switch (currentSortCol) {
      case 'name':
        valA = (dlA.fileName || dlA.file_name || '').toLowerCase();
        valB = (dlB.fileName || dlB.file_name || '').toLowerCase();
        break;
      case 'size':
        valA = dlA.totalBytes || dlA.total_bytes || 0;
        valB = dlB.totalBytes || dlB.total_bytes || 0;
        break;
      case 'progress': {
        const totA = dlA.totalBytes || dlA.total_bytes || 0;
        const recA = dlA.receivedBytes || dlA.received_bytes || 0;
        valA = totA > 0 ? (recA / totA) : 0;
        const totB = dlB.totalBytes || dlB.total_bytes || 0;
        const recB = dlB.receivedBytes || dlB.received_bytes || 0;
        valB = totB > 0 ? (recB / totB) : 0;
        break;
      }
      case 'speed':
        valA = dlA.downloadRateBps || dlA.download_rate_bps || 0;
        valB = dlB.downloadRateBps || dlB.download_rate_bps || 0;
        break;
      case 'sources':
        valA = dlA.sourceCount || dlA.source_count || 0;
        valB = dlB.sourceCount || dlB.source_count || 0;
        break;
      case 'priority': {
        const pOrder = { high: 3, normal: 2, low: 1, auto: 2 };
        valA = pOrder[(dlA.priority || 'auto').toLowerCase()] || 0;
        valB = pOrder[(dlB.priority || 'auto').toLowerCase()] || 0;
        break;
      }
      case 'status':
        valA = (dlA.status || '').toLowerCase();
        valB = (dlB.status || '').toLowerCase();
        break;
      default:
        valA = 0; valB = 0;
    }

    if (valA < valB) return currentSortDir === 'asc' ? -1 : 1;
    if (valA > valB) return currentSortDir === 'asc' ? 1 : -1;
    return 0;
  });

  if (filtered.length === 0) {
    lastRenderedDownloadIds = '';
    tbody.innerHTML = `<tr><td colspan="8" style="padding: 20px;"><div class="empty-state-box"><img src="/assets/empty-downloads.svg?v=1" alt="" class="empty-state-img"><div class="empty-state-title">Sin transferencias visibles</div><div class="empty-state-sub">No hay descargas que coincidan con los filtros aplicados. Añade un enlace eD2k/Magnet o ajusta los filtros superiores.</div></div></td></tr>`;
    return;
  }

  const currentIdsKey = filtered.map(item => {
    const dl = item.snapshot || item;
    return dl.id || item.id || '';
  }).join('|');

  // Optimización de renderizado: In-place DOM patching si el listado y orden no han cambiado
  if (lastRenderedDownloadIds === currentIdsKey && tbody.children.length === filtered.length) {
    filtered.forEach((item, index) => {
      const row = tbody.children[index];
      if (!row) return;

      const dl = item.snapshot || item;
      const id = dl.id || item.id || '';
      const total = dl.totalBytes || dl.total_bytes || 0;
      const transferred = dl.receivedBytes || dl.received_bytes || 0;
      const progress = total > 0 ? ((transferred / total) * 100).toFixed(1) : (dl.progress ? (dl.progress * 100).toFixed(1) : '0.0');
      const isComplete = progress >= 100.0;
      const speed = dl.downloadRateBps || dl.download_rate_bps || 0;
      const sourceCount = dl.sourceCount || dl.source_count || 0;
      const statusLower = (dl.status || '').toLowerCase();
      const isTransferringToNas = statusLower === 'completing';

      const isSelected = selectedDownloadId === id;
      const isChecked = selectedIds.has(id);

      // Actualizar estado de selección de fila
      if (row.classList.contains('selected-row') !== isSelected) {
        row.classList.toggle('selected-row', isSelected);
      }

      // Celda 0: Checkbox
      const cb = row.querySelector('.row-checkbox');
      if (cb && cb.checked !== isChecked) cb.checked = isChecked;

      // Celda 1: Barra de progreso o cinta de partes
      const pRibbon = row.querySelector('.part-ribbon');
      if (pRibbon) {
        if (isComplete) {
          pRibbon.remove();
        } else if (dl.parts && dl.parts.length > 0) {
          const nextSegs = buildPartRibbonSegments(dl.parts);
          if (pRibbon.innerHTML !== nextSegs) pRibbon.innerHTML = nextSegs;
        }
      }
      const pFill = row.querySelector('.progress-bar-fill');
      if (pFill) {
        if (isComplete) {
          const pBg = row.querySelector('.progress-bar-bg');
          if (pBg) pBg.remove();
        } else {
          pFill.style.width = `${progress}%`;
          const fillClass = `progress-bar-fill ${isTransferringToNas ? 'completing' : (statusLower === 'paused' ? 'paused' : '')}`;
          if (pFill.className !== fillClass) pFill.className = fillClass;
        }
      }

      // Celda 2: Tamaño descargado / total
      if (row.children[2]) {
        row.children[2].innerHTML = `<div><strong>${formatBytes(transferred)}</strong></div><div style="font-size: 0.75rem; color: var(--text-muted);">${formatBytes(total)}</div>`;
      }

      // Celda 3: Porcentaje
      if (row.children[3]) {
        row.children[3].innerHTML = `<strong style="color: ${isComplete ? '#34d399' : '#fff'};">${progress}%</strong>`;
      }

      // Celda 4: Velocidad
      if (row.children[4]) {
        const speedText = speed > 0 ? formatSpeed(speed) : '-';
        if (row.children[4].textContent !== speedText) row.children[4].textContent = speedText;
      }

      // Celda 5: Fuentes
      if (row.children[5]) {
        const srcText = String(sourceCount);
        if (row.children[5].textContent !== srcText) row.children[5].textContent = srcText;
      }

      // Celda 6: Prioridad
      if (row.children[6]) {
        const prioSpan = row.children[6].querySelector('span');
        if (prioSpan) prioSpan.textContent = dl.priority || 'Auto';
      }

      // Celda 7: Badge de estado
      if (row.children[7]) {
        const statusBadge = getDownloadStatusBadge(statusLower, isComplete, isTransferringToNas);
        row.children[7].innerHTML = statusBadge;
      }
    });
    return;
  }

  // Reconstrucción completa del DOM si la lista cambió
  lastRenderedDownloadIds = currentIdsKey;
  let html = '';
  filtered.forEach(item => {
    const dl = item.snapshot || item;
    const rawName = dl.fileName || dl.file_name || 'Archivo';
    const hash = dl.ed2kHash || dl.ed2k_hash || '';
    const id = dl.id || item.id || '';
    const total = dl.totalBytes || dl.total_bytes || 0;
    const transferred = dl.receivedBytes || dl.received_bytes || 0;
    const progress = total > 0 ? ((transferred / total) * 100).toFixed(1) : (dl.progress ? (dl.progress * 100).toFixed(1) : '0.0');
    const isComplete = progress >= 100.0;
    const speed = dl.downloadRateBps || dl.download_rate_bps || 0;
    const sourceCount = dl.sourceCount || dl.source_count || 0;

    const isTorrent = dl.protocol === 'torrent' || item.protocol === 'torrent' || rawName.startsWith('⚡ [Torrent]') || (hash.length === 40 && !item.ed2kHash && (!dl.parts || dl.parts.length === 0));
    const rawClean = rawName.replace(/^⚡ \[Torrent\]\s*/i, '').replace(/^\[Torrent\]\s*/i, '');
    const displayName = fixMojibake(rawClean);

    // Micro-insignia vectorial de protocolo
    const protocolIcon = isTorrent
      ? `<span class="proto-icon torrent" title="BitTorrent"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg></span>`
      : `<span class="proto-icon ed2k" title="eDonkey / Kad"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3"/><line x1="12" y1="3" x2="12" y2="9"/><line x1="4.2" y1="16.5" x2="9.4" y2="13.5"/><line x1="19.8" y1="16.5" x2="14.6" y2="13.5"/></svg></span>`;

    // Categoría con estilo outline sutil
    const category = dl.categoryId || dl.category_id || item.categoryId;
    const categoryBadge = category ? `<span class="badge-category-outline">${category}</span>` : '';

    const statusLower = (dl.status || '').toLowerCase();
    const isTransferringToNas = statusLower === 'completing';
    const statusBadge = getDownloadStatusBadge(statusLower, isComplete, isTransferringToNas);

    // Cinta continua de partes (o barra clásica si aún no hay mapa de partes); se oculta al 100%
    let progressBarHtml = '';
    if (!isComplete) {
      if (dl.parts && dl.parts.length > 0 && !isTransferringToNas) {
        progressBarHtml = `<div class="part-ribbon" title="${progress}% completado">${buildPartRibbonSegments(dl.parts)}</div>`;
      } else {
        progressBarHtml = `
          <div class="progress-bar-bg ${isTransferringToNas ? 'completing-bg' : ''}">
            <div class="progress-bar-fill ${isTransferringToNas ? 'completing' : (statusLower === 'paused' ? 'paused' : '')}" style="width: ${progress}%"></div>
          </div>
        `;
      }
    }

    const isSelected = selectedDownloadId === id;
    const isChecked = selectedIds.has(id);

    // Mini mapa de bloques de partes (visible en modo detallado o en la fila seleccionada)
    let partMapHtml = '';
    if ((!isCompactMode || isSelected) && dl.parts && dl.parts.length > 0 && !isComplete) {
      partMapHtml = '<div class="part-map-grid">';
      dl.parts.slice(0, 80).forEach(p => {
        partMapHtml += `<div class="part-block ${p.state || 'missing'}" title="Parte #${p.partIndex}: ${p.state}"></div>`;
      });
      partMapHtml += '</div>';
    }

    html += `
      <tr data-id="${id}" class="clickable-row ${isSelected ? 'selected-row' : ''}" onclick="selectDownload('${id}')" oncontextmenu="handleRowContextMenu(event, '${id}')">
        <td style="text-align: center;" onclick="event.stopPropagation();">
          <input type="checkbox" class="row-checkbox" data-id="${id}" ${isChecked ? 'checked' : ''} onchange="toggleRowSelection('${id}', this.checked)">
        </td>
        <td class="hover-tip-cell" data-tip-type="name" data-id="${id}">
          <div style="display: flex; align-items: center; gap: 6px; margin-bottom: 2px; flex-wrap: nowrap; min-width: 0;">
            ${protocolIcon}
            ${categoryBadge}
            <div class="dl-row-title" style="font-weight: 600; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${displayName}</div>
          </div>
          <div class="dl-hash-muted" style="font-size: 0.72rem; color: var(--text-muted); font-family: monospace;">${hash}</div>
          ${progressBarHtml}
          ${partMapHtml}
        </td>
        <td class="hover-tip-cell tabular-nums" data-tip-type="progress" data-id="${id}">
          <div><strong>${formatBytes(transferred)}</strong></div>
          <div style="font-size: 0.75rem; color: var(--text-muted);">${formatBytes(total)}</div>
        </td>
        <td class="hover-tip-cell tabular-nums" data-tip-type="progress" data-id="${id}">
          <strong style="color: ${isComplete ? '#34d399' : '#fff'};">${progress}%</strong>
        </td>
        <td class="speed-down hover-tip-cell tabular-nums" data-tip-type="speed" data-id="${id}">${speed > 0 ? formatSpeed(speed) : '-'}</td>
        <td class="hover-tip-cell tabular-nums" data-tip-type="sources" data-id="${id}">${sourceCount}</td>
        <td class="hover-tip-cell" data-tip-type="priority" data-id="${id}"><span style="font-size: 0.8rem; text-transform: capitalize;">${dl.priority || 'Auto'}</span></td>
        <td class="hover-tip-cell" data-tip-type="status" data-id="${id}">${statusBadge}</td>
      </tr>
    `;
  });

  tbody.innerHTML = html;
}

// === SISTEMA DE TOOLTIPS INTELIGENTES (HOVER CARDS) ===
let hoverTipTimer = null;
const tooltipCard = document.getElementById('smart-tooltip-card');

function formatEtaDetail(seconds, speed, remaining) {
  if (seconds && seconds > 0) {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    return h > 0 ? `${h}h ${m}m ${s}s` : `${m}m ${s}s`;
  }
  if (speed > 0 && remaining > 0) {
    const s = Math.floor(remaining / speed);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }
  return 'Calculando...';
}

function showSmartTooltip(targetEl, type, id) {
  if (!tooltipCard) return;

  let html = '';

  if (type === 'host') {
    const hostName = document.getElementById('host-name-label')?.textContent || 'Servidor';
    const hostAddr = document.getElementById('host-addr-label')?.textContent || location.hostname || '127.0.0.1';
    const daemonVer = document.getElementById('statusbar-version-label')?.textContent || '—';
    const osName = lastDiskSpaceInfo?.host?.os || 'linux';
    const ramTotal = lastDiskSpaceInfo?.host?.memTotalBytes ? `${(lastDiskSpaceInfo.host.memTotalBytes / (1024 * 1024 * 1024)).toFixed(1)} GB RAM` : '--';
    const primaryPath = lastDiskSpaceInfo?.incoming?.path || lastDiskSpaceInfo?.dataDir?.path || '-';
    html = `
      <div class="stt-header" style="color: #10b981;">🖥️ Servidor Host (${hostName})</div>
      <div class="stt-desc">Demonio P2P y automatización multimedia PulseCore en ejecución.</div>
      <div class="stt-grid">
        <span class="stt-lbl">Dirección:</span><span class="stt-val">${hostAddr}:${location.port || '4660'}</span>
        <span class="stt-lbl">Sistema / RAM:</span><span class="stt-val">${osName} · ${ramTotal}</span>
        <span class="stt-lbl">Descargas:</span><span class="stt-val">${primaryPath}</span>
        <span class="stt-lbl">Conexión:</span><span class="stt-val" style="color: #22c55e;">WebSocket RPC Activo</span>
        <span class="stt-lbl">Versión:</span><span class="stt-val">PulseCore ${daemonVer}</span>
      </div>
    `;
  } else if (type === 'header-speed') {
    html = `
      <div class="stt-header" style="color: #38bdf8;">⚡ Telemetría Global de Red</div>
      <div class="stt-desc">Ancho de banda agregado en tiempo real entre todas las transferencias.</div>
      <div class="stt-grid">
        <span class="stt-lbl">Tasa Bajada:</span><span class="stt-val" style="color: #38bdf8; font-weight:700;">${document.getElementById('global-down')?.textContent || '0.0 kB/s'}</span>
        <span class="stt-lbl">Tasa Subida:</span><span class="stt-val" style="color: #34d399; font-weight:700;">${document.getElementById('global-up')?.textContent || '0.0 kB/s'}</span>
        <span class="stt-lbl">Pico Bajada / Subida:</span><span class="stt-val">${formatSpeed(sessionPeakDown)} / ${formatSpeed(sessionPeakUp)}</span>
        <span class="stt-lbl">Modo Tortuga:</span><span class="stt-val">${turtleModeActive ? 'Activado (500/100 kB/s)' : 'Desactivado (Sin límites)'}</span>
        <span class="stt-lbl">Monitor en vivo:</span><span class="stt-val" style="color: #38bdf8;">Haz clic para abrir gráfica detallada</span>
      </div>
    `;
  } else if (type === 'header-network') {
    const ed2kName = document.getElementById('cs-name')?.textContent || 'Servidor eD2k';
    const ed2kHigh = document.getElementById('cs-highid-badge')?.textContent || 'HighID';
    const kadNodes = document.getElementById('kad-card-nodes')?.textContent || '40';
    const natText = document.getElementById('nat-pill')?.textContent || 'Abierto (OK)';
    html = `
      <div class="stt-header" style="color: #fbbf24;">🌐 Redes P2P y Enjambres Activos</div>
      <div class="stt-desc">Estado unificado de servidores eDonkey2000, Kademlia DHT y BitTorrent.</div>
      <div class="stt-grid">
        <span class="stt-lbl">Servidor eD2k:</span><span class="stt-val" style="color: #22c55e;">${ed2kName} (${ed2kHigh})</span>
        <span class="stt-lbl">Red Kademlia:</span><span class="stt-val" style="color: #38bdf8;">Conectado (${kadNodes} nodos activos)</span>
        <span class="stt-lbl">BitTorrent:</span><span class="stt-val" style="color: #c084fc;">DHT Activo</span>
        <span class="stt-lbl">Firewall / NAT:</span><span class="stt-val">${natText}</span>
      </div>
    `;
  } else if (type === 'header-storage') {
    const storageLabel = document.getElementById('sidebar-storage-label')?.textContent || 'Almacenamiento';
    const nasInfo = lastDiskSpaceInfo?.nas || {};
    const nfsInfo = nasInfo.nfs || {};
    const freeStr = document.getElementById('sidebar-nas-free')?.textContent || '--';
    const totalStr = document.getElementById('sidebar-nas-total')?.textContent || '';
    const pctStr = document.getElementById('sidebar-nas-pct')?.textContent || '--%';
    const fsKind = nfsInfo.fsKind ? nfsInfo.fsKind.toUpperCase() : 'LOCAL';
    const remoteExport = nfsInfo.remoteExport || nasInfo.path || '-';
    const primaryFree = document.getElementById('rpi-disk-val')?.textContent || '--';
    html = `
      <div class="stt-header" style="color: #f59e0b;">🗄️ Volumen ${storageLabel} (${fsKind})</div>
      <div class="stt-desc">Almacenamiento secundario configurado para la biblioteca multimedia.</div>
      <div class="stt-grid">
        <span class="stt-lbl">Ruta / Origen:</span><span class="stt-val">${remoteExport}</span>
        <span class="stt-lbl">Disponible:</span><span class="stt-val" style="color: #22c55e;">${freeStr} ${totalStr} (${pctStr} usado)</span>
        <span class="stt-lbl">Volumen primario:</span><span class="stt-val">${primaryFree}</span>
      </div>
    `;
  } else {
    const item = cachedDownloads.find(d => {
      const dl = d.snapshot || d;
      return (dl.id || d.id) === id;
    });
    if (!item) return;

    const dl = item.snapshot || item;
    const rawName = dl.fileName || dl.file_name || 'Archivo';
    const hash = dl.ed2kHash || dl.ed2k_hash || '';
    const total = dl.totalBytes || dl.total_bytes || 0;
    const transferred = dl.receivedBytes || dl.received_bytes || 0;
    const remaining = Math.max(0, total - transferred);
    const progress = total > 0 ? ((transferred / total) * 100).toFixed(1) : (dl.progress ? (dl.progress * 100).toFixed(1) : '0.0');
    const isComplete = progress >= 100.0;
    const speed = dl.downloadRateBps || dl.download_rate_bps || 0;
    const sourceCount = dl.sourceCount || dl.source_count || 0;
    const statusLower = (dl.status || '').toLowerCase();
    const isTransferringToNas = statusLower === 'completing';
    const isActivelyWritingToNas = isTransferringToNas && (dl.nasCopyActive === true || dl.nas_copy_active === true);
    const isQueuedForNas = isTransferringToNas && (dl.nasCopyActive === false || dl.nas_copy_active === false);
    const isTorrent = rawName.startsWith('⚡ [Torrent]') || (hash.length === 40 && !item.ed2kHash);
    const displayName = fixMojibake(isTorrent ? rawName.replace(/^⚡ \[Torrent\]\s*/, '') : rawName);
    const category = dl.categoryId || dl.category_id || 'Sin categoría';

    switch (type) {
      case 'status':
        if (isTransferringToNas) {
          if (isQueuedForNas) {
            html = `
              <div class="stt-header" style="color: #fbbf24;">🕒 En Cola de Guardado NAS (Esperando Turno)</div>
              <div class="stt-desc">Descarga completada y verificada al 100%. Para no saturar los discos ni la CPU del Synology DS212j, el gestor transfiere los archivos al NAS de uno en uno en cola FIFO secuencial.</div>
              <div class="stt-grid">
                <span class="stt-lbl">Ubicación actual:</span><span class="stt-val">SSD Local rápido (/tmp/staging)</span>
                <span class="stt-lbl">Destino NAS:</span><span class="stt-val">/home/caraveliko/amule/incoming</span>
                <span class="stt-lbl">Política I/O:</span><span class="stt-val">FIFO secuencial (1 copia a la vez)</span>
                <span class="stt-lbl">Turno:</span><span class="stt-val" style="color: #38bdf8;">Iniciará al terminar la transferencia actual</span>
              </div>
            `;
          } else {
            html = `
              <div class="stt-header" style="color: #f59e0b;">⏳ Copiando a NAS (Transferencia NFS Activa)</div>
              <div class="stt-desc">Descarga completada y verificada al 100%. Transfiriendo archivo final al volumen del Synology DS212j a velocidad segura de ~8 MB/s.</div>
              <div class="stt-grid">
                <span class="stt-lbl">Destino NFS:</span><span class="stt-val">/home/caraveliko/amule/incoming</span>
                <span class="stt-lbl">Protocolo:</span><span class="stt-val">NFSv3 secuencial (seguro)</span>
                <span class="stt-lbl">Protección:</span><span class="stt-val">~8 MB/s (CPU Marvell Kirkwood)</span>
                <span class="stt-lbl">Estado slot:</span><span class="stt-val" style="color: #22c55e;">Ocupando slot exclusivo de escritura NAS</span>
              </div>
            `;
          }
        } else if (statusLower === 'completed' || isComplete) {
        html = `
          <div class="stt-header" style="color: #22c55e;">🟢 Guardado en NAS (Completado)</div>
          <div class="stt-desc">Archivo completamente transferido, verificado y listo en el almacenamiento del NAS.</div>
          <div class="stt-grid">
            <span class="stt-lbl">Tamaño final:</span><span class="stt-val">${formatBytes(total)} (${total.toLocaleString()} B)</span>
            <span class="stt-lbl">Ubicación:</span><span class="stt-val">/volume1/incoming</span>
            <span class="stt-lbl">Hash eD2K:</span><span class="stt-val">${hash || '-'}</span>
            <span class="stt-lbl">Disponibilidad:</span><span class="stt-val" style="color: #22c55e;">Compartiendo en red local</span>
          </div>
        `;
      } else if (statusLower === 'downloading') {
        html = `
          <div class="stt-header" style="color: #38bdf8;">⚡ Descarga Activa (Ranura Inteligente)</div>
          <div class="stt-desc">Esta descarga ocupa una de las 4 ranuras activas permitidas. Transfiriendo bloques a alta velocidad.</div>
          <div class="stt-grid">
            <span class="stt-lbl">Velocidad:</span><span class="stt-val" style="color: #38bdf8; font-weight:700;">${formatSpeed(speed)}</span>
            <span class="stt-lbl">Fuentes activas:</span><span class="stt-val">${sourceCount} conectadas</span>
            <span class="stt-lbl">Progreso:</span><span class="stt-val">${progress}% (${formatBytes(transferred)})</span>
            <span class="stt-lbl">Tiempo restante:</span><span class="stt-val">${formatEtaDetail(dl.etaSeconds, speed, remaining)}</span>
            <span class="stt-lbl">Prioridad:</span><span class="stt-val">${dl.priority || 'Normal'}</span>
          </div>
        `;
      } else if (statusLower === 'queued') {
        html = `
          <div class="stt-header" style="color: #94a3b8;">🕒 En Cola Inteligente (Esperando Ranura)</div>
          <div class="stt-desc">En espera activa. Para no saturar el ancho de banda ni la CPU del Synology DS212j, el gestor inteligente limita a 4 descargas activas simultáneas.</div>
          <div class="stt-grid">
            <span class="stt-lbl">Prioridad:</span><span class="stt-val" style="color: #f1f5f9;">${dl.priority || 'Normal'}</span>
            <span class="stt-lbl">Fuentes en DB:</span><span class="stt-val">${sourceCount} fuentes descubiertas</span>
            <span class="stt-lbl">Progreso actual:</span><span class="stt-val">${progress}%</span>
            <span class="stt-lbl">Activación:</span><span class="stt-val" style="color: #38bdf8;">Automática al liberar ranura</span>
          </div>
        `;
      } else if (statusLower === 'waiting_for_sources' || statusLower === 'waitingforsources') {
        html = `
          <div class="stt-header" style="color: #7dd3fc;">🔍 Buscando Fuentes</div>
          <div class="stt-desc">Ranura activa asignada. Consultando servidores eD2K y nodos Kademlia para conectar pares viables.</div>
          <div class="stt-grid">
            <span class="stt-lbl">Fuentes conocidas:</span><span class="stt-val">${sourceCount}</span>
            <span class="stt-lbl">Canales de búsqueda:</span><span class="stt-val">eD2K TCP/UDP + Kad DHT</span>
            <span class="stt-lbl">Prioridad:</span><span class="stt-val">${dl.priority || 'Auto'}</span>
          </div>
        `;
      } else if (statusLower === 'verifying') {
        html = `
          <div class="stt-header" style="color: #c084fc;">🔍 Verificando Integridad (MD4/AICH)</div>
          <div class="stt-desc">Comprobando árbol de hashes y sumas de verificación de todas las partes antes de la copia al NAS.</div>
        `;
      } else if (statusLower === 'paused') {
        html = `
          <div class="stt-header" style="color: #d29922;">⏸ En Pausa</div>
          <div class="stt-desc">La descarga ha sido pausada manualmente por el usuario. No consume ranuras de la cola ni ancho de banda.</div>
        `;
      }
      break;

    case 'name':
      html = `
        <div class="stt-header">📄 ${displayName}</div>
        <div class="stt-desc">Metadatos del archivo en la red ${isTorrent ? 'BitTorrent' : 'eDonkey2000'}</div>
        <div class="stt-grid">
          <span class="stt-lbl">Protocolo:</span><span class="stt-val">${isTorrent ? 'BitTorrent' : 'eDonkey2000 (eD2K)'}</span>
          <span class="stt-lbl">Hash eD2K:</span><span class="stt-val">${hash || 'No disponible'}</span>
          <span class="stt-lbl">Tamaño total:</span><span class="stt-val">${total.toLocaleString()} bytes (${formatBytes(total)})</span>
          <span class="stt-lbl">Categoría:</span><span class="stt-val">${category}</span>
          <span class="stt-lbl">Descargado:</span><span class="stt-val">${formatBytes(transferred)} (${progress}%)</span>
        </div>
      `;
      break;

    case 'progress':
      const parts = dl.parts || [];
      const verifiedParts = parts.filter(p => p.state === 'completed' || p.state === 'verified').length;
      html = `
        <div class="stt-header" style="color: #38bdf8;">📊 Progreso: ${progress}%</div>
        <div class="stt-desc">Estado de bloques y segmentos en disco</div>
        <div class="stt-grid">
          <span class="stt-lbl">Descargado:</span><span class="stt-val">${formatBytes(transferred)}</span>
          <span class="stt-lbl">Tamaño total:</span><span class="stt-val">${formatBytes(total)}</span>
          <span class="stt-lbl">Restante:</span><span class="stt-val">${formatBytes(remaining)}</span>
          <span class="stt-lbl">Partes completas:</span><span class="stt-val">${parts.length > 0 ? `${verifiedParts} / ${parts.length} partes (~9.28 MB)` : 'Calculando...'}</span>
          <span class="stt-lbl">Estado final:</span><span class="stt-val">${isComplete ? 'Completado' : 'En transferencia'}</span>
        </div>
      `;
      break;

    case 'sources':
      html = `
        <div class="stt-header" style="color: #38bdf8;">👥 Fuentes y Pares P2P</div>
        <div class="stt-desc">Información del enjambre y conexiones simultáneas</div>
        <div class="stt-grid">
          <span class="stt-lbl">Fuentes detectadas:</span><span class="stt-val">${sourceCount}</span>
          <span class="stt-lbl">Límite por archivo:</span><span class="stt-val">${dl.maxSources || 300} fuentes máx.</span>
          <span class="stt-lbl">Redes de búsqueda:</span><span class="stt-val">Servidores eD2K + Kademlia DHT</span>
          <span class="stt-lbl">Intercambio SX:</span><span class="stt-val">Activo con clientes eMule</span>
        </div>
      `;
      break;

    case 'speed':
      html = `
        <div class="stt-header" style="color: #38bdf8;">⚡ Telemetría de Velocidad</div>
        <div class="stt-desc">Rendimiento en tiempo real y estimación de tiempo</div>
        <div class="stt-grid">
          <span class="stt-lbl">Velocidad bajada:</span><span class="stt-val" style="color: #38bdf8; font-weight:700;">${formatSpeed(speed)}</span>
          <span class="stt-lbl">Subido al enjambre:</span><span class="stt-val">${formatBytes(dl.uploadedBytes || 0)}</span>
          <span class="stt-lbl">Tiempo restante (ETA):</span><span class="stt-val">${formatEtaDetail(dl.etaSeconds, speed, remaining)}</span>
          <span class="stt-lbl">Faltan transferir:</span><span class="stt-val">${formatBytes(remaining)}</span>
        </div>
      `;
      break;

    case 'priority':
      html = `
        <div class="stt-header" style="color: #f59e0b;">⭐ Prioridad y Cola Inteligente</div>
        <div class="stt-desc">Determina el acceso prioritario a las 4 ranuras activas simultáneas.</div>
        <div class="stt-grid">
          <span class="stt-lbl">Prioridad asignada:</span><span class="stt-val" style="text-transform: capitalize; font-weight: 700;">${dl.priority || 'Normal'}</span>
          <span class="stt-lbl">Progreso:</span><span class="stt-val">${progress}% (mayor progreso = mayor prioridad)</span>
          <span class="stt-lbl">Criterio:</span><span class="stt-val">Heurística: Prioridad + Progreso + Fuentes</span>
        </div>
      `;
      break;

    default:
      return;
  }
}

  const rect = targetEl.getBoundingClientRect();
  const card = tooltipCard;
  card.innerHTML = html;
  card.style.display = 'block';

  // Posicionamiento inteligente boundary-aware
  const cardWidth = card.offsetWidth || 340;
  const cardHeight = card.offsetHeight || 160;

  let left = rect.left;
  let top = rect.bottom + 8;

  if (left + cardWidth > window.innerWidth - 16) {
    left = window.innerWidth - cardWidth - 16;
  }
  if (left < 16) left = 16;

  if (top + cardHeight > window.innerHeight - 16) {
    top = Math.max(16, rect.top - cardHeight - 8);
  }

  card.style.left = `${left}px`;
  card.style.top = `${top}px`;
  card.style.opacity = '1';
}

function hideSmartTooltip() {
  if (!tooltipCard) return;
  tooltipCard.style.opacity = '0';
  setTimeout(() => {
    if (tooltipCard.style.opacity === '0') {
      tooltipCard.style.display = 'none';
    }
  }, 120);
}

// Delegación global de eventos hover para header, tabla y componentes
document.addEventListener('mouseover', (e) => {
  const cell = e.target.closest('.hover-tip-cell');
  if (!cell) return;
  const type = cell.dataset.tipType;
  const id = cell.dataset.id;
  if (!type) return;

  clearTimeout(hoverTipTimer);
  hoverTipTimer = setTimeout(() => {
    showSmartTooltip(cell, type, id);
  }, 100);
});

document.addEventListener('mouseout', (e) => {
  const cell = e.target.closest('.hover-tip-cell');
  if (cell) {
    clearTimeout(hoverTipTimer);
    hideSmartTooltip();
  }
});

