// Pulse Core WebUI — Acciones globales, barra de herramientas, notificaciones toast, menú contextual, aviso legal y detección LowID.
// Extraído de web/index.html.

// === ACCIONES GLOBALES Y BOTONES ===
window.pauseDownload = async (id) => {
  await client.call('downloads.pause', { id });
  refreshDownloads();
};
window.resumeDownload = async (id) => {
  await client.call('downloads.resume', { id });
  refreshDownloads();
};
window.restartDownload = async (id) => {
  try {
    await client.call('downloads.restart', { id });
    refreshDownloads();
  } catch (e) {
    console.error('Error restarting download:', e);
  }
};

let pendingRemoveId = null;
window.confirmRemoveDownload = (id, encodedName, isTorrent) => {
  pendingRemoveId = id;
  const name = decodeURIComponent(encodedName);
  document.getElementById('remove-modal-title').textContent = isTorrent ? '¿Eliminar Torrent de la cola?' : '¿Eliminar descarga eD2k de la cola?';
  document.getElementById('remove-modal-filename').textContent = name;
  document.getElementById('remove-delete-files').checked = false;
  document.getElementById('remove-modal').classList.add('active');
};
document.getElementById('btn-cancel-remove').onclick = () => {
  document.getElementById('remove-modal').classList.remove('active');
  pendingRemoveId = null;
};
document.getElementById('btn-confirm-remove').onclick = async () => {
  if (!pendingRemoveId) return;
  const deleteFiles = document.getElementById('remove-delete-files').checked;
  try {
    await client.call('downloads.remove', { id: pendingRemoveId, deleteFiles });
    document.getElementById('remove-modal').classList.remove('active');
    pendingRemoveId = null;
    refreshDownloads();
  } catch (e) {
    appNotify('Error al eliminar descarga: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};

window.removeDownload = (id) => {
  window.confirmRemoveDownload(id, 'este archivo', false);
};

// === ACCIONES DE BARRA DE HERRAMIENTAS (CONTEXTUALES + GLOBALES) ===
const btnActionResume = document.getElementById('btn-action-resume');
if (btnActionResume) {
  btnActionResume.onclick = async () => {
    if (selectedIds.size > 0) {
      const count = selectedIds.size;
      for (const id of selectedIds) {
        try { await client.call('downloads.resume', { id }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
      }
      addLog('info', `Reanudadas ${count} descargas seleccionadas.`);
      selectedIds.clear();
      updateBatchActionBar();
      refreshDownloads();
    } else {
      const paused = cachedDownloads.filter(d => {
        const s = ((d.snapshot || d).status || '').toLowerCase();
        return s === 'paused';
      });
      if (paused.length === 0) return appNotify('No hay descargas en pausa.', 'info', '⏸️');
      btnActionResume.disabled = true;
      for (const item of paused) {
        const id = (item.snapshot || item).id || item.id;
        try { await client.call('downloads.resume', { id }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
      }
      btnActionResume.disabled = false;
      refreshDownloads();
    }
  };
}

const btnActionPause = document.getElementById('btn-action-pause');
if (btnActionPause) {
  btnActionPause.onclick = async () => {
    if (selectedIds.size > 0) {
      const count = selectedIds.size;
      for (const id of selectedIds) {
        try { await client.call('downloads.pause', { id }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
      }
      addLog('info', `Pausadas ${count} descargas seleccionadas.`);
      selectedIds.clear();
      updateBatchActionBar();
      refreshDownloads();
    } else {
      const active = cachedDownloads.filter(d => {
        const s = ((d.snapshot || d).status || '').toLowerCase();
        return s === 'downloading' || s === 'waiting';
      });
      if (active.length === 0) return appNotify('No hay descargas activas para pausar.', 'info', 'ℹ️');
      btnActionPause.disabled = true;
      for (const item of active) {
        const id = (item.snapshot || item).id || item.id;
        try { await client.call('downloads.pause', { id }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
      }
      btnActionPause.disabled = false;
      refreshDownloads();
    }
  };
}

const btnActionRestart = document.getElementById('btn-action-restart');
if (btnActionRestart) {
  btnActionRestart.onclick = async () => {
    if (selectedIds.size > 0) {
      const count = selectedIds.size;
      for (const id of selectedIds) {
        try { await client.call('downloads.restart', { id }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
      }
      addLog('info', `Reiniciadas ${count} descargas seleccionadas.`);
      selectedIds.clear();
      updateBatchActionBar();
      refreshDownloads();
    } else {
      const list = cachedDownloads;
      if (list.length === 0) return appNotify('No hay descargas en la lista.', 'info', 'ℹ️');
      btnActionRestart.disabled = true;
      for (const item of list) {
        const id = (item.snapshot || item).id || item.id;
        try { await client.call('downloads.restart', { id }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
      }
      btnActionRestart.disabled = false;
      refreshDownloads();
    }
  };
}

const btnActionDelete = document.getElementById('btn-action-delete');
if (btnActionDelete) {
  btnActionDelete.onclick = async () => {
    if (selectedIds.size === 0) return;
    const count = selectedIds.size;
    const delFiles = await appConfirm({
      title: 'Quitar descargas seleccionadas',
      message: `¿Deseas eliminar las ${count} descargas seleccionadas de la cola?\n\n(Los archivos en disco permanecerán intactos).`,
      confirmText: 'Quitar de la cola'
    });
    if (!delFiles) return;
    for (const id of selectedIds) {
      try { await client.call('downloads.remove', { id, deleteFiles: false }); } catch (e) { console.warn('[PulseCore] error en acción en lote:', e); }
    }
    addLog('info', `Quitadas ${count} descargas de la lista.`);
    selectedIds.clear();
    updateBatchActionBar();
    refreshDownloads();
  };
}

// === MENÚ DESPLEGABLE LIMPIAR ===
const btnDropdownClean = document.getElementById('btn-dropdown-clean');
const cleanMenu = document.getElementById('clean-dropdown-menu');
if (btnDropdownClean && cleanMenu) {
  btnDropdownClean.onclick = (e) => {
    e.stopPropagation();
    cleanMenu.style.display = cleanMenu.style.display === 'block' ? 'none' : 'block';
  };
  document.addEventListener('click', (e) => {
    if (!e.target.closest('.dropdown')) {
      cleanMenu.style.display = 'none';
    }
  });
}

// === MENÚ DESPLEGABLE HERRAMIENTAS CINE & SERIES ===
const btnMediaTools = document.getElementById('btn-media-tools-toggle');
const mediaToolsMenu = document.getElementById('media-tools-dropdown-menu');
if (btnMediaTools && mediaToolsMenu) {
  btnMediaTools.onclick = (e) => {
    e.stopPropagation();
    mediaToolsMenu.style.display = mediaToolsMenu.style.display === 'block' ? 'none' : 'block';
  };
  document.addEventListener('click', (e) => {
    if (!e.target.closest('#media-tools-dropdown-container')) {
      mediaToolsMenu.style.display = 'none';
    }
  });
}

let hideCompletedInUi = localStorage.getItem('mule_hide_completed') === 'true';
function updateHideCompletedCheck() {
  const checkEl = document.getElementById('hide-completed-check');
  if (checkEl) checkEl.style.display = hideCompletedInUi ? 'inline' : 'none';
}
updateHideCompletedCheck();

const menuToggleCompleted = document.getElementById('menu-toggle-completed');
if (menuToggleCompleted) {
  menuToggleCompleted.onclick = () => {
    hideCompletedInUi = !hideCompletedInUi;
    localStorage.setItem('mule_hide_completed', hideCompletedInUi);
    updateHideCompletedCheck();
    if (cleanMenu) cleanMenu.style.display = 'none';
    lastRenderedDownloadIds = '';
    renderDownloadsTable();
  };
}

const menuClearCompleted = document.getElementById('menu-clear-completed');
if (menuClearCompleted) {
  menuClearCompleted.onclick = () => {
    if (cleanMenu) cleanMenu.style.display = 'none';
    clearCompletedFromList();
  };
}

// === NOTIFICACIÓN TOAST FLOTANTE ===
function showToast(message, type = 'info', icon = 'ℹ️') {
  const toast = document.getElementById('toast-notify');
  const toastMsg = document.getElementById('toast-msg');
  const toastIcon = document.getElementById('toast-icon');
  if (!toast) return;
  toast.className = `toast-notification ${type} active`;
  if (toastMsg) toastMsg.textContent = message;
  if (toastIcon) toastIcon.textContent = icon;
  clearTimeout(window.__toastTimer);
  window.__toastTimer = setTimeout(() => {
    toast.classList.remove('active');
  }, 4000);
}
const showToastNotification = showToast;
window.showToast = showToast;
window.showToastNotification = showToastNotification;

// === MODO COMPACTO DE LA TABLA ===
let isCompactMode = localStorage.getItem('mule_compact_mode') !== 'false'; // Por defecto activado (más limpio y ergonómico)
function initCompactMode() {
  if (isCompactMode) {
    document.body.classList.add('compact-mode');
  } else {
    document.body.classList.remove('compact-mode');
  }
  updateCompactModeBtn();
}
/**
 * Genera segmentos contiguos HTML para la cinta continua de partes de una descarga.
 */
function buildPartRibbonSegments(parts) {
  if (!Array.isArray(parts) || parts.length === 0) return '';
  const total = parts.length;
  const runs = [];
  for (let i = 0; i < total; i++) {
    const st = parts[i]?.state || 'missing';
    if (runs.length > 0 && runs[runs.length - 1].state === st) {
      runs[runs.length - 1].count++;
    } else {
      runs.push({ state: st, count: 1 });
    }
  }
  return runs.map(r => {
    const pct = ((r.count / total) * 100).toFixed(3);
    return `<span class="part-ribbon-seg ${r.state}" style="width:${pct}%"></span>`;
  }).join('');
}

function updateCompactModeBtn() {
  const btn = document.getElementById('btn-toggle-compact-mode');
  if (!btn) return;
  const iconCompact = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="18" x2="21" y2="18"/></svg>';
  const iconDetailed = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/></svg>';
  btn.innerHTML = isCompactMode
    ? `${iconCompact}<span>Modo Compacto</span>`
    : `${iconDetailed}<span>Modo Detallado</span>`;
  btn.classList.remove('btn-primary');
  btn.style.display = 'inline-flex';
  btn.style.alignItems = 'center';
  btn.style.gap = '6px';
  btn.title = isCompactMode ? 'Modo compacto activo (haz clic para desplegar mapa de trozos en las filas)' : 'Modo detallado activo (haz clic para compactar la lista)';
}
const btnToggleCompact = document.getElementById('btn-toggle-compact-mode');
if (btnToggleCompact) {
  btnToggleCompact.onclick = () => {
    isCompactMode = !isCompactMode;
    localStorage.setItem('mule_compact_mode', isCompactMode ? 'true' : 'false');
    initCompactMode();
    lastRenderedDownloadIds = '';
    renderDownloadsTable();
    showToast(isCompactMode ? 'Modo Compacto activado: Máxima densidad de filas' : 'Modo Detallado activado: Vista ampliada', 'info', isCompactMode ? '📐' : '📑');
  };
}
initCompactMode();

async function clearCompletedFromList() {
  const completed = cachedDownloads.filter(item => {
    const dl = item.snapshot || item;
    const status = (dl.status || '').toLowerCase();
    const total = dl.totalBytes || dl.total_bytes || 0;
    const transferred = dl.receivedBytes || dl.received_bytes || 0;
    return (total > 0 && transferred >= total) || status === 'completed';
  });

  if (completed.length === 0) {
    showToast('No hay descargas completadas en la lista.', 'warning', '⚠️');
    return;
  }

  const ok = await appConfirm({
    title: 'Limpiar descargas completadas',
    message: `¿Deseas quitar de la lista las ${completed.length} descargas completadas?\n\n(Tus archivos descargados en el NAS o disco permanecerán intactos y a salvo; solo se quitan de la vista).`,
    confirmText: 'Limpiar vista'
  });
  if (!ok) return;

  addLog('info', `Quitando ${completed.length} descargas completadas de la lista (archivos conservados en el NAS)...`);
  for (const item of completed) {
    const id = (item.snapshot || item).id || item.id;
    try {
      await client.call('downloads.remove', { id, deleteFiles: false });
    } catch (e) {
      console.error('Error al limpiar descarga completada:', e);
    }
  }
  showToast(`Se han retirado ${completed.length} descargas de la vista. Archivos a salvo en el NAS.`, 'success', '🧹');
  addLog('success', `${completed.length} descargas completadas limpiadas de la lista.`);
  if (selectedDownloadId && completed.some(c => ((c.snapshot || c).id || c.id) === selectedDownloadId)) {
    window.closeInspector();
  }
  refreshDownloads();
}

const btnClearOld = document.getElementById('btn-clear-completed');
if (btnClearOld) btnClearOld.onclick = clearCompletedFromList;

// === ACCIONES EN LOTE (BATCH MULTI-SELECCIÓN) ===
const thSelectAll = document.getElementById('th-select-all');
if (thSelectAll) {
  thSelectAll.onchange = (e) => {
    selectAllRows(e.target.checked);
  };
}

// === ACCIONES DIRECTAS EN PANEL INSPECTOR ===
document.getElementById('btn-insp-remove-list').onclick = async () => {
  if (!selectedDownloadId) return;
  if (!(await appConfirm({
    title: 'Quitar de la cola',
    message: '¿Quitar esta descarga de la cola?\nEl archivo descargado se mantendrá intacto en el disco.',
    confirmText: 'Quitar'
  }))) return;
  try {
    await client.call('downloads.remove', { id: selectedDownloadId, deleteFiles: false });
    addLog('info', 'Descarga quitada de la lista.');
    window.closeInspector();
    refreshDownloads();
  } catch (e) {
    appNotify('Error: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};

document.getElementById('btn-insp-delete-files').onclick = async () => {
  if (!selectedDownloadId) return;
  if (!(await appConfirm({
    title: '⚠️ Borrar descarga y archivo',
    message: '¿PELIGRO: Eliminar esta descarga Y BORRAR EL ARCHIVO del disco/NAS?\nEsta acción es irreversible.',
    confirmText: 'Borrar archivo',
    danger: true
  }))) return;
  try {
    await client.call('downloads.remove', { id: selectedDownloadId, deleteFiles: true });
    addLog('warn', 'Descarga y datos eliminados del disco.');
    window.closeInspector();
    refreshDownloads();
  } catch (e) {
    appNotify('Error: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};

// === MENÚ CONTEXTUAL FLOTANTE (CLIC DERECHO) ===
let contextMenuTargetId = null;
const contextMenu = document.getElementById('context-menu');

window.handleRowContextMenu = (event, id) => {
  event.preventDefault();
  contextMenuTargetId = id;
  selectDownload(id);

  // Posicionar menú
  const x = Math.min(event.clientX, window.innerWidth - 240);
  const y = Math.min(event.clientY, window.innerHeight - 320);
  contextMenu.style.left = `${x}px`;
  contextMenu.style.top = `${y}px`;
  contextMenu.style.display = 'block';
};

document.addEventListener('click', (e) => {
  if (!contextMenu.contains(e.target)) {
    contextMenu.style.display = 'none';
  }
});

document.querySelectorAll('#context-menu .context-item').forEach(item => {
  item.addEventListener('click', async (e) => {
    e.stopPropagation();
    contextMenu.style.display = 'none';
    const action = item.dataset.action;
    const id = contextMenuTargetId;

    if (action === 'clear-completed') {
      clearCompletedFromList();
      return;
    }

    if (!id) return;
    const targetDl = cachedDownloads.find(d => ((d.snapshot || d).id || d.id) === id);
    const snap = targetDl ? (targetDl.snapshot || targetDl) : {};
    const displayName = fixMojibake(snap.fileName || 'Archivo');

    switch (action) {
      case 'force-start':
        await client.call('downloads.setPriority', { id, priority: 'high' });
        await client.call('downloads.resume', { id });
        addLog('info', `Forzado inicio prioritario: ${displayName}`);
        showToast('⚡ Inicio prioritario asignado a ' + displayName);
        refreshDownloads();
        break;
      case 'resume':
        await client.call('downloads.resume', { id });
        addLog('info', `Reanudada descarga: ${displayName}`);
        refreshDownloads();
        break;
      case 'pause':
        await client.call('downloads.pause', { id });
        addLog('info', `Pausada descarga: ${displayName}`);
        refreshDownloads();
        break;
      case 'restart':
        await client.call('downloads.restart', { id });
        addLog('info', `Reiniciada descarga: ${displayName}`);
        refreshDownloads();
        break;
      case 'open-folder':
        if (typeof openDestinationFolder === 'function') {
          openDestinationFolder({ downloadId: id, title: displayName });
        }
        break;
      case 'prio-high':
      case 'prio-normal':
      case 'prio-low':
      case 'prio-auto': {
        const prio = action.replace('prio-', '');
        await client.call('downloads.setPriority', { id, priority: prio });
        addLog('info', `Prioridad de ${displayName} cambiada a ${prio.toUpperCase()}`);
        refreshDownloads();
        break;
      }
      case 'copy-ed2k': {
        const hash = snap.ed2kHash || snap.ed2k_hash || '';
        const size = snap.totalBytes || snap.total_bytes || 0;
        const link = `ed2k://|file|${encodeURIComponent(displayName)}|${size}|${hash}|/`;
        navigator.clipboard.writeText(link);
        showToast('📋 Enlace eD2k copiado al portapapeles');
        break;
      }
      case 'copy-magnet': {
        const hash = snap.ed2kHash || snap.ed2k_hash || '';
        const isTorrent = (snap.fileName || '').startsWith('⚡ [Torrent]') || hash.length === 40;
        const magnet = isTorrent ? `magnet:?xt=urn:btih:${hash}&dn=${encodeURIComponent(displayName)}` : `ed2k://|file|${encodeURIComponent(displayName)}|${snap.totalBytes || 0}|${hash}|/`;
        navigator.clipboard.writeText(magnet);
        showToast('🧲 Enlace copiado al portapapeles');
        break;
      }
      case 'copy-hash': {
        const hash = snap.ed2kHash || snap.ed2k_hash || '';
        navigator.clipboard.writeText(hash);
        showToast('📋 Hash copiado: ' + hash);
        break;
      }
      case 'remove-list':
        if (await appConfirm({
          title: 'Quitar de la cola',
          message: `¿Quitar "${displayName}" de la cola de descargas?\nEl archivo en disco permanecerá intacto.`,
          confirmText: 'Quitar'
        })) {
          await client.call('downloads.remove', { id, deleteFiles: false });
          addLog('info', `Descarga quitada de la lista: ${displayName}`);
          if (selectedDownloadId === id) window.closeInspector();
          refreshDownloads();
        }
        break;
      case 'delete-files':
        if (await appConfirm({
          title: '⚠️ Borrar descarga y archivo',
          message: `¿PELIGRO: Eliminar "${displayName}" Y BORRAR SU ARCHIVO DEL DISCO/NAS?\nAcción irreversible.`,
          confirmText: 'Borrar archivo',
          danger: true
        })) {
          await client.call('downloads.remove', { id, deleteFiles: true });
          addLog('warn', `Eliminada descarga y datos: ${displayName}`);
          if (selectedDownloadId === id) window.closeInspector();
          refreshDownloads();
        }
        break;
    }
  });
});

document.getElementById('btn-turtle-mode').onclick = toggleTurtleMode;

// Controladores del Registro en Vivo (Live Logs)
document.getElementById('btn-toggle-autoscroll').onclick = () => {
  autoScrollLogs = !autoScrollLogs;
  const btn = document.getElementById('btn-toggle-autoscroll');
  btn.textContent = autoScrollLogs ? 'Auto-scroll: ON' : 'Auto-scroll: OFF';
  btn.className = autoScrollLogs ? 'btn btn-sm btn-primary' : 'btn btn-sm';
};

const btnRefreshLogs = document.getElementById('btn-refresh-logs');
if (btnRefreshLogs) {
  btnRefreshLogs.onclick = () => refreshSystemLogs();
}

const inputSearchLogs = document.getElementById('input-search-logs');
if (inputSearchLogs) {
  let searchTimeout = null;
  inputSearchLogs.addEventListener('input', (e) => {
    clearTimeout(searchTimeout);
    searchTimeout = setTimeout(() => {
      logSearchQuery = e.target.value;
      refreshSystemLogs();
    }, 300);
  });
}

document.getElementById('btn-copy-logs').onclick = () => {
  const container = document.getElementById('log-console-container');
  if (container) {
    navigator.clipboard.writeText(container.innerText);
    appNotify('Registro de logs copiado al portapapeles.', 'success', '📋');
  }
};

document.getElementById('btn-clear-logs').onclick = () => {
  document.getElementById('log-console-container').innerHTML = '';
  addLog('info', 'Registro en pantalla vaciado.');
};

document.querySelectorAll('#filter-logs-group .filter-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('#filter-logs-group .filter-btn').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    currentLogFilter = btn.dataset.logFilter;
    refreshSystemLogs();
  });
});

// Servidores actions
window.connectServer = async (address) => {
  try {
    await client.call('servers.connect', { address });
    refreshServers();
  } catch (e) {
    appNotify('Error conectando con servidor: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};
window.disconnectServer = async () => {
  try {
    await client.call('servers.disconnect');
    refreshServers();
  } catch (e) {
    appNotify('Error desconectando: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};
document.getElementById('btn-connect-fastest').onclick = async () => {
  if (cachedServers.length === 0) return appNotify('No hay servidores en la lista.', 'warning', '⚠️');
  const allowed = cachedServers.filter(s => s.filter_status === 'allowed' || !s.filter_status);
  const sorted = (allowed.length > 0 ? allowed : cachedServers).slice().sort((a, b) => (a.ping || 9999) - (b.ping || 9999));
  if (sorted.length > 0) {
    await connectServer(sorted[0].address);
  }
};
document.getElementById('btn-disconnect-server').onclick = window.disconnectServer;
document.getElementById('btn-update-servers-url').onclick = async () => {
  const btn = document.getElementById('btn-update-servers-url');
  btn.disabled = true;
  btn.textContent = 'Descargando server.met...';
  try {
    const res = await client.call('servers.updateFromUrl', { url: 'http://upd.emule-security.org/server.met' });
    appNotify(`Servidores importados con éxito: ${res.imported || res.totalCount || 0}`, 'success', '✅');
    refreshServers();
  } catch (e) {
    appNotify('Error actualizando lista: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  } finally {
    btn.disabled = false;
    btn.textContent = '🔄 Actualizar server.met desde URL';
  }
};
document.getElementById('btn-import-amule-servers').onclick = async () => {
  const path = prompt('Ruta al archivo server.met de aMule:', '~/.aMule/server.met');
  if (!path) return;
  try {
    const res = await client.call('servers.importMet', { filePath: path });
    appNotify(`Servidores importados: ${res.imported || res.totalCount || 0}`, 'success', '✅');
    refreshServers();
  } catch (e) {
    appNotify('Error al importar server.met: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};

// Kad actions
document.getElementById('btn-bootstrap-kad').onclick = async () => {
  const btn = document.getElementById('btn-bootstrap-kad');
  btn.disabled = true;
  btn.textContent = 'Descargando nodes.dat...';
  try {
    const res = await client.call('kad.bootstrapFromUrl', { url: 'http://upd.emule-security.org/nodes.dat' });
    appNotify(`Bootstrap Kad iniciado. Contactos importados: ${res.importedCount || 0}`, 'success', '🚀');
    refreshKad();
    setTimeout(refreshKadContacts, 1500);
  } catch (e) {
    appNotify('Error en bootstrap Kad: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  } finally {
    btn.disabled = false;
    btn.textContent = '🚀 Bootstrap Kad desde URL';
  }
};
document.getElementById('btn-import-amule-kad').onclick = async () => {
  const path = prompt('Ruta al archivo nodes.dat de aMule:', '~/.aMule/nodes.dat');
  if (!path) return;
  try {
    const res = await client.call('kad.importNodesDat', { path });
    appNotify(`Nodos importados: ${res.importedCount || 0}`, 'success', '✅');
    refreshKad();
    refreshKadContacts();
  } catch (e) {
    appNotify('Error al importar nodes.dat: ' + (e.message || JSON.stringify(e)), 'error', '❌');
  }
};
document.getElementById('btn-refresh-kad-contacts').onclick = refreshKadContacts;

// === GESTIÓN DE AVISO LEGAL & PRIMER INICIO & DETECCIÓN LOWID / PUERTOS ===
let lastKnownIsHighId = null;
let lastKnownKadFw = null;
let lowIdBannerDismissed = false;
let legalDisclaimerAccepted = false;

async function checkLegalDisclaimer() {
  // 1. Comprobar primero en localStorage para respuesta instantánea
  if (localStorage.getItem('mule_disclaimer_accepted') === 'true') {
    legalDisclaimerAccepted = true;
    return true;
  }

  // 2. Comprobar en ajustes persistidos del daemon
  try {
    const s = await client.call('settings.get', {});
    if (s && s.legalDisclaimerAccepted) {
      localStorage.setItem('mule_disclaimer_accepted', 'true');
      legalDisclaimerAccepted = true;
      return true;
    }
  } catch (err) {
    console.warn('No se pudo verificar el estado del aviso legal en el daemon:', err);
  }

  // 3. Si no ha sido aceptado, mostrar el modal bloqueante
  const modal = document.getElementById('legal-disclaimer-modal');
  if (modal) {
    modal.style.display = 'flex';
    modal.classList.add('active');
  }
  return false;
}

async function handleLegalDisclaimerResponse(agreed) {
  const modal = document.getElementById('legal-disclaimer-modal');
  if (agreed) {
    // El usuario acepta las condiciones
    legalDisclaimerAccepted = true;
    localStorage.setItem('mule_disclaimer_accepted', 'true');
    try {
      await client.call('settings.set', { legalDisclaimerAccepted: true });
    } catch (e) {
      console.warn('Error al guardar aceptación del aviso legal:', e);
    }
    if (modal) {
      modal.classList.remove('active');
      modal.style.display = 'none';
    }
    showToast('Condiciones aceptadas. Iniciando servicios...', 'success', '⚖️');
    // Proceder con la inicialización habitual de primer inicio
    checkFirstRunSetup();
  } else {
    // El usuario rechaza las condiciones: detener el daemon y cerrar la sesión
    if (modal) {
      modal.innerHTML = `
        <div class="modal" style="max-width: 520px; width: 90%; text-align: center; padding: 32px 24px;">
          <span style="font-size: 3rem; display: block; margin-bottom: 16px;">🛑</span>
          <h3 style="margin-bottom: 12px; font-size: 1.25rem; color: #ef4444;">Programa detenido</h3>
          <p style="color: #cbd5e1; font-size: 0.9rem; line-height: 1.5; margin-bottom: 20px;">
            Ha indicado disconformidad con el aviso legal y condiciones de uso. El servidor Pulse Core se ha apagado ordenadamente.
          </p>
          <div style="background: rgba(239, 68, 68, 0.1); border: 1px solid rgba(239, 68, 68, 0.3); padding: 10px; border-radius: 6px; font-size: 0.8rem; color: #fca5a5; margin-bottom: 20px;">
            Ya puede cerrar con seguridad esta pestaña del navegador.
          </div>
          <button class="btn" onclick="window.close();" style="padding: 8px 20px;">Cerrar pestaña</button>
        </div>
      `;
    }

    try {
      await client.call('system.stop', {});
    } catch (e) {
      console.warn('Error al solicitar detención del sistema:', e);
    }

    if (client && typeof client.disconnect === 'function') {
      try { client.disconnect(); } catch (err) { console.debug('[PulseCore] desconexión cliente:', err); }
    }
  }
}

async function checkFirstRunSetup() {
  // Si el disclaimer no está aceptado, no ejecutar aprovisionamiento inicial
  if (!legalDisclaimerAccepted && localStorage.getItem('mule_disclaimer_accepted') !== 'true') {
    return;
  }
  try {
    const setupDone = localStorage.getItem('mule_first_run_completed');
    if (setupDone === 'true') {
      return;
    }

    addLog('info', '🚀 Primer inicio detectado: Iniciando aprovisionamiento automático de redes...');
    showToast('🚀 Configuración inicial: Descargando lista de servidores y conectando...', 'info', '⚡');

    // 1. Activar reconexión y autoconexión automática en el daemon
    try {
      await client.call('settings.set', {
        serverReconnect: true,
        serverAutoconnect: true,
        upnpEnabled: true
      });
    } catch (e) {
      console.warn('No se pudieron fijar ajustes iniciales:', e);
    }

    // 2. Descargar e importar server.met canónico
    let serversCount = 0;
    try {
      const sRes = await client.call('servers.updateFromUrl', {
        url: 'http://upd.emule-security.org/server.met'
      });
      serversCount = sRes.imported || sRes.totalCount || 0;
      addLog('success', `Lista inicial de servidores eD2k importada (${serversCount} servidores).`);
      await refreshServers();
    } catch (e) {
      console.warn('Error al descargar server.met inicial:', e);
    }

    // 3. Iniciar Kad DHT y bootstrap de nodes.dat
    try {
      await client.call('kad.startUdp', {});
      const kRes = await client.call('kad.bootstrapFromUrl', {
        url: 'http://upd.emule-security.org/nodes.dat'
      });
      addLog('success', `Bootstrap inicial Kademlia iniciado (${kRes.importedCount || 0} contactos).`);
      await refreshKad();
    } catch (e) {
      console.warn('Error al iniciar bootstrap Kademlia:', e);
    }

    // 4. Conectar automáticamente al mejor servidor eD2k disponible
    try {
      if (cachedServers && cachedServers.length > 0) {
        const allowed = cachedServers.filter(s => s.filter_status === 'allowed' || !s.filter_status);
        const pool = allowed.length > 0 ? allowed : cachedServers;
        const target = pool.slice().sort((a, b) => (a.ping || 9999) - (b.ping || 9999))[0];
        if (target && target.address) {
          await client.call('servers.connect', { address: target.address });
          addLog('info', `Conectando automáticamente a servidor: ${target.name || target.address}`);
        }
      }
    } catch (e) {
      console.warn('Error al conectar al servidor inicial:', e);
    }

    // 5. Marcar como completado el primer inicio
    localStorage.setItem('mule_first_run_completed', 'true');
    showToast('✅ Redes eD2k y Kademlia inicializadas con éxito.', 'success', '🌐');
  } catch (err) {
    console.error('Error durante el aprovisionamiento de primer inicio:', err);
  }
}

function evaluateLowIdState(isHighId, kadFirewall) {
  if (isHighId !== null) lastKnownIsHighId = isHighId;
  if (kadFirewall !== null) lastKnownKadFw = kadFirewall;

  const neverShow = localStorage.getItem('mule_lowid_tutorial_suppressed') === 'true';
  if (neverShow || lowIdBannerDismissed) {
    return;
  }

  const banner = document.getElementById('lowid-warning-banner');
  if (!banner) return;

  const isLowId = lastKnownIsHighId === false;
  const isKadFirewalled = lastKnownKadFw === 'firewalled';

  if (isLowId || isKadFirewalled) {
    banner.style.display = 'flex';
    const titleEl = document.getElementById('lowid-banner-title');
    const descEl = document.getElementById('lowid-banner-desc');
    if (isLowId && isKadFirewalled) {
      if (titleEl) titleEl.textContent = 'Estado Limitado: eD2k LowID y Kad Firewalled detectados';
      if (descEl) descEl.textContent = 'Los puertos TCP y UDP están inaccesibles desde el exterior. El router o firewall impiden conexiones entrantes.';
    } else if (isLowId) {
      if (titleEl) titleEl.textContent = 'Estado Limitado: eD2k LowID detectado';
      if (descEl) descEl.textContent = 'El puerto TCP está bloqueado por tu router o firewall. Esto reduce las fuentes disponibles y la velocidad.';
    } else {
      if (titleEl) titleEl.textContent = 'Estado Limitado: Kademlia tras NAT (Firewalled)';
      if (descEl) descEl.textContent = 'El puerto UDP de Kademlia no recibe paquetes directos. Se recomienda abrir puertos para mayor eficacia DHT.';
    }
  } else if (lastKnownIsHighId === true && (!lastKnownKadFw || lastKnownKadFw === 'open' || lastKnownKadFw === 'unverified')) {
    banner.style.display = 'none';
  }
}

window.dismissLowIdBanner = function() {
  lowIdBannerDismissed = true;
  const banner = document.getElementById('lowid-warning-banner');
  if (banner) banner.style.display = 'none';
};

window.openPortTutorialModal = async function() {
  openModal('modal-port-tutorial');
  // Obtener puertos configurados actualmente
  try {
    const s = await client.call('settings.get');
    const tcpPort = s.ed2kPort || 4662;
    const kadPort = s.kadPort || 4672;
    const localIp = window.location.hostname || '192.168.1.X';

    const elTcp = document.getElementById('tut-tcp-port');
    const elKad = document.getElementById('tut-kad-port');
    const elIp = document.getElementById('tut-local-ip');

    if (elTcp) elTcp.textContent = tcpPort;
    if (elKad) elKad.textContent = kadPort;
    if (elIp) elIp.textContent = localIp;

    document.querySelectorAll('.tut-val-tcp').forEach(el => el.textContent = tcpPort);
    document.querySelectorAll('.tut-val-udp').forEach(el => el.textContent = kadPort);
    document.querySelectorAll('.tut-val-ip').forEach(el => el.textContent = localIp);

    const chk = document.getElementById('chk-never-show-lowid');
    if (chk) {
      chk.checked = localStorage.getItem('mule_lowid_tutorial_suppressed') === 'true';
      chk.onchange = (e) => {
        localStorage.setItem('mule_lowid_tutorial_suppressed', e.target.checked ? 'true' : 'false');
      };
    }
  } catch (e) {
    console.warn('Error leyendo opciones para tutorial de puertos:', e);
  }
};

window.tryUpnpQuickFix = async function(btn) {
  if (btn) {
    btn.disabled = true;
    btn.innerHTML = '🔄 Solicitando UPnP...';
  }
  const statusDiv = document.getElementById('tutorial-upnp-status');
  if (statusDiv) {
    statusDiv.style.display = 'block';
    statusDiv.style.color = '#38bdf8';
    statusDiv.textContent = 'Enviando petición UPnP IGD a la puerta de enlace...';
  }

  try {
    const s = await client.call('settings.get');
    const tcpPort = s.ed2kPort || 4662;
    const kadPort = s.kadPort || 4672;

    const res = await client.call('upnp.mapPorts', { tcpPort, udpPort: kadPort });
    const isMapped = res.tcpMapped && res.udpMapped;

    if (isMapped) {
      if (statusDiv) {
        statusDiv.style.color = '#34d399';
        statusDiv.textContent = `✅ ¡Puertos mapeados con éxito en el router! (IP Externa: ${res.externalIp || 'detectada'})`;
      }
      showToast('✅ Reglas UPnP aplicadas con éxito en el router', 'success', '🔌');
      // Reintentar conexión para verificar si pasa a HighID
      await refreshServers();
      await refreshKad();
    } else {
      if (statusDiv) {
        statusDiv.style.color = '#fbbf24';
        statusDiv.textContent = '⚠️ El router no respondió a las peticiones UPnP. Sigue la Opción 2 para abrirlos manualmente.';
      }
      showToast('El router no admitió UPnP. Requiere apertura manual.', 'warning', '⚠️');
    }
  } catch (err) {
    if (statusDiv) {
      statusDiv.style.color = '#fb7185';
      statusDiv.textContent = 'Error UPnP: ' + (err.message || err);
    }
    showToast('Error en petición UPnP: ' + (err.message || err), 'error', '❌');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.innerHTML = '⚡ Probar UPnP Ahora';
    }
  }
};

window.recheckPortsAfterTutorial = async function(btn) {
  if (btn) {
    btn.disabled = true;
    btn.textContent = 'Comprobando...';
  }
  try {
    showToast('Verificando estado de red...', 'info', '🔍');
    await refreshServers();
    await refreshKad();
    const sStatus = await client.call('servers.status');
    if (sStatus && sStatus.isHighId) {
      showToast('🎉 ¡Enhorabuena! Has conseguido HighID.', 'success', '🟢');
      closeModal('modal-port-tutorial');
      dismissLowIdBanner();
    } else {
      showToast('Aún se detecta LowID. Asegúrate de guardar los cambios y reiniciar el router si es necesario.', 'warning', '⚠️');
    }
  } catch (e) {
    console.warn('Error al recomprobar puertos:', e);
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '🔄 Comprobar de Nuevo';
    }
  }
};

