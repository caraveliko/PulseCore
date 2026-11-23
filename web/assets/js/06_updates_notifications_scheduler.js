// Pulse Core WebUI — Actualizaciones del sistema, centro de notificaciones push y planificador adaptativo.
// Extraído de web/index.html.

// === GESTIÓN DE ACTUALIZACIONES DEL SISTEMA (HÍBRIDA: OFICIAL + BINARIO LOCAL) ===
const updateUiState = {
  snapshot: { status: 'idle' },
  environment: null,
  lastCheckAt: null,
  selectedFile: null,
  isUploading: false,
  isRestarting: false,
  officialPollTimer: null,
  silentCheckScheduled: false,
};
window._updateModalLocked = false;

function formatUpdateMb(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0.0 MB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}

window._updateEnvFromHello = function(helloRes) {
  if (!helloRes) return;
  if (!updateUiState.environment) {
    updateUiState.environment = {};
  }
  if (helloRes.daemonVersion) updateUiState.environment.currentVersion = helloRes.daemonVersion;
  if (helloRes.platformTriple) updateUiState.environment.platformTriple = helloRes.platformTriple;
  if (typeof helloRes.managedBySystemd === 'boolean') {
    updateUiState.environment.managedBySystemd = helloRes.managedBySystemd;
  }
  if (typeof helloRes.runningInDocker === 'boolean') {
    updateUiState.environment.runningInDocker = helloRes.runningInDocker;
  }
  if (helloRes.workdirPath) {
    updateUiState.environment.workdirPath = helloRes.workdirPath;
  }
  renderUpdateEnvironment();
};

function applyDockerOptionsNotices(isDocker, workdirPath) {
  const show = Boolean(isDocker);
  const toggleDisplay = (id, displayVal) => {
    const el = document.getElementById(id);
    if (el) el.style.display = displayVal;
  };

  const badge = document.getElementById('pref-docker-env-badge');
  if (badge) {
    badge.style.display = show ? 'inline-flex' : 'none';
    if (show) {
      badge.textContent = workdirPath ? `🐳 Docker · ${workdirPath}` : '🐳 Docker';
    }
  }

  toggleDisplay('docker-folders-notice', show ? 'block' : 'none');
  toggleDisplay('docker-nfs-notice', show ? 'block' : 'none');
  toggleDisplay('docker-network-notice', show ? 'block' : 'none');
  toggleDisplay('docker-updates-notice', show ? 'block' : 'none');
  toggleDisplay('btn-upload-update-bin', show ? 'none' : 'inline-flex');
}
window._applyDockerOptionsNotices = applyDockerOptionsNotices;

function showUpdateModalFeedback(kind, message) {
  const box = document.getElementById('modal-upd-feedback');
  if (!box) return;
  if (!message) {
    box.style.display = 'none';
    box.textContent = '';
    return;
  }
  const palette = {
    error: { bg: 'rgba(239, 68, 68, 0.14)', border: 'rgba(239, 68, 68, 0.45)', color: '#fca5a5' },
    success: { bg: 'rgba(34, 197, 94, 0.14)', border: 'rgba(34, 197, 94, 0.45)', color: '#86efac' },
    info: { bg: 'rgba(56, 189, 248, 0.12)', border: 'rgba(56, 189, 248, 0.38)', color: '#bae6fd' },
  }[kind || 'info'];
  box.style.display = 'block';
  box.style.background = palette.bg;
  box.style.border = `1px solid ${palette.border}`;
  box.style.color = palette.color;
  box.textContent = message;
}

function setUpdateModalLocked(locked) {
  window._updateModalLocked = !!locked;
  const closeBtn = document.getElementById('btn-modal-update-close');
  const closeX = document.getElementById('btn-modal-update-close-x');
  if (closeBtn) closeBtn.disabled = !!locked;
  if (closeX) closeX.disabled = !!locked;
}

function renderUpdateEnvironment() {
  const env = updateUiState.environment;
  if (!env) return;

  const verLabel = env.currentVersion ? `v${env.currentVersion}` : '—';
  const triple = env.platformTriple || 'desconocida';
  const isDocker = env.runningInDocker === true;
  const runtimeText = isDocker
    ? 'Contenedor Docker'
    : (env.managedBySystemd ? 'systemd (reinicio automático)' : 'Proceso independiente');
  const runtimeShort = isDocker
    ? 'Docker'
    : (env.managedBySystemd ? 'systemd' : 'Independiente');
  const canWrite = typeof env.canSelfUpdate === 'boolean' ? env.canSelfUpdate : null;

  if (typeof env.runningInDocker === 'boolean') {
    applyDockerOptionsNotices(isDocker, env.workdirPath);
  }
  const applyBtn = document.getElementById('btn-apply-update');
  if (applyBtn && isDocker) {
    applyBtn.style.display = 'none';
  }

  const verBadge = document.getElementById('update-current-ver-badge');
  if (verBadge && env.currentVersion) verBadge.textContent = `Versión instalada: ${verLabel}`;
  const brandVer = document.getElementById('brand-version-pill');
  if (brandVer && env.currentVersion) brandVer.textContent = verLabel;
  const statusVer = document.getElementById('statusbar-version-label');
  if (statusVer && env.currentVersion) statusVer.textContent = verLabel;

  const archEl = document.getElementById('update-arch-val');
  if (archEl) archEl.textContent = triple;

  const chanVal = (env.channel === 'beta') ? 'beta' : 'stable';
  const chanSelect = document.getElementById('update-channel-select');
  if (chanSelect) chanSelect.value = chanVal;
  const modalChanSelect = document.getElementById('modal-update-channel-select');
  if (modalChanSelect) modalChanSelect.value = chanVal;

  const autoUpdateOn = env.autoUpdateEnabled === true;
  const autoSelect = document.getElementById('update-auto-mode-select');
  if (autoSelect) autoSelect.value = autoUpdateOn ? 'auto' : 'manual';
  const modalAutoSwitch = document.getElementById('modal-update-auto-switch');
  if (modalAutoSwitch && modalAutoSwitch.checked !== autoUpdateOn) {
    modalAutoSwitch.checked = autoUpdateOn;
  }

  const modeEl = document.getElementById('update-runtime-mode-val');
  if (modeEl) modeEl.textContent = runtimeText;

  const writeEl = document.getElementById('update-writable-val');
  if (writeEl) {
    if (isDocker) {
      writeEl.textContent = 'Gestionado por imagen Docker';
      writeEl.style.color = '#60a5fa';
    } else if (canWrite !== null) {
      writeEl.textContent = canWrite ? 'Correcto (escribible)' : 'Solo lectura';
      writeEl.style.color = canWrite ? '#4ade80' : '#fbbf24';
    }
  }

  const ttVer = document.getElementById('header-update-tt-ver');
  const ttArch = document.getElementById('header-update-tt-arch');
  const ttMode = document.getElementById('header-update-tt-mode');
  if (ttVer) ttVer.textContent = verLabel;
  if (ttArch) ttArch.textContent = triple;
  if (ttMode) ttMode.textContent = runtimeShort;

  const mVer = document.getElementById('modal-upd-env-ver');
  const mArch = document.getElementById('modal-upd-env-arch');
  const mMode = document.getElementById('modal-upd-env-mode');
  const mWrite = document.getElementById('modal-upd-env-write');
  const mWarn = document.getElementById('modal-upd-readonly-warn');
  const mExe = document.getElementById('modal-upd-exe-path');

  if (mVer) mVer.textContent = `Versión: ${verLabel}`;
  if (mArch) mArch.textContent = `Plataforma: ${triple}`;
  if (mMode) mMode.textContent = `Entorno: ${runtimeShort}`;
  if (mWrite && canWrite !== null) {
    mWrite.textContent = canWrite ? 'Escritura: OK' : 'Escritura: Solo lectura';
    mWrite.style.background = canWrite ? 'rgba(16, 185, 129, 0.14)' : 'rgba(245, 158, 11, 0.16)';
    mWrite.style.color = canWrite ? '#34d399' : '#fbbf24';
    mWrite.style.borderColor = canWrite ? 'rgba(16, 185, 129, 0.35)' : 'rgba(245, 158, 11, 0.45)';
  }
  if (mWarn) {
    mWarn.style.display = canWrite === false ? 'block' : 'none';
  }
  if (mExe && env.executablePath) {
    mExe.textContent = env.executablePath;
  }
  const rbSection = document.getElementById('modal-upd-rollback-section');
  if (rbSection) {
    rbSection.style.display = env.hasRollbackBackup === true ? 'block' : 'none';
  }
}

async function changeUpdateChannel(channel) {
  const normalized = (channel === 'beta') ? 'beta' : 'stable';
  const chanSelect = document.getElementById('update-channel-select');
  if (chanSelect) chanSelect.value = normalized;
  const modalChanSelect = document.getElementById('modal-update-channel-select');
  if (modalChanSelect) modalChanSelect.value = normalized;

  try {
    await client.call('settings.set', { updateChannel: normalized });
    if (updateUiState.environment) {
      updateUiState.environment.channel = normalized;
    }
    const label = normalized === 'beta' ? 'Beta (prelanzamientos)' : 'Estable';
    addLog('info', `Canal de actualizaciones cambiado a: ${label}`);
    showUpdateModalFeedback('info', `Canal cambiado a ${label}. Comprobando versiones en GitHub...`);
    await checkSystemUpdate(true, false);
  } catch (err) {
    showUpdateModalFeedback('error', 'Error cambiando canal de actualización: ' + (err.message || err));
  }
}
window.changeUpdateChannel = changeUpdateChannel;

async function changeAutoUpdateMode(enabled) {
  const isAuto = Boolean(enabled);
  const autoSelect = document.getElementById('update-auto-mode-select');
  if (autoSelect) autoSelect.value = isAuto ? 'auto' : 'manual';
  const modalAutoSwitch = document.getElementById('modal-update-auto-switch');
  if (modalAutoSwitch) modalAutoSwitch.checked = isAuto;

  try {
    await client.call('settings.set', { autoUpdateEnabled: isAuto });
    if (!updateUiState.environment) updateUiState.environment = {};
    updateUiState.environment.autoUpdateEnabled = isAuto;
    renderUpdateEnvironment();

    const msg = isAuto
      ? '⚡ Auto-actualización activada: el servidor instalará nuevas versiones oficiales automáticamente.'
      : '🖐️ Modo manual activado: se avisará antes de descargar o instalar actualizaciones.';
    addLog('info', msg);
    showUpdateModalFeedback(isAuto ? 'success' : 'info', msg);
    if (typeof showToast === 'function') {
      showToast(isAuto ? 'Auto-actualización activada' : 'Actualizaciones en modo manual', 'info', isAuto ? '⚡' : '🖐️');
    }

    if (isAuto && updateUiState.snapshot && updateUiState.snapshot.status === 'update_available') {
      confirmAndApplyOfficialUpdate();
    }
  } catch (err) {
    showUpdateModalFeedback('error', 'Error guardando modo de actualización: ' + (err.message || err));
  }
}
window.changeAutoUpdateMode = changeAutoUpdateMode;

function requestRollbackConfirm() {
  const box = document.getElementById('modal-upd-rollback-confirm');
  if (box) box.style.display = 'flex';
}
window.requestRollbackConfirm = requestRollbackConfirm;

function cancelRollbackConfirm() {
  const box = document.getElementById('modal-upd-rollback-confirm');
  if (box) box.style.display = 'none';
}
window.cancelRollbackConfirm = cancelRollbackConfirm;

async function confirmAndRollbackUpdate() {
  cancelRollbackConfirm();
  showUpdateModalFeedback(null, '');
  const rbBtn = document.getElementById('btn-modal-rollback-trigger');
  if (rbBtn) rbBtn.disabled = true;
  setUpdateModalLocked(true);

  try {
    addLog('warn', 'Solicitando reversión atómica hacia la copia .old...');
    const res = await client.call('system.rollbackUpdate', { autoRestart: true });
    showUpdateModalFeedback('success', (res && res.message) || 'Versión anterior restaurada.');
    if (res && res.restarting) {
      waitForDaemonRestartAndReload();
    } else {
      setUpdateModalLocked(false);
      if (rbBtn) rbBtn.disabled = false;
      refreshSystemUpdateStatus();
    }
  } catch (err) {
    setUpdateModalLocked(false);
    if (rbBtn) rbBtn.disabled = false;
    showUpdateModalFeedback('error', 'No se pudo revertir la versión: ' + (err.message || err));
  }
}
window.confirmAndRollbackUpdate = confirmAndRollbackUpdate;

function applyUpdateStateToUI(data) {
  if (!data) return;
  if (data.environment) {
    updateUiState.environment = Object.assign({}, updateUiState.environment || {}, data.environment);
  }
  const snap = data.status ? data : updateUiState.snapshot;
  updateUiState.snapshot = snap;
  renderUpdateEnvironment();

  const headerBtn = document.getElementById('btn-header-update');
  const headerLabel = document.getElementById('header-update-label');
  const ttBadge = document.getElementById('header-update-tt-badge');
  const ttDesc = document.getElementById('header-update-tt-desc');

  const pill = document.getElementById('update-status-pill');
  const cardTitle = document.getElementById('update-card-title');
  const cardDesc = document.getElementById('update-card-desc');
  const applyBtn = document.getElementById('btn-apply-update');
  const lastCheckEl = document.getElementById('update-last-check-val');
  const optProg = document.getElementById('update-progress-container');
  const optProgBar = document.getElementById('update-progress-bar');
  const optProgPct = document.getElementById('update-progress-pct');
  const optProgLbl = document.getElementById('update-progress-label');

  const mBadge = document.getElementById('modal-update-status-badge');
  const mOffTitle = document.getElementById('modal-upd-official-title');
  const mOffDesc = document.getElementById('modal-upd-official-desc');
  const mOffBtn = document.getElementById('btn-modal-apply-official');
  const mOffProg = document.getElementById('modal-upd-official-progress');
  const mOffProgBar = document.getElementById('modal-upd-official-prog-bar');
  const mOffProgPct = document.getElementById('modal-upd-official-prog-pct');
  const mOffProgLbl = document.getElementById('modal-upd-official-prog-label');

  if (updateUiState.lastCheckAt && lastCheckEl) {
    lastCheckEl.textContent = updateUiState.lastCheckAt;
  }

  if (headerBtn) {
    headerBtn.classList.remove('update-idle', 'update-ok', 'update-available', 'update-busy', 'update-error');
  }

  const status = snap.status || 'idle';
  if (status === 'update_available') {
    const ver = snap.version || 'nueva';
    const sizeMb = formatUpdateMb(snap.downloadSize || 0);
    if (headerBtn) headerBtn.classList.add('update-available');
    if (headerLabel) headerLabel.textContent = `v${ver}`;
    if (ttBadge) {
      ttBadge.textContent = `Nueva v${ver}`;
      ttBadge.style.background = 'rgba(245, 158, 11, 0.22)';
      ttBadge.style.color = '#fbbf24';
    }
    if (ttDesc) ttDesc.textContent = `Nueva versión oficial v${ver} (${sizeMb}) lista para descargar e instalar.`;

    if (pill) {
      pill.textContent = `★ Nueva versión v${ver}`;
      pill.style.background = 'rgba(245, 158, 11, 0.2)';
      pill.style.color = '#fbbf24';
      pill.style.borderColor = 'rgba(245, 158, 11, 0.4)';
    }
    if (cardTitle) cardTitle.textContent = `Nueva versión disponible: v${ver}`;
    if (cardDesc) {
      cardDesc.innerHTML = `<strong>Novedades:</strong> ${escapeHtml(snap.summary || 'Mejoras de rendimiento y estabilidad.')}<br><span style="color: var(--text-muted); font-size: 0.74rem;">Tamaño: ~${sizeMb} | Firma Ed25519 verificada</span>`;
    }
    if (applyBtn) applyBtn.style.display = 'inline-flex';
    if (optProg) optProg.style.display = 'none';

    if (mBadge) {
      mBadge.textContent = `Disponible v${ver}`;
      mBadge.style.background = 'rgba(245, 158, 11, 0.2)';
      mBadge.style.color = '#fbbf24';
      mBadge.style.borderColor = 'rgba(245, 158, 11, 0.45)';
    }
    if (mOffTitle) mOffTitle.textContent = `Nueva versión oficial disponible: v${ver} (${sizeMb})`;
    if (mOffDesc) {
      mOffDesc.innerHTML = `${escapeHtml(snap.summary || 'Mejoras de rendimiento y estabilidad.')}`;
    }
    if (mOffBtn) {
      mOffBtn.style.display = 'inline-flex';
      mOffBtn.disabled = false;
      mOffBtn.textContent = `🚀 Descargar e instalar v${ver}`;
    }
    if (mOffProg) mOffProg.style.display = 'none';
  } else if (status === 'up_to_date') {
    const cur = snap.currentVersion || (updateUiState.environment && updateUiState.environment.currentVersion) || 'actual';
    if (headerBtn) headerBtn.classList.add('update-ok');
    if (headerLabel) headerLabel.textContent = '';
    if (ttBadge) {
      ttBadge.textContent = 'Al día';
      ttBadge.style.background = 'rgba(34, 197, 94, 0.2)';
      ttBadge.style.color = '#4ade80';
    }
    if (ttDesc) ttDesc.textContent = `Tu instalación (v${cur}) coincide con la última publicación oficial.`;

    if (pill) {
      pill.textContent = '● Sistema al día';
      pill.style.background = 'rgba(34, 197, 94, 0.2)';
      pill.style.color = '#4ade80';
      pill.style.borderColor = 'rgba(34, 197, 94, 0.4)';
    }
    if (cardTitle) cardTitle.textContent = 'Tu Pulse Core está actualizado';
    if (cardDesc) cardDesc.textContent = `Tienes instalada la versión más reciente (v${cur}). También puedes instalar un binario local compilado.`;
    if (applyBtn) applyBtn.style.display = 'none';
    if (optProg) optProg.style.display = 'none';

    if (mBadge) {
      mBadge.textContent = 'Al día';
      mBadge.style.background = 'rgba(34, 197, 94, 0.18)';
      mBadge.style.color = '#4ade80';
      mBadge.style.borderColor = 'rgba(34, 197, 94, 0.4)';
    }
    if (mOffTitle) mOffTitle.textContent = `Sistema al día (v${cur})`;
    if (mOffDesc) mOffDesc.textContent = 'No hay versiones oficiales más recientes en el canal configurado.';
    if (mOffBtn) mOffBtn.style.display = 'none';
    if (mOffProg) mOffProg.style.display = 'none';
  } else if (status === 'checking' || status === 'downloading') {
    const isDl = status === 'downloading';
    const dlBytes = snap.downloadedBytes || 0;
    const totalBytes = snap.totalBytes || 0;
    const pct = totalBytes > 0 ? Math.min(100, Math.round((dlBytes * 100) / totalBytes)) : 0;

    if (headerBtn) headerBtn.classList.add('update-busy');
    if (headerLabel) headerLabel.textContent = isDl ? `${pct}%` : '';
    if (ttBadge) {
      ttBadge.textContent = isDl ? `Descargando ${pct}%` : 'Consultando';
      ttBadge.style.background = 'rgba(94, 106, 210, 0.25)';
      ttBadge.style.color = '#c7d2fe';
    }
    if (ttDesc) ttDesc.textContent = isDl ? `Descargando actualización: ${formatUpdateMb(dlBytes)} de ${formatUpdateMb(totalBytes)}` : 'Consultando manifiesto oficial…';

    if (pill) {
      pill.textContent = isDl ? `⏳ Descargando ${pct}%` : '● Comprobando...';
      pill.style.background = 'rgba(94, 106, 210, 0.2)';
      pill.style.color = '#a5b4fc';
      pill.style.borderColor = 'rgba(94, 106, 210, 0.4)';
    }
    if (isDl) {
      if (optProg) optProg.style.display = 'block';
      if (optProgBar) optProgBar.style.width = `${pct}%`;
      if (optProgPct) optProgPct.textContent = `${pct}%`;
      if (optProgLbl) optProgLbl.textContent = `Descargando (${formatUpdateMb(dlBytes)} / ${formatUpdateMb(totalBytes)})…`;

      if (mOffProg) mOffProg.style.display = 'block';
      if (mOffProgBar) mOffProgBar.style.width = `${pct}%`;
      if (mOffProgPct) mOffProgPct.textContent = `${pct}%`;
      if (mOffProgLbl) mOffProgLbl.textContent = `Descargando y verificando (${formatUpdateMb(dlBytes)} / ${formatUpdateMb(totalBytes)})…`;
    }
  } else if (status === 'ready_to_install') {
    if (headerBtn) headerBtn.classList.add('update-ok');
    if (headerLabel) headerLabel.textContent = '';
    if (ttBadge) {
      ttBadge.textContent = 'Listo';
      ttBadge.style.background = 'rgba(16, 185, 129, 0.22)';
      ttBadge.style.color = '#34d399';
    }
    if (mBadge) {
      mBadge.textContent = 'Preparado';
      mBadge.style.background = 'rgba(16, 185, 129, 0.2)';
      mBadge.style.color = '#34d399';
    }
  } else if (status === 'error') {
    const msg = snap.message || 'No se pudo completar la consulta de actualizaciones.';
    if (headerBtn) headerBtn.classList.add('update-error');
    if (headerLabel) headerLabel.textContent = '';
    if (ttBadge) {
      ttBadge.textContent = 'Aviso';
      ttBadge.style.background = 'rgba(239, 68, 68, 0.2)';
      ttBadge.style.color = '#fca5a5';
    }
    if (ttDesc) ttDesc.textContent = msg;

    if (pill) {
      pill.textContent = '⚠️ Error de consulta';
      pill.style.background = 'rgba(239, 68, 68, 0.2)';
      pill.style.color = '#f87171';
      pill.style.borderColor = 'rgba(239, 68, 68, 0.4)';
    }
    if (cardTitle) cardTitle.textContent = 'Sin manifiesto oficial o error de red';
    if (cardDesc) cardDesc.textContent = msg;
    if (applyBtn) applyBtn.style.display = 'none';
    if (optProg) optProg.style.display = 'none';

    if (mBadge) {
      mBadge.textContent = 'Aviso';
      mBadge.style.background = 'rgba(239, 68, 68, 0.18)';
      mBadge.style.color = '#fca5a5';
      mBadge.style.borderColor = 'rgba(239, 68, 68, 0.4)';
    }
    if (mOffTitle) mOffTitle.textContent = 'No se pudo obtener lanzamiento oficial';
    if (mOffDesc) mOffDesc.textContent = msg;
    if (mOffBtn) mOffBtn.style.display = 'none';
    if (mOffProg) mOffProg.style.display = 'none';
  } else {
    if (headerBtn) headerBtn.classList.add('update-idle');
    if (headerLabel) headerLabel.textContent = '';
  }
}

async function refreshSystemUpdateStatus() {
  try {
    const res = await client.call('system.getUpdateStatus', {});
    applyUpdateStateToUI(res);
    return res;
  } catch (e) {
    console.debug('Error al consultar estado de actualizaciones:', e);
    return null;
  }
}

function openUpdateModal() {
  showUpdateModalFeedback(null, '');
  cancelOfficialUpdateConfirm();
  const restartPanel = document.getElementById('modal-upd-restarting-panel');
  if (restartPanel && !updateUiState.isRestarting) {
    restartPanel.style.display = 'none';
  }
  openModal('modal-system-update');
  refreshSystemUpdateStatus();
}
window.openUpdateModal = openUpdateModal;

function scheduleSilentUpdateCheckIfEnabled(statusRes) {
  if (updateUiState.silentCheckScheduled) return;
  const env = (statusRes && statusRes.environment) || updateUiState.environment;
  const st = statusRes && statusRes.status;
  if (!env || !env.autoCheckEnabled || (st && st !== 'idle')) return;
  updateUiState.silentCheckScheduled = true;
  setTimeout(() => {
    if (document.hidden) return;
    checkSystemUpdate(false, true).catch(() => {});
  }, 3000);
}

async function checkSystemUpdate(force = true, silent = false) {
  const btnSettings = document.getElementById('btn-check-updates');
  const btnModal = document.getElementById('btn-modal-check-update');
  if (!silent) {
    showUpdateModalFeedback(null, '');
  }
  if (btnSettings) {
    btnSettings.disabled = true;
    btnSettings.innerHTML = '<span>⏳ Comprobando...</span>';
  }
  if (btnModal) {
    btnModal.disabled = true;
    btnModal.textContent = '⏳ Comprobando…';
  }

  try {
    const res = await client.call('system.checkUpdate', { force: !!force });
    updateUiState.lastCheckAt = new Date().toLocaleTimeString();
    applyUpdateStateToUI(res);
    if (res && res.status === 'update_available') {
      addLog('info', `Actualización oficial detectada: versión v${res.version} disponible.`);
    }
  } catch (err) {
    console.error('Error al comprobar actualización:', err);
    if (!silent) {
      showUpdateModalFeedback('error', 'Error al comprobar actualizaciones: ' + (err.message || err));
    }
  } finally {
    if (btnSettings) {
      btnSettings.disabled = false;
      btnSettings.innerHTML = '<span>🔍 Comprobar Ahora</span>';
    }
    if (btnModal) {
      btnModal.disabled = false;
      btnModal.textContent = '🔍 Comprobar ahora';
    }
  }
}
window.checkSystemUpdate = checkSystemUpdate;

function requestOfficialUpdateConfirm() {
  const box = document.getElementById('modal-upd-confirm-box');
  if (box) box.style.display = 'block';
}
window.requestOfficialUpdateConfirm = requestOfficialUpdateConfirm;

function cancelOfficialUpdateConfirm() {
  const box = document.getElementById('modal-upd-confirm-box');
  if (box) box.style.display = 'none';
}
window.cancelOfficialUpdateConfirm = cancelOfficialUpdateConfirm;

function stopOfficialDownloadPolling() {
  if (updateUiState.officialPollTimer) {
    clearInterval(updateUiState.officialPollTimer);
    updateUiState.officialPollTimer = null;
  }
}

function startOfficialDownloadPolling() {
  stopOfficialDownloadPolling();
  updateUiState.officialPollTimer = setInterval(() => {
    refreshSystemUpdateStatus().catch(() => {});
  }, 350);
}

async function confirmAndApplyOfficialUpdate() {
  cancelOfficialUpdateConfirm();
  showUpdateModalFeedback(null, '');
  const applyModalBtn = document.getElementById('btn-modal-apply-official');
  const mOffProg = document.getElementById('modal-upd-official-progress');
  const mOffProgLbl = document.getElementById('modal-upd-official-prog-label');

  if (applyModalBtn) applyModalBtn.disabled = true;
  if (mOffProg) mOffProg.style.display = 'block';
  if (mOffProgLbl) mOffProgLbl.textContent = 'Iniciando descarga verificada…';
  setUpdateModalLocked(true);
  startOfficialDownloadPolling();

  try {
    addLog('info', 'Iniciando descarga en streaming y reemplazo atómico de actualización...');
    const res = await client.call('system.applyUpdate', { autoRestart: true });
    stopOfficialDownloadPolling();
    await refreshSystemUpdateStatus();

    if (res && res.stage === 'restarting') {
      showUpdateModalFeedback('success', res.message || 'Actualización aplicada. Reiniciando Pulse Core…');
      addLog('success', 'Actualización aplicada con éxito. Esperando reinicio ordenado...');
      waitForDaemonRestartAndReload();
    } else {
      setUpdateModalLocked(false);
      showUpdateModalFeedback('success', (res && res.message) || 'Actualización preparada.');
    }
  } catch (err) {
    stopOfficialDownloadPolling();
    setUpdateModalLocked(false);
    console.error('Error aplicando actualización oficial:', err);
    showUpdateModalFeedback('error', 'No se pudo completar la actualización: ' + (err.message || err));
    if (applyModalBtn) applyModalBtn.disabled = false;
  }
}
window.confirmAndApplyOfficialUpdate = confirmAndApplyOfficialUpdate;

function setSelectedLocalBinary(file) {
  if (!file) return;
  updateUiState.selectedFile = file;
  const infoBox = document.getElementById('local-file-selected-info');
  const nameEl = document.getElementById('local-file-name');
  const sizeEl = document.getElementById('local-file-size');
  const uploadBtn = document.getElementById('btn-modal-upload-local');

  if (nameEl) nameEl.textContent = file.name;
  if (sizeEl) sizeEl.textContent = `(${formatUpdateMb(file.size)})`;
  if (infoBox) infoBox.style.display = 'flex';
  if (uploadBtn) uploadBtn.disabled = false;
  showUpdateModalFeedback(null, '');
}

function clearSelectedLocalBinary() {
  updateUiState.selectedFile = null;
  const inputEl = document.getElementById('input-local-update-file');
  const infoBox = document.getElementById('local-file-selected-info');
  const uploadBtn = document.getElementById('btn-modal-upload-local');
  if (inputEl) inputEl.value = '';
  if (infoBox) infoBox.style.display = 'none';
  if (uploadBtn) uploadBtn.disabled = true;
}
window.clearSelectedLocalBinary = clearSelectedLocalBinary;

function setupLocalUpdateDropzone() {
  const dz = document.getElementById('update-dropzone');
  const inputEl = document.getElementById('input-local-update-file');
  if (!dz || !inputEl || dz.dataset.bound === '1') return;
  dz.dataset.bound = '1';

  dz.addEventListener('click', () => inputEl.click());
  dz.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      inputEl.click();
    }
  });
  inputEl.addEventListener('change', () => {
    if (inputEl.files && inputEl.files.length > 0) {
      setSelectedLocalBinary(inputEl.files[0]);
    }
  });
  ['dragenter', 'dragover'].forEach((evt) => {
    dz.addEventListener(evt, (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      dz.classList.add('drag-over');
    });
  });
  ['dragleave', 'drop'].forEach((evt) => {
    dz.addEventListener(evt, (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      dz.classList.remove('drag-over');
    });
  });
  dz.addEventListener('drop', (ev) => {
    const files = ev.dataTransfer && ev.dataTransfer.files;
    if (files && files.length > 0) {
      setSelectedLocalBinary(files[0]);
    }
  });
}

function uploadLocalUpdateBinary() {
  const file = updateUiState.selectedFile;
  if (!file || updateUiState.isUploading) return;

  const shaInput = document.getElementById('input-local-update-sha256');
  const expectedSha = shaInput ? shaInput.value.trim() : '';
  if (expectedSha && !/^[0-9a-fA-F]{64}$/.test(expectedSha)) {
    showUpdateModalFeedback('error', 'La suma SHA-256 opcional debe tener exactamente 64 caracteres hexadecimales.');
    return;
  }

  const maxBytes = (updateUiState.environment && updateUiState.environment.maxUploadBytes) || (64 * 1024 * 1024);
  if (file.size === 0) {
    showUpdateModalFeedback('error', 'El archivo seleccionado está vacío.');
    return;
  }
  if (file.size > maxBytes) {
    showUpdateModalFeedback('error', `El archivo supera el límite permitido de ${formatUpdateMb(maxBytes)}.`);
    return;
  }

  const autoRestartChk = document.getElementById('chk-local-auto-restart');
  const autoRestart = autoRestartChk ? !!autoRestartChk.checked : true;
  const uploadBtn = document.getElementById('btn-modal-upload-local');
  const progBox = document.getElementById('modal-upd-upload-progress');
  const progBar = document.getElementById('modal-upd-upload-prog-bar');
  const progPct = document.getElementById('modal-upd-upload-prog-pct');
  const progLbl = document.getElementById('modal-upd-upload-prog-label');

  updateUiState.isUploading = true;
  setUpdateModalLocked(true);
  showUpdateModalFeedback(null, '');
  if (uploadBtn) {
    uploadBtn.disabled = true;
    uploadBtn.textContent = '⏳ Subiendo e instalando…';
  }
  if (progBox) progBox.style.display = 'block';
  if (progBar) progBar.style.width = '0%';
  if (progPct) progPct.textContent = '0%';
  if (progLbl) progLbl.textContent = `Enviando ${file.name} (${formatUpdateMb(file.size)})…`;

  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/api/system/upload-update', true);
  xhr.setRequestHeader('Content-Type', 'application/octet-stream');
  if (client.token) {
    xhr.setRequestHeader('Authorization', `Bearer ${client.token}`);
  }
  const asciiName = file.name.replace(/[^\x20-\x7E]/g, '_');
  if (asciiName) {
    xhr.setRequestHeader('X-Update-Filename', asciiName);
  }
  if (expectedSha) {
    xhr.setRequestHeader('X-Update-Sha256', expectedSha);
  }
  xhr.setRequestHeader('X-Auto-Restart', autoRestart ? 'true' : 'false');

  xhr.upload.onprogress = (ev) => {
    if (!ev.lengthComputable || ev.total <= 0) return;
    const pct = Math.min(100, Math.round((ev.loaded * 100) / ev.total));
    if (progBar) progBar.style.width = `${pct}%`;
    if (progPct) progPct.textContent = `${pct}%`;
    if (progLbl) {
      progLbl.textContent = pct >= 100
        ? 'Validando formato ELF/PE y sustituyendo binario en servidor…'
        : `Subiendo binario (${formatUpdateMb(ev.loaded)} / ${formatUpdateMb(ev.total)})…`;
    }
  };

  xhr.onload = () => {
    updateUiState.isUploading = false;
    let parsed = null;
    try {
      parsed = JSON.parse(xhr.responseText || '{}');
    } catch (err) { console.debug('[PulseCore] respuesta no JSON:', err); }

    if (xhr.status >= 200 && xhr.status < 300 && parsed && parsed.ok) {
      if (progBar) progBar.style.width = '100%';
      if (progPct) progPct.textContent = '100%';
      if (progLbl) progLbl.textContent = 'Binario verificado y aplicado en disco.';
      addLog('success', `Binario local instalado (${parsed.label}, SHA-256 ${String(parsed.sha256 || '').slice(0, 12)}…).`);

      if (parsed.restarting) {
        showUpdateModalFeedback('success', parsed.message || 'Binario instalado. Reiniciando Pulse Core…');
        waitForDaemonRestartAndReload();
      } else {
        setUpdateModalLocked(false);
        if (uploadBtn) {
          uploadBtn.disabled = false;
          uploadBtn.textContent = '⬆️ Subir e instalar binario';
        }
        showUpdateModalFeedback('success', parsed.message || 'Binario instalado correctamente. Se aplicará al próximo reinicio.');
        refreshSystemUpdateStatus();
      }
    } else {
      setUpdateModalLocked(false);
      if (uploadBtn) {
        uploadBtn.disabled = false;
        uploadBtn.textContent = '⬆️ Subir e instalar binario';
      }
      const errMsg = (parsed && parsed.error) || `El servidor devolvió HTTP ${xhr.status}`;
      showUpdateModalFeedback('error', errMsg);
      addLog('error', `Error subiendo binario local: ${errMsg}`);
    }
  };

  xhr.onerror = () => {
    updateUiState.isUploading = false;
    setUpdateModalLocked(false);
    if (uploadBtn) {
      uploadBtn.disabled = false;
      uploadBtn.textContent = '⬆️ Subir e instalar binario';
    }
    showUpdateModalFeedback('error', 'Error de conexión al subir el binario local al servidor.');
  };

  xhr.send(file);
}
window.uploadLocalUpdateBinary = uploadLocalUpdateBinary;

async function waitForDaemonRestartAndReload() {
  updateUiState.isRestarting = true;
  setUpdateModalLocked(true);
  const panel = document.getElementById('modal-upd-restarting-panel');
  const titleEl = document.getElementById('modal-upd-restart-title');
  const subEl = document.getElementById('modal-upd-restart-sub');
  if (panel) panel.style.display = 'block';
  if (titleEl) titleEl.textContent = 'Reiniciando Pulse Core de forma ordenada…';
  if (subEl) subEl.textContent = 'Esperando a que el nuevo proceso vuelva a responder en /health…';

  await new Promise((r) => setTimeout(r, 1200));

  const maxAttempts = 45;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    if (subEl) {
      subEl.textContent = `Comprobando disponibilidad de /health (${attempt}/${maxAttempts}s)…`;
    }
    try {
      const resp = await fetch('/health', { cache: 'no-store' });
      if (resp.ok) {
        const health = await resp.json().catch(() => ({}));
        const ver = health && health.daemonVersion ? ` v${health.daemonVersion}` : '';
        if (titleEl) titleEl.textContent = `¡Pulse Core${ver} en línea de nuevo!`;
        if (subEl) subEl.textContent = 'Recargando la interfaz web automáticamente…';
        setTimeout(() => {
          window.location.reload();
        }, 500);
        return;
      }
    } catch (err) { console.debug('[PulseCore] espera de reinicio:', err); }
    await new Promise((r) => setTimeout(r, 1000));
  }

  updateUiState.isRestarting = false;
  setUpdateModalLocked(false);
  showUpdateModalFeedback('error', 'Pulse Core tardó más de 45s en responder tras reiniciar. Recarga la página manualmente cuando el servicio esté activo.');
}

setupLocalUpdateDropzone();

// ==========================================
// CENTRO DE NOTIFICACIONES Y CANALES PUSH
// ==========================================
let notifItems = [];
let notifUnreadCount = 0;
let notifActiveFilter = 'all';
let notifLastServerStatus = '';
window._notifConfigLoaded = false;

function escapeHtmlNotif(str) {
  return escapeHtml(str);
}

function notifIconFor(item) {
  if (!item) return '🔔';
  switch (item.kind) {
    case 'downloadCompleted': return '✅';
    case 'downloadFailed': return '❌';
    case 'mediaReady': return '🎬';
    case 'mediaGrabbed': return '🧲';
    case 'updateAvailable': return '🚀';
    case 'diskLow': return '💾';
    case 'serverDisconnected': return '📡';
    case 'systemWarning': return '⚠️';
    default:
      if (item.severity === 'success') return '✅';
      if (item.severity === 'error') return '🚨';
      if (item.severity === 'warning') return '⚠️';
      return '🔔';
  }
}

function formatNotifRelativeTime(unixMs) {
  if (!unixMs) return '';
  const diffSec = Math.max(0, Math.floor((Date.now() - unixMs) / 1000));
  if (diffSec < 60) return 'Ahora';
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `Hace ${diffMin} min`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `Hace ${diffHr} h`;
  try {
    return new Date(unixMs).toLocaleDateString('es-ES', { day: '2-digit', month: '2-digit' });
  } catch (_) {
    return '';
  }
}

function isBrowserNotifPrefEnabled() {
  return localStorage.getItem('pulse_browser_notifs') === '1';
}

function isSoundNotifPrefEnabled() {
  return localStorage.getItem('pulse_sound_notifs') === '1';
}

function syncNotificationLocalPrefControls() {
  const browserOn = isBrowserNotifPrefEnabled();
  const soundOn = isSoundNotifPrefEnabled();
  const qBrowser = document.getElementById('notif-quick-browser');
  const pBrowser = document.getElementById('notif-pref-browser');
  const qSound = document.getElementById('notif-quick-sound');
  const pSound = document.getElementById('notif-pref-sound');
  if (qBrowser) qBrowser.checked = browserOn;
  if (pBrowser) pBrowser.checked = browserOn;
  if (qSound) qSound.checked = soundOn;
  if (pSound) pSound.checked = soundOn;

  const statusEl = document.getElementById('notif-browser-status-text');
  const reqBtn = document.getElementById('btn-notif-request-browser');
  if (statusEl) {
    if (!('Notification' in window)) {
      statusEl.textContent = 'Este navegador o contexto HTTP no admite Web Notifications API (requiere HTTPS o localhost).';
      if (reqBtn) reqBtn.disabled = true;
    } else if (Notification.permission === 'granted') {
      statusEl.textContent = browserOn
        ? '✅ Permiso concedido y avisos del navegador activos.'
        : 'ℹ️ Permiso concedido en el navegador (desactivado en esta sesión).';
      if (reqBtn) reqBtn.style.display = 'none';
    } else if (Notification.permission === 'denied') {
      statusEl.textContent = '⚠️ Permiso bloqueado en la configuración del navegador para este sitio.';
      if (reqBtn) reqBtn.style.display = 'inline-flex';
    } else {
      statusEl.textContent = 'Permiso pendiente: pulsa «Solicitar permiso» para recibir avisos aunque la pestaña esté en segundo plano.';
      if (reqBtn) reqBtn.style.display = 'inline-flex';
    }
  }
}

async function enableBrowserNotifications() {
  if (!('Notification' in window)) {
    showToast('Este navegador o dirección HTTP no permite notificaciones nativas.', 'warning', '⚠️');
    syncNotificationLocalPrefControls();
    return false;
  }
  if (Notification.permission === 'granted') {
    localStorage.setItem('pulse_browser_notifs', '1');
    syncNotificationLocalPrefControls();
    return true;
  }
  if (Notification.permission === 'denied') {
    showToast('El navegador bloqueó el permiso. Habilítalo en el candado de la barra de direcciones.', 'warning', '⚠️');
    syncNotificationLocalPrefControls();
    return false;
  }
  try {
    const perm = await Notification.requestPermission();
    if (perm === 'granted') {
      localStorage.setItem('pulse_browser_notifs', '1');
      showToast('Notificaciones del navegador activadas.', 'success', '🔔');
      syncNotificationLocalPrefControls();
      return true;
    }
    localStorage.setItem('pulse_browser_notifs', '0');
    syncNotificationLocalPrefControls();
    return false;
  } catch (e) {
    console.warn('Error solicitando permiso de notificaciones:', e);
    return false;
  }
}

async function toggleQuickBrowserNotifications(checked) {
  if (checked) {
    const ok = await enableBrowserNotifications();
    if (!ok) {
      localStorage.setItem('pulse_browser_notifs', '0');
      syncNotificationLocalPrefControls();
    }
  } else {
    localStorage.setItem('pulse_browser_notifs', '0');
    syncNotificationLocalPrefControls();
  }
}

function toggleQuickSoundNotifications(checked) {
  localStorage.setItem('pulse_sound_notifs', checked ? '1' : '0');
  syncNotificationLocalPrefControls();
  if (checked) playNotificationChime('info');
}

function playNotificationChime(severity = 'info') {
  try {
    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (!AudioCtx) return;
    const ctx = new AudioCtx();
    const now = ctx.currentTime;
    const freqs = severity === 'error' || severity === 'warning' ? [523.25, 440] : [587.33, 880];
    freqs.forEach((freq, idx) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(freq, now + idx * 0.09);
      gain.gain.setValueAtTime(0.0001, now + idx * 0.09);
      gain.gain.exponentialRampToValueAtTime(0.055, now + idx * 0.09 + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + idx * 0.09 + 0.22);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(now + idx * 0.09);
      osc.stop(now + idx * 0.09 + 0.24);
    });
    setTimeout(() => { ctx.close().catch(() => {}); }, 600);
  } catch (err) { console.debug('[PulseCore] audio no disponible:', err); }
}

function updateNotificationBadges() {
  const btn = document.getElementById('btn-header-notifications');
  const badge = document.getElementById('header-notif-badge');
  const popLabel = document.getElementById('popover-unread-label');
  const count = Math.max(0, Number(notifUnreadCount) || 0);
  if (badge) {
    if (count > 0) {
      badge.textContent = count > 99 ? '99+' : String(count);
      badge.style.display = 'inline-block';
    } else {
      badge.style.display = 'none';
    }
  }
  if (btn) btn.classList.toggle('has-unread', count > 0);
  if (popLabel) popLabel.textContent = count === 1 ? '1 nueva' : `${count} nuevas`;
}

function renderNotificationsPopover() {
  const listEl = document.getElementById('notifications-popover-list');
  if (!listEl) return;

  const filtered = notifItems.filter(item => {
    if (notifActiveFilter === 'unread') return !item.isRead;
    if (notifActiveFilter !== 'all') return item.category === notifActiveFilter;
    return true;
  });

  if (filtered.length === 0) {
    listEl.innerHTML = `<div style="padding: 26px 14px; text-align: center; color: var(--text-muted); font-size: 0.78rem;">No hay notificaciones en esta vista.</div>`;
    return;
  }

  listEl.innerHTML = filtered.map(item => {
    const icon = notifIconFor(item);
    const title = escapeHtmlNotif(item.title || 'Notificación');
    const body = escapeHtmlNotif(item.body || '');
    const rel = escapeHtmlNotif(formatNotifRelativeTime(item.createdAtUnixMs));
    const cls = item.isRead ? 'notif-item' : 'notif-item unread';
    return `<div class="${cls}" onclick="markNotificationRead(${Number(item.id)})">
      <div class="notif-item-icon">${icon}</div>
      <div class="notif-item-content">
        <div class="notif-item-top">
          <div class="notif-item-title">${title}</div>
          <div class="notif-item-time">${rel}</div>
        </div>
        <div class="notif-item-body">${body}</div>
      </div>
    </div>`;
  }).join('');
}

async function refreshNotifications() {
  try {
    const res = await client.call('notifications.list', { limit: 60 });
    if (res) {
      notifItems = Array.isArray(res.notifications) ? res.notifications : [];
      notifUnreadCount = Number(res.unreadCount || 0);
      updateNotificationBadges();
      renderNotificationsPopover();
    }
  } catch (e) {
    console.debug('Error cargando notificaciones:', e);
  }
}

function toggleNotificationsPopover(ev) {
  if (ev) ev.stopPropagation();
  const pop = document.getElementById('notifications-popover');
  const btn = document.getElementById('btn-header-notifications');
  if (!pop) return;
  const opening = !pop.classList.contains('open');
  pop.classList.toggle('open', opening);
  if (btn) btn.classList.toggle('open', opening);
  if (opening) {
    syncNotificationLocalPrefControls();
    refreshNotifications();
  }
}

function closeNotificationsPopover() {
  const pop = document.getElementById('notifications-popover');
  const btn = document.getElementById('btn-header-notifications');
  if (pop) pop.classList.remove('open');
  if (btn) btn.classList.remove('open');
}

function setNotificationFilter(filterKey) {
  notifActiveFilter = filterKey || 'all';
  document.querySelectorAll('#notif-popover-filters .notif-filter-chip').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.notifFilter === notifActiveFilter);
  });
  renderNotificationsPopover();
}

async function markNotificationRead(id) {
  if (!id) return;
  try {
    const res = await client.call('notifications.markRead', { id });
    const target = notifItems.find(n => Number(n.id) === Number(id));
    if (target) target.isRead = true;
    if (res && typeof res.unreadCount === 'number') {
      notifUnreadCount = res.unreadCount;
    }
    updateNotificationBadges();
    renderNotificationsPopover();
  } catch (e) {
    console.warn('Error marcando notificación como leída:', e);
  }
}

async function markAllNotificationsRead() {
  try {
    const res = await client.call('notifications.markRead', { all: true });
    notifItems.forEach(n => { n.isRead = true; });
    notifUnreadCount = res && typeof res.unreadCount === 'number' ? res.unreadCount : 0;
    updateNotificationBadges();
    renderNotificationsPopover();
  } catch (e) {
    showToast('Error marcando notificaciones: ' + (e.message || e), 'error', '❌');
  }
}

async function clearNotificationsHistory() {
  try {
    await client.call('notifications.clear', {});
    notifItems = [];
    notifUnreadCount = 0;
    updateNotificationBadges();
    renderNotificationsPopover();
    showToast('Historial de notificaciones limpiado.', 'info', '🧹');
  } catch (e) {
    showToast('Error limpiando historial: ' + (e.message || e), 'error', '❌');
  }
}

function openNotificationSettingsTab() {
  closeNotificationsPopover();
  const settingsNavBtn = document.querySelector('.nav-btn[data-tab="tab-settings"]');
  if (settingsNavBtn) settingsNavBtn.click();
  switchOptionsTab('opt-notifications');
}

function formatChannelStatusText(statusObj) {
  if (!statusObj || !statusObj.lastAttemptUnixMs) {
    return { text: 'Sin pruebas recientes', color: 'var(--text-muted)' };
  }
  if (statusObj.lastError) {
    return { text: `❌ ${statusObj.lastError}`, color: '#fca5a5' };
  }
  const ms = statusObj.lastLatencyMs != null ? ` (${statusObj.lastLatencyMs} ms)` : '';
  return { text: `✅ Último envío correcto${ms}`, color: '#4ade80' };
}

function applyNotificationConfigViewToUi(view) {
  if (!view) return;
  window._notifConfigLoaded = true;
  const setChk = (id, v) => { const el = document.getElementById(id); if (el) el.checked = !!v; };
  const setVal = (id, v) => { const el = document.getElementById(id); if (el) el.value = v ?? ''; };
  const pad2 = n => String(Math.max(0, Number(n) || 0)).padStart(2, '0');

  setChk('notif-cfg-enabled', view.enabled);
  setChk('notif-cfg-external-enabled', view.externalEnabled);
  setVal('notif-cfg-disk-gb', view.diskLowThresholdGb ?? 5);

  const ev = view.events || {};
  setChk('notif-ev-download-completed', ev.downloadCompleted);
  setChk('notif-ev-download-failed', ev.downloadFailed);
  setChk('notif-ev-media-ready', ev.mediaReady);
  setChk('notif-ev-media-grabbed', ev.mediaGrabbed);
  setChk('notif-ev-update-available', ev.updateAvailable);
  setChk('notif-ev-disk-low', ev.diskLow);
  setChk('notif-ev-server-disconnected', ev.serverDisconnected);
  setChk('notif-ev-system-warning', ev.systemWarning);

  const qh = view.quietHours || {};
  setChk('notif-qh-enabled', qh.enabled);
  setVal('notif-qh-start', `${pad2(qh.startHour ?? 23)}:${pad2(qh.startMinute ?? 0)}`);
  setVal('notif-qh-end', `${pad2(qh.endHour ?? 8)}:${pad2(qh.endMinute ?? 0)}`);
  setChk('notif-qh-allow-critical', qh.allowCritical !== false);

  const tg = view.telegram || {};
  setChk('notif-tg-enabled', tg.enabled);
  setVal('notif-tg-chat-id', tg.chatId || '');
  setVal('notif-tg-token', '');
  const tgRow = document.getElementById('notif-tg-token-row');
  if (tgRow) tgRow.style.display = tg.hasBotToken ? 'flex' : 'none';

  const ntfy = view.ntfy || {};
  setChk('notif-ntfy-enabled', ntfy.enabled);
  setVal('notif-ntfy-url', ntfy.url || '');
  setVal('notif-ntfy-token', '');
  const ntfyRow = document.getElementById('notif-ntfy-token-row');
  if (ntfyRow) ntfyRow.style.display = ntfy.hasToken ? 'flex' : 'none';

  const disc = view.discord || {};
  setChk('notif-discord-enabled', disc.enabled);
  setVal('notif-discord-url', '');
  const discRow = document.getElementById('notif-discord-url-row');
  if (discRow) discRow.style.display = disc.hasWebhookUrl ? 'flex' : 'none';

  const wh = view.webhook || {};
  setChk('notif-webhook-enabled', wh.enabled);
  setVal('notif-webhook-url', wh.url || '');
  setVal('notif-webhook-secret', '');
  const whRow = document.getElementById('notif-webhook-secret-row');
  if (whRow) whRow.style.display = wh.hasSecret ? 'flex' : 'none';

  const statuses = view.channelStatuses || {};
  const applySt = (elId, key) => {
    const el = document.getElementById(elId);
    if (!el) return;
    const info = formatChannelStatusText(statuses[key]);
    el.textContent = info.text;
    el.style.color = info.color;
  };
  applySt('notif-tg-status', 'telegram');
  applySt('notif-ntfy-status', 'ntfy');
  applySt('notif-discord-status', 'discord');
  applySt('notif-webhook-status', 'webhook');
  syncNotificationLocalPrefControls();
}

async function loadNotificationConfig() {
  try {
    const view = await client.call('notifications.getConfig', {});
    applyNotificationConfigViewToUi(view);
  } catch (e) {
    console.debug('Error cargando configuración de notificaciones:', e);
  }
}

function parseTimeInput(id, fallbackHour, fallbackMin) {
  const el = document.getElementById(id);
  if (!el || !el.value || !el.value.includes(':')) return [fallbackHour, fallbackMin];
  const [hStr, mStr] = el.value.split(':');
  const h = Math.max(0, Math.min(23, parseInt(hStr, 10) || 0));
  const m = Math.max(0, Math.min(59, parseInt(mStr, 10) || 0));
  return [h, m];
}

function buildNotificationPatchFromUi() {
  const getChk = id => !!document.getElementById(id)?.checked;
  const getStr = id => (document.getElementById(id)?.value || '').trim();
  const diskGb = Math.max(1, Math.min(1000, parseInt(getStr('notif-cfg-disk-gb'), 10) || 5));
  const [sh, sm] = parseTimeInput('notif-qh-start', 23, 0);
  const [eh, em] = parseTimeInput('notif-qh-end', 8, 0);

  const patch = {
    enabled: getChk('notif-cfg-enabled'),
    externalEnabled: getChk('notif-cfg-external-enabled'),
    diskLowThresholdGb: diskGb,
    events: {
      downloadCompleted: getChk('notif-ev-download-completed'),
      downloadFailed: getChk('notif-ev-download-failed'),
      mediaReady: getChk('notif-ev-media-ready'),
      mediaGrabbed: getChk('notif-ev-media-grabbed'),
      serverDisconnected: getChk('notif-ev-server-disconnected'),
      updateAvailable: getChk('notif-ev-update-available'),
      diskLow: getChk('notif-ev-disk-low'),
      systemWarning: getChk('notif-ev-system-warning'),
    },
    quietHours: {
      enabled: getChk('notif-qh-enabled'),
      startHour: sh,
      startMinute: sm,
      endHour: eh,
      endMinute: em,
      allowCritical: getChk('notif-qh-allow-critical'),
    },
    telegram: {
      enabled: getChk('notif-tg-enabled'),
      chatId: getStr('notif-tg-chat-id'),
    },
    ntfy: {
      enabled: getChk('notif-ntfy-enabled'),
      url: getStr('notif-ntfy-url'),
    },
    discord: {
      enabled: getChk('notif-discord-enabled'),
    },
    webhook: {
      enabled: getChk('notif-webhook-enabled'),
      url: getStr('notif-webhook-url'),
    },
  };

  const tgTok = getStr('notif-tg-token');
  if (tgTok) patch.telegram.botToken = tgTok;
  const ntfyTok = getStr('notif-ntfy-token');
  if (ntfyTok) patch.ntfy.token = ntfyTok;
  const discUrl = getStr('notif-discord-url');
  if (discUrl) patch.discord.webhookUrl = discUrl;
  const whSec = getStr('notif-webhook-secret');
  if (whSec) patch.webhook.secret = whSec;

  return patch;
}

async function saveNotificationConfig(silent = false) {
  const btn = document.getElementById('btn-save-notifications');
  if (btn && !silent) {
    btn.disabled = true;
    btn.textContent = 'Guardando...';
  }
  try {
    const patch = buildNotificationPatchFromUi();
    const view = await client.call('notifications.setConfig', patch);
    applyNotificationConfigViewToUi(view);
    if (!silent) {
      showToast('Configuración de notificaciones guardada.', 'success', '🔔');
    }
  } catch (e) {
    const msg = e.message || JSON.stringify(e);
    if (!silent) {
      showToast('Error guardando notificaciones: ' + msg, 'error', '❌');
    } else {
      throw e;
    }
  } finally {
    if (btn && !silent) {
      btn.disabled = false;
      btn.innerHTML = '<span>💾 Guardar Notificaciones</span>';
    }
  }
}

async function clearNotificationSecret(channel) {
  const patch = {};
  if (channel === 'telegram') patch.telegram = { clearBotToken: true };
  else if (channel === 'ntfy') patch.ntfy = { clearToken: true };
  else if (channel === 'discord') patch.discord = { clearWebhookUrl: true };
  else if (channel === 'webhook') patch.webhook = { clearSecret: true };
  else return;

  try {
    const view = await client.call('notifications.setConfig', patch);
    applyNotificationConfigViewToUi(view);
    showToast(`Credencial de ${channel} eliminada.`, 'info', '🧹');
  } catch (e) {
    showToast('Error eliminando credencial: ' + (e.message || e), 'error', '❌');
  }
}

async function testNotificationChannel(channel, btnEl) {
  const originalHtml = btnEl ? btnEl.innerHTML : '';
  if (btnEl) {
    btnEl.disabled = true;
    btnEl.textContent = 'Probando...';
  }
  try {
    if (channel !== 'webui' && window._notifConfigLoaded) {
      await saveNotificationConfig(true);
    }
    const res = await client.call('notifications.test', { channel });
    if (channel !== 'webui') {
      await loadNotificationConfig();
    }
    if (res && res.ok) {
      if (channel !== 'webui') {
        showToast(`Prueba enviada por ${channel} (${res.latencyMs || 0} ms).`, 'success', '✅');
      }
    } else {
      const errMsg = (res && res.error) || 'Fallo al enviar prueba';
      showToast(`Error probando ${channel}: ${errMsg}`, 'error', '❌');
    }
  } catch (e) {
    showToast(`Error probando ${channel}: ` + (e.message || e), 'error', '❌');
  } finally {
    if (btnEl) {
      btnEl.disabled = false;
      btnEl.innerHTML = originalHtml;
    }
  }
}

document.addEventListener('click', (ev) => {
  const container = document.getElementById('notif-header-container');
  if (container && !container.contains(ev.target)) {
    closeNotificationsPopover();
  }
});
document.addEventListener('keydown', (ev) => {
  if (ev.key === 'Escape') closeNotificationsPopover();
});

let schedulerSeconds = 0;

client.on('connected', () => {
  const host = window.location.hostname || '192.168.1.166';
  if (document.getElementById('arr-host-input')) {
    document.getElementById('arr-host-input').value = host;
  }

  setupTableSorting();
  setupLocalUpdateDropzone();
  updateTurtleModeUI();
  addLog('success', 'Conectado al daemon Pulse Core vía WebSocket.');

  try { loadPreferences(); } catch (e) { console.error('Error loadPreferences:', e); }
  try { refreshKad(); } catch (e) { console.error('Error refreshKad:', e); }
  try { refreshServers(); } catch (e) { console.error('Error refreshServers:', e); }
  try { refreshDownloads(); } catch (e) { console.error('Error refreshDownloads:', e); }
  try { loadMediaLibrary(); } catch (e) { console.error('Error loadMediaLibrary:', e); }
  try { refreshStorage(); } catch (e) { console.error('Error refreshStorage:', e); }
  try { refreshBandwidthLimits(); } catch (e) { console.error('Error refreshBandwidthLimits:', e); }
  try { refreshNotifications(); } catch (e) { console.error('Error refreshNotifications:', e); }
  try { loadNotificationConfig(); } catch (e) { console.error('Error loadNotificationConfig:', e); }
  try {
    refreshSystemUpdateStatus().then(scheduleSilentUpdateCheckIfEnabled);
  } catch (e) {
    console.error('Error refreshSystemUpdateStatus:', e);
  }

  // Comprobación de aviso legal y primer arranque
  setTimeout(async () => {
    const accepted = await checkLegalDisclaimer();
    if (accepted) {
      checkFirstRunSetup();
    }
  }, 500);

  // === PLANIFICADOR MAESTRO ADAPTATIVO (Smart Adaptive Scheduler) ===
  // Un único temporizador sincronizado a 1s que adapta la frecuencia de sondeo
  // según la visibilidad del documento y la pestaña activa, reduciendo uso de CPU y red.
  schedulerSeconds = 0;
  if (window._masterSchedulerInterval) {
    clearInterval(window._masterSchedulerInterval);
  }
  window._masterSchedulerInterval = setInterval(() => {
    if (document.hidden) return; // Pausa completa en reposo/minimizada
    schedulerSeconds++;

    const activeTabEl = document.querySelector('.nav-btn.active');
    const activeTab = activeTabEl ? activeTabEl.dataset.tab : 'tab-media';

    // 1. Cada 2 segundos: Refrescar descargas y velocidades globales
    if (schedulerSeconds % 2 === 0) {
      refreshDownloads().catch(e => console.error('Scheduler refreshDownloads error:', e));
    }

    // 2. Cada 5 segundos: Refrescar contenido específico de la pestaña activa
    if (schedulerSeconds % 5 === 0) {
      if (activeTab === 'tab-media' && !mediaLibraryLoaded && !mediaLibraryLoading) {
        loadMediaLibrary().catch(e => console.error('Scheduler loadMediaLibrary error:', e));
      } else if (activeTab === 'tab-kad') {
        refreshKad().catch(e => console.error('Scheduler refreshKad error:', e));
        refreshKadContacts().catch(e => console.error('Scheduler refreshKadContacts error:', e));
      } else if (activeTab === 'tab-servers') {
        refreshServers().catch(e => console.error('Scheduler refreshServers error:', e));
      } else if (activeTab === 'tab-shares') {
        refreshShares().catch(e => console.error('Scheduler refreshShares error:', e));
      } else if (activeTab === 'tab-logs') {
        refreshSystemLogs().catch(e => console.error('Scheduler refreshSystemLogs error:', e));
      }
    }

    // 3. Cada 10 segundos: Salud general de red (Kad y eD2k status) si no estamos en esas pestañas
    if (schedulerSeconds % 10 === 0) {
      if (activeTab !== 'tab-kad') {
        refreshKad().catch(e => console.error('Scheduler Kad background error:', e));
      }
      if (activeTab !== 'tab-servers') {
        refreshServers().catch(e => console.error('Scheduler Servers background error:', e));
      }
    }

    // 4. Cada 30 segundos: Almacenamiento (disco y NAS)
    if (schedulerSeconds % 30 === 0) {
      refreshStorage().catch(e => console.error('Scheduler Storage error:', e));
    }
  }, 1000);
});

document.addEventListener('visibilitychange', () => {
  if (!document.hidden && client.ws && client.ws.readyState === WebSocket.OPEN) {
    // Despertar inmediato al volver a la pestaña
    schedulerSeconds = 0;
    const activeTabEl = document.querySelector('.nav-btn.active');
    const activeTab = activeTabEl ? activeTabEl.dataset.tab : 'tab-media';
    try { refreshDownloads(); } catch (e) { console.warn('[PulseCore] error en refresco periódico:', e); }
    try { refreshKad(); } catch (e) { console.warn('[PulseCore] error en refresco periódico:', e); }
    try { refreshServers(); } catch (e) { console.warn('[PulseCore] error en refresco periódico:', e); }
    try { refreshStorage(); } catch (e) { console.warn('[PulseCore] error en refresco periódico:', e); }
    if (activeTab === 'tab-media' || !mediaLibraryLoaded) {
      try { loadMediaLibrary(); } catch (e) { console.warn('[PulseCore] error en refresco periódico:', e); }
    }
  }
});

// Enrutamiento de eventos push del daemon al Registro en Vivo y Centro de Notificaciones
client.on('server.sessionUpdated', (d) => {
  const session = d && d.session ? d.session : d;
  if (!session || !session.status) return;
  if (session.status === notifLastServerStatus) return;
  notifLastServerStatus = session.status;
  if (session.status === 'connected') {
    const sName = session.serverName || session.serverAddress || 'Servidor';
    const idType = session.isHighId ? 'HighID' : 'LowID';
    addLog('success', `Conectado a servidor eD2k: ${sName} (${idType})`);
  } else if (session.status === 'disconnected') {
    addLog('warn', 'Desconectado del servidor eD2k.');
  }
});
client.on('event.statsTick', (d) => {
  if (d && d.stats) {
    latestStatsTick = d.stats;
    const modal = document.getElementById('bandwidth-details-modal');
    if (modal && modal.classList.contains('active')) {
      renderBandwidthModal();
    }
  }
});
client.on('event.downloadAdded', (d) => {
  const dl = d && d.download ? d.download : d;
  const name = fixMojibake((dl && (dl.fileName || dl.file_name)) || 'Nuevo archivo');
  addLog('info', `Nueva descarga en cola: ${name}`);
});
// Sistema de notificaciones nativas de escritorio (HTML5 Web Notifications)
function sendDesktopNotification(title, message) {
  if (!('Notification' in window)) return;
  if (Notification.permission === 'granted') {
    try {
      new Notification(title, {
        body: message,
        icon: "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>⚡</text></svg>"
      });
    } catch (e) {
      console.warn('Error enviando notificación desktop:', e);
    }
  }
}

client.on('event.downloadUpdated', (d) => {
  if (!d || !d.download) return;
  const status = (d.download.status || '').toLowerCase();
  if (status === 'completed') {
    refreshDownloads();
  }
});

client.on('event.downloadCompleted', (d) => {
  const dl = d && d.download ? d.download : d;
  const name = fixMojibake((dl && (dl.fileName || dl.file_name)) || 'Archivo');
  addLog('success', `Descarga COMPLETADA: ${name}`);
  refreshDownloads();
  loadMediaLibrary();
});

client.on('event.downloadFailed', (d) => {
  const dl = d && d.download ? d.download : d;
  const name = fixMojibake((dl && (dl.fileName || dl.file_name)) || 'Archivo');
  const reason = d && d.reason ? ` (${d.reason})` : '';
  addLog('error', `Error en descarga: ${name}${reason}`);
  refreshDownloads();
});

client.on('event.notificationCreated', (d) => {
  const item = d && d.notification ? d.notification : d;
  if (!item || !item.title) return;
  notifItems = [item, ...notifItems.filter(n => n.id !== item.id)].slice(0, 60);
  if (!item.isRead) {
    notifUnreadCount = (Number(notifUnreadCount) || 0) + 1;
  }
  updateNotificationBadges();
  renderNotificationsPopover();

  const toastType = ['success', 'warning', 'error', 'info'].includes(item.severity)
    ? item.severity
    : 'info';
  const icon = notifIconFor(item);
  showToast(item.body ? `${item.title}: ${item.body}` : item.title, toastType, icon);
  addLog(toastType === 'error' ? 'error' : (toastType === 'warning' ? 'warn' : toastType), `${item.title} — ${item.body || ''}`);

  if (isBrowserNotifPrefEnabled()) {
    sendDesktopNotification(`Pulse Core · ${item.title}`, item.body || '');
  }
  if (isSoundNotifPrefEnabled()) {
    playNotificationChime(item.severity);
  }
});

