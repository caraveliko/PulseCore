// Pulse Core WebUI — Guía local interactiva y subpaneles de configuración del servidor.
// Extraído de web/index.html.

// === GUÍA LOCAL INTERACTIVA (DRAWER SIN CONEXIÓN A MODELOS IA) ===
let assistantTurnActive = false;
let currentAssistantMsgEl = null;

window.toggleAssistantDrawer = function() {
  const drawer = document.getElementById('assistant-drawer');
  if (!drawer) return;
  const isVisible = drawer.style.display !== 'none';
  if (isVisible) {
    drawer.style.display = 'none';
  } else {
    drawer.style.display = 'flex';
    const input = document.getElementById('ast-input');
    if (input) setTimeout(() => input.focus(), 100);
  }
};

window.clearAssistantChat = function() {
  const container = document.getElementById('ast-messages');
  if (container) {
    container.innerHTML = `
      <div class="ast-msg assistant">
        <div class="ast-msg-avatar"><img src="/assets/copilot-avatar.svg?v=1" alt="Guía" class="ast-avatar-img"></div>
        <div class="ast-msg-body">
          Guía reiniciada. Selecciona un tema rápido o escribe tu consulta:
          <div style="margin-top: 8px; display: flex; flex-wrap: wrap; gap: 6px;">
            <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:router')" style="font-size: 0.73rem; padding: 2px 8px; background: rgba(56, 189, 248, 0.15); border: 1px solid rgba(56, 189, 248, 0.3); color: #38bdf8;">Tutorial Abrir Puertos</button>
            <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:links')" style="font-size: 0.73rem; padding: 2px 8px; background: rgba(34, 197, 94, 0.15); border: 1px solid rgba(34, 197, 94, 0.3); color: #4ade80;">Añadir Enlaces</button>
            <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:kad')" style="font-size: 0.73rem; padding: 2px 8px; background: rgba(168, 85, 247, 0.15); border: 1px solid rgba(168, 85, 247, 0.3); color: #c084fc;">Kademlia DHT</button>
            <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:mcp')" style="font-size: 0.73rem; padding: 2px 8px; background: rgba(245, 158, 11, 0.15); border: 1px solid rgba(245, 158, 11, 0.3); color: #fbbf24;">Servidor MCP (IA)</button>
          </div>
        </div>
      </div>
    `;
  }
};

function setAssistantInputEnabled(enabled) {
  const input = document.getElementById('ast-input');
  const btn = document.getElementById('ast-send-btn');
  if (input) input.disabled = !enabled;
  if (btn) {
    btn.disabled = !enabled;
    btn.textContent = enabled ? 'Enviar' : '...';
  }
}

function appendUserMessage(text) {
  const container = document.getElementById('ast-messages');
  if (!container) return;
  const msgDiv = document.createElement('div');
  msgDiv.className = 'ast-msg user';
  msgDiv.innerHTML = `
    <div class="ast-msg-avatar"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"></path><circle cx="12" cy="7" r="4"></circle></svg></div>
    <div class="ast-msg-body">${escapeHtml(text)}</div>
  `;
  container.appendChild(msgDiv);
  container.scrollTop = container.scrollHeight;
}

function ensureAssistantMessageBubble() {
  if (currentAssistantMsgEl) return currentAssistantMsgEl;
  const container = document.getElementById('ast-messages');
  if (!container) return null;
  const msgDiv = document.createElement('div');
  msgDiv.className = 'ast-msg assistant';
  msgDiv.innerHTML = `
    <div class="ast-msg-avatar"><img src="/assets/copilot-avatar.svg?v=1" alt="Guía" class="ast-avatar-img"></div>
    <div class="ast-msg-body"></div>
  `;
  container.appendChild(msgDiv);
  currentAssistantMsgEl = msgDiv.querySelector('.ast-msg-body');
  container.scrollTop = container.scrollHeight;
  return currentAssistantMsgEl;
}

window.sendAssistantMessage = async function() {
  const input = document.getElementById('ast-input');
  if (!input) return;
  const text = input.value.trim();
  if (!text || assistantTurnActive) return;

  appendUserMessage(text);
  input.value = '';
  currentAssistantMsgEl = null;
  assistantTurnActive = true;
  setAssistantInputEnabled(false);

  // Guía local interactiva (consulta únicamente el daemon RPC local y guías integradas)
  try {
    await handleAutonomousAssistantTurn(text);
  } finally {
    assistantTurnActive = false;
    setAssistantInputEnabled(true);
  }
};

async function handleAutonomousAssistantTurn(query) {
  const q = query.toLowerCase();

  // 1. Tutorial y Apertura de Puertos / LowID / NAT
  if (q.includes('puerto') || q.includes('nat') || q.includes('router') || q.includes('lowid') || q.includes('highid') || q.includes('firewall') || q === 'tutorial:router') {
    let tcpPort = 4662;
    let kadPort = 4672;
    try {
      const s = await client.call('settings.get');
      tcpPort = s.ed2kPort || 4662;
      kadPort = s.kadPort || 4672;
    } catch (err) { console.warn('[PulseCore] error en diagnóstico local:', err); }

    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #f59e0b; margin-bottom: 6px; display: flex; align-items: center; gap: 6px;">
          <span>🌐</span> <span>Guía Interactiva: Abrir Puertos en el Router (HighID)</span>
        </div>
        <div>
          Para conseguir <b>HighID</b> en eD2k y estado <b>Abierto</b> en Kademlia, otros usuarios deben poder conectarse a tus puertos sin que el router los rechace:
        </div>
        
        <div style="background: rgba(0,0,0,0.3); border-radius: 6px; padding: 8px; margin: 8px 0; border: 1px solid rgba(255,255,255,0.08);">
          <div style="font-weight:600; color:#38bdf8; margin-bottom:4px;">Puertos activos en este equipo:</div>
          • <b>TCP:</b> <code style="color:#60a5fa;">${tcpPort}</code> (Transferencias eD2k)<br>
          • <b>UDP:</b> <code style="color:#34d399;">${kadPort}</code> (Búsquedas Kademlia DHT)<br>
          • <b>IP Local Destino:</b> <code style="color:#10b981;">${window.location.hostname || '192.168.1.X'}</code>
        </div>

        <div style="margin-top: 6px;">
          <b>Pasos para configurar el router:</b>
          <ol style="margin: 4px 0 8px 16px; padding: 0;">
            <li>Entra al router abriendo <a href="http://192.168.1.1" target="_blank" style="color: #38bdf8;">192.168.1.1</a> o <a href="http://192.168.0.1" target="_blank" style="color: #38bdf8;">192.168.0.1</a>.</li>
            <li>Ve a <i>Port Forwarding / Reenvío de Puertos / Servidores Virtuales</i>.</li>
            <li>Crea una regla TCP para el puerto <code>${tcpPort}</code> y una UDP para <code>${kadPort}</code> hacia tu IP local.</li>
          </ol>
        </div>

        <div style="margin-top: 8px; display: flex; gap: 6px;">
          <button class="btn btn-sm btn-primary" onclick="openPortTutorialModal()" style="font-size: 0.74rem;">📖 Abrir Guía Visual Completa</button>
          <button class="btn btn-sm" onclick="tryUpnpQuickFix(this)" style="font-size: 0.74rem;">⚡ Probar UPnP Automático</button>
        </div>
      `;
    }
    return;
  }

  // 2. Cómo añadir descargas y enlaces
  if (q.includes('descargar') || q.includes('enlace') || q.includes('ed2k://') || q.includes('magnet') || q.includes('torrent') || q === 'tutorial:links') {
    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #34d399; margin-bottom: 6px;">📥 Cómo Añadir Descargas en eMuleRust</div>
        <div>
          eMuleRust admite enlaces duales <b>eD2k</b> y <b>BitTorrent</b> en el mismo enjambre:
        </div>
        <ul style="margin: 6px 0 8px 16px; padding: 0;">
          <li><b>Botón "+ Añadir":</b> Pulsa el botón superior en la vista de descargas y pega enlaces con formato <code>ed2k://|file|...</code> o <code>magnet:?xt=...</code>.</li>
          <li><b>Búsqueda Directa:</b> Usa la pestaña <i>"Buscar"</i> o el atajo <kbd style="background:#1e293b; padding:1px 4px; border-radius:3px;">Ctrl + K</kbd> para localizar contenidos en los servidores eD2k y DHT.</li>
          <li><b>Mediateca Automática:</b> Si usas la Filmoteca, el auto-grabber rastreará lanzamientos automáticamente.</li>
        </ul>
        <button class="btn btn-sm btn-primary" onclick="document.getElementById('add-modal').classList.add('active')" style="font-size: 0.74rem; margin-top: 4px;">＋ Abrir Diálogo de Añadir</button>
      `;
    }
    return;
  }

  // 3. Kademlia DHT
  if (q.includes('kad') || q.includes('kademlia') || q.includes('dht') || q.includes('nodes.dat') || q === 'tutorial:kad') {
    let kadCount = 0;
    let isUdp = false;
    try {
      const k = await client.call('kad.status');
      kadCount = k.totalContacts || 0;
      isUdp = !!k.isUdpActive;
    } catch (err) { console.warn('[PulseCore] error en diagnóstico local:', err); }

    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #a855f7; margin-bottom: 6px;">🚀 Red Kademlia DHT (P2P Descentralizado)</div>
        <div>
          A diferencia de los servidores eD2k centrales, <b>Kademlia</b> no depende de ningún servidor fijo: cada usuario es un nodo de búsqueda.
        </div>
        <div style="background: rgba(0,0,0,0.3); border-radius: 6px; padding: 8px; margin: 8px 0;">
          • <b>Estado UDP:</b> ${isUdp ? '🟢 Activo' : '🔴 Inactivo'}<br>
          • <b>Contactos en memoria:</b> <code>${kadCount} nodos</code>
        </div>
        <div style="margin-top: 6px; display: flex; gap: 6px;">
          <button class="btn btn-sm btn-primary" onclick="document.getElementById('btn-bootstrap-kad').click()" style="font-size: 0.74rem;">🚀 Bootstrap nodes.dat</button>
          <button class="btn btn-sm" onclick="switchNetworkView('kad')" style="font-size: 0.74rem;">Ir a Red Kademlia</button>
        </div>
      `;
    }
    return;
  }

  // 4. Diagnóstico en vivo de Salud y Rendimiento (Estado general)
  if (q.includes('estado') || q.includes('salud') || q.includes('general') || q.includes('resumen') || q.includes('velocidad')) {
    let serversStatus = null;
    let kadStatus = null;
    let dls = [];
    try {
      serversStatus = await client.call('servers.status');
      kadStatus = await client.call('kad.status');
      const dlRes = await client.call('downloads.list');
      dls = dlRes.downloads || [];
    } catch (err) { console.warn('[PulseCore] error en diagnóstico local:', err); }

    const activeDls = dls.filter(d => (d.status || '').toLowerCase() === 'downloading');
    const completedDls = dls.filter(d => (d.status || '').toLowerCase() === 'completed');
    const isEd2kConnected = serversStatus && serversStatus.status === 'connected';
    const isHighId = serversStatus && serversStatus.isHighId;

    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #38bdf8; margin-bottom: 6px;">📊 Telemetría y Diagnóstico del Sistema</div>
        <div style="display: flex; flex-direction: column; gap: 6px; margin: 8px 0;">
          <div style="padding: 6px 8px; background: rgba(0,0,0,0.25); border-radius: 4px;">
            <b>eD2k:</b> ${isEd2kConnected ? `🟢 Conectado (${isHighId ? 'HighID ✔' : 'LowID ⚠'})` : '⚪ Desconectado'}
          </div>
          <div style="padding: 6px 8px; background: rgba(0,0,0,0.25); border-radius: 4px;">
            <b>Kademlia:</b> ${kadStatus && kadStatus.isUdpActive ? `🟢 ${kadStatus.totalContacts || 0} nodos (${kadStatus.udpFirewallStatus || 'ok'})` : '⚪ Inactivo'}
          </div>
          <div style="padding: 6px 8px; background: rgba(0,0,0,0.25); border-radius: 4px;">
            <b>Descargas:</b> ${dls.length} en cola (${activeDls.length} activas, ${completedDls.length} completadas)
          </div>
        </div>
        ${!isHighId && isEd2kConnected ? '<div style="color: #fbbf24; font-size: 0.75rem;">⚠️ Estás en LowID. Pulsa abajo para ver la guía de apertura de puertos.</div>' : ''}
        <div style="margin-top: 8px; display: flex; gap: 6px;">
          <button class="btn btn-sm btn-primary" onclick="sendQuickPrompt('tutorial:router')" style="font-size: 0.74rem;">Ver Guía Puertos</button>
          <button class="btn btn-sm" onclick="cleanCompletedDownloads()" style="font-size: 0.74rem;">Limpiar Completadas</button>
        </div>
      `;
    }
    return;
  }

  // 5. Reanimar descargas lentas o paradas
  if (q.includes('lenta') || q.includes('parada') || q.includes('reanimar') || q.includes('fuentes')) {
    let restarted = 0;
    try {
      const res = await client.call('downloads.list');
      const dls = res.downloads || [];
      for (const d of dls) {
        const st = (d.status || '').toLowerCase();
        if (st === 'downloading' || st === 'waiting' || st === 'paused') {
          try {
            await client.call('downloads.retry', { id: d.id });
            restarted++;
          } catch (err) { console.warn('[PulseCore] error en diagnóstico local:', err); }
        }
      }
    } catch (err) { console.warn('[PulseCore] error en diagnóstico local:', err); }

    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #34d399; margin-bottom: 6px;">⚡ Acción Ejecutada: Reconsulta de Fuentes</div>
        <div>
          Se ha forzado la reconexión y petición de fuentes en servidores y red DHT para <b>${restarted}</b> descargas activas.
        </div>
        <div style="font-size: 0.75rem; color: var(--text-muted); margin-top: 6px;">
          Las fuentes eD2k y Kad responderán progresivamente durante los próximos minutos.
        </div>
      `;
    }
    return;
  }

  // 6. Limpieza de completadas
  if (q.includes('limpiar') || q.includes('limpieza') || q.includes('completada')) {
    cleanCompletedDownloads();
    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #38bdf8; margin-bottom: 6px;">🧹 Limpieza de Cola de Descargas</div>
        <div>
          Se han retirado las descargas completadas de la lista para mantener la interfaz limpia.
          Tus archivos permanecen intactos en la carpeta de destino o NAS.
        </div>
      `;
    }
    return;
  }

  // 7. Automatización con IA mediante servidor MCP
  if (q.includes('mcp') || q.includes('ia') || q.includes('claude') || q.includes('antigravity') || q.includes('gemini') || q.includes('modelo') || q.includes('llm') || q.includes('ollama') || q === 'tutorial:mcp') {
    const host = window.location.hostname || '127.0.0.1';
    const port = window.location.port || '4660';
    const body = ensureAssistantMessageBubble();
    if (body) {
      body.innerHTML = `
        <div style="font-weight: 700; color: #fbbf24; margin-bottom: 6px;">🤖 Automatización con IA mediante MCP</div>
        <div>
          Este panel es una <b>guía local</b> sin conexión a modelos de lenguaje. Para controlar Pulse Core con Inteligencia Artificial de forma segura se utiliza el servidor <b>Model Context Protocol</b> (<code>mule-mcp</code>).
        </div>
        <div style="background: rgba(0,0,0,0.3); border-radius: 6px; padding: 8px; margin: 8px 0; border: 1px solid rgba(255,255,255,0.08);">
          <div style="font-weight: 600; color: #38bdf8; margin-bottom: 4px;">Detalles de conexión MCP:</div>
          • <b>Binario:</b> <code>mule-mcp</code><br>
          • <b>Daemon RPC:</b> <code>ws://${escapeHtml(host)}:${escapeHtml(port)}/rpc</code><br>
          • <b>Clientes compatibles:</b> Antigravity, Claude Code, Gemini CLI, Cursor
        </div>
        <div style="font-size: 0.75rem; color: var(--text-muted);">
          Consulta la documentación en <code>wiki/11-Automatizacion-con-IA-y-MCP.md</code> para ver ejemplos de configuración y el catálogo de herramientas.
        </div>
      `;
    }
    return;
  }

  // Respuesta genérica de ayuda local
  const body = ensureAssistantMessageBubble();
  if (body) {
    body.innerHTML = `
      <div>
        Esta es la <b>guía local interactiva</b> de Pulse Core. Elige uno de los temas disponibles para consultar instrucciones o ejecutar diagnósticos locales:
      </div>
      <div style="margin-top: 8px; display: flex; flex-direction: column; gap: 5px;">
        <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:router')" style="text-align: left; font-size: 0.75rem;">🌐 <b>Abrir puertos en el router:</b> Guía paso a paso y UPnP</button>
        <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:links')" style="text-align: left; font-size: 0.75rem;">📥 <b>Cómo descargar:</b> Enlaces eD2k, magnets y torrents</button>
        <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:kad')" style="text-align: left; font-size: 0.75rem;">🚀 <b>Kademlia DHT:</b> Cómo funciona la red sin servidores</button>
        <button class="btn btn-sm" onclick="sendQuickPrompt('estado general')" style="text-align: left; font-size: 0.75rem;">📊 <b>Diagnóstico:</b> Salud de transferencias y redes</button>
        <button class="btn btn-sm" onclick="sendQuickPrompt('tutorial:mcp')" style="text-align: left; font-size: 0.75rem;">🤖 <b>IA y MCP:</b> Conectar asistentes mediante <code>mule-mcp</code></button>
      </div>
      <div style="font-size: 0.72rem; color: #94a3b8; margin-top: 8px;">
        <i>Para funciones de IA con lenguaje natural y herramientas, conecta tu cliente favorito al servidor <code>mule-mcp</code>.</i>
      </div>
    `;
  }
}

window.sendQuickPrompt = function(promptText) {
  const input = document.getElementById('ast-input');
  if (input) {
    input.value = promptText;
    sendAssistantMessage();
  }
};

window.handleAssistantKeyDown = function(e) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendAssistantMessage();
  }
};

// Filtros de descargas (Pestañas de estado sobre la tabla)
document.querySelectorAll('#filter-status-group .filter-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('#filter-status-group .filter-btn').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    currentFilterStatus = btn.dataset.filter;
    lastRenderedDownloadIds = '';
    renderDownloadsTable();
  });
});

// Filtros de protocolo (Segmented control a la derecha sobre la tabla)
document.querySelectorAll('#filter-proto-group .filter-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('#filter-proto-group .filter-btn').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    currentFilterProto = btn.dataset.proto;
    lastRenderedDownloadIds = '';
    renderDownloadsTable();
  });
});

const searchInput = document.getElementById('filter-dl-search');
if (searchInput) {
  searchInput.addEventListener('input', (e) => {
    currentSearchFilter = e.target.value.trim().toLowerCase();
    lastRenderedDownloadIds = '';
    renderDownloadsTable();
  });
}

// Navegación
let activeNetworkSubView = 'servers';

/**
 * Cambia entre la subvista de Servidores eD2k y Red Kademlia DHT bajo la pestaña Red P2P.
 */
function switchNetworkView(sub) {
  activeNetworkSubView = sub === 'kad' ? 'kad' : 'servers';
  const targetId = activeNetworkSubView === 'kad' ? 'tab-kad' : 'tab-servers';
  document.querySelectorAll('.nav-btn').forEach(b => b.classList.remove('active'));
  document.querySelectorAll('.view-panel').forEach(p => p.classList.remove('active'));
  const netNav = document.querySelector('.nav-btn[data-tab="tab-servers"]');
  if (netNav) netNav.classList.add('active');
  const target = document.getElementById(targetId);
  if (target) target.classList.add('active');
  if (activeNetworkSubView === 'kad') {
    refreshKad();
    refreshKadContacts();
  } else {
    refreshServers();
  }
}

document.querySelectorAll('.nav-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    if (btn.dataset.tab === 'tab-servers') {
      switchNetworkView(activeNetworkSubView);
      return;
    }
    document.querySelectorAll('.nav-btn').forEach(b => b.classList.remove('active'));
    document.querySelectorAll('.view-panel').forEach(p => p.classList.remove('active'));
    btn.classList.add('active');
    const target = document.getElementById(btn.dataset.tab);
    if (target) {
      target.classList.add('active');
      if (btn.dataset.tab === 'tab-kad') { refreshKad(); refreshKadContacts(); }
      if (btn.dataset.tab === 'tab-settings') loadPreferences();
      if (btn.dataset.tab === 'tab-media') loadMediaLibrary();
      if (btn.dataset.tab === 'tab-logs') refreshSystemLogs();
      if (btn.dataset.tab === 'tab-shares') refreshShares();
    }
  });
});

// Modal Add
const addModal = document.getElementById('add-modal');
document.getElementById('btn-open-add-modal').onclick = () => addModal.classList.add('active');
document.getElementById('btn-close-modal').onclick = () => addModal.classList.remove('active');
document.getElementById('btn-submit-link').onclick = async () => {
  const rawText = document.getElementById('ed2k-input').value.trim();
  if (!rawText) return alert('Por favor pega uno o varios enlaces ed2k:// o magnet: válidos');

  const categoryChoice = document.getElementById('ed2k-category')?.value || 'auto';
  const initialPriority = document.getElementById('ed2k-priority')?.value || 'auto';

  // Extraer todos los enlaces ed2k://|file|... y magnet:?xt=...
  const lines = rawText.split(/\r?\n/).map(l => l.trim()).filter(l => l.length > 0);
  const links = [];
  for (const line of lines) {
    const ed2kMatches = line.match(/ed2k:\/\/\|file\|[^|]+\|\d+\|[a-fA-F0-9]{32}\|[^\s]*/g);
    if (ed2kMatches) {
      links.push(...ed2kMatches);
    } else if (line.startsWith('magnet:?')) {
      links.push(line);
    } else if (line.startsWith('ed2k://')) {
      links.push(line);
    }
  }

  if (links.length === 0) {
    return alert('No se detectaron enlaces eD2k ni Magnet válidos en el texto.');
  }

  function detectCategoryFromLink(link) {
    const lower = link.toLowerCase();
    if (/\.(mkv|mp4|avi|mov|wmv|mpg|mpeg|m4v)/.test(lower)) return 'video';
    if (/\.(mp3|flac|wav|aac|ogg|m4a|alac|opus)/.test(lower)) return 'audio';
    if (/\.(iso|zip|rar|7z|tar|gz|bin|cue)/.test(lower)) return 'software';
    if (/\.(pdf|epub|mobi|cbz|cbr|txt)/.test(lower)) return 'docs';
    return undefined;
  }

  const submitBtn = document.getElementById('btn-submit-link');
  submitBtn.disabled = true;
  submitBtn.textContent = `Añadiendo ${links.length}...`;

  let addedCount = 0;
  let errorCount = 0;
  for (const link of links) {
    let cat = categoryChoice;
    if (cat === 'auto') {
      cat = detectCategoryFromLink(link);
    } else if (!cat) {
      cat = undefined;
    }

    try {
      await client.call('downloads.addEd2kLink', {
        link,
        categoryId: cat,
        priority: initialPriority
      });
      addedCount++;
    } catch (e) {
      errorCount++;
      console.error('Error añadiendo enlace:', link, e);
    }
  }

  submitBtn.disabled = false;
  submitBtn.textContent = 'Descargar';
  document.getElementById('ed2k-input').value = '';
  addModal.classList.remove('active');
  refreshDownloads();

  if (addedCount > 0) {
    const msg = addedCount === 1 ? 'Descarga añadida a la cola' : `${addedCount} descargas añadidas a la cola`;
    addLog('success', msg);
    showToast(`✅ ${msg}`);
  }
  if (errorCount > 0) {
    addLog('warn', `Hubo ${errorCount} enlaces que no se pudieron procesar.`);
  }
};

// Modal NAS Details
const nasModal = document.getElementById('nas-details-modal');
const storagePillNas = document.getElementById('storage-pill-nas');
if (storagePillNas && nasModal) {
  storagePillNas.onclick = () => {
    nasModal.classList.add('active');
    refreshNasTelemetry(true);
  };
}
const btnCloseNasModal = document.getElementById('btn-close-nas-modal');
if (btnCloseNasModal && nasModal) {
  btnCloseNasModal.onclick = () => nasModal.classList.remove('active');
}
const btnRefreshNasModal = document.getElementById('btn-refresh-nas-modal');
if (btnRefreshNasModal) {
  btnRefreshNasModal.onclick = async () => {
    btnRefreshNasModal.disabled = true;
    btnRefreshNasModal.textContent = '⏳ Leyendo NAS...';
    await refreshNasTelemetry(true);
    btnRefreshNasModal.disabled = false;
    btnRefreshNasModal.textContent = '🔄 Actualizar datos';
  };
}
if (nasModal) {
  nasModal.onclick = (e) => {
    if (e.target === nasModal) nasModal.classList.remove('active');
  };
}

let activeOptionsTabId = 'opt-folders';

function switchOptionsTab(tabId) {
  activeOptionsTabId = tabId;
  const searchInput = document.getElementById('opt-search-input');
  if (searchInput && searchInput.value) {
    searchInput.value = '';
    const section = document.getElementById('tab-settings');
    if (section) delete section.dataset.optSearching;
    document.querySelectorAll('#tab-settings .opt-card').forEach(c => c.style.display = '');
  }
  document.querySelectorAll('.opt-subpanel').forEach(p => p.style.display = 'none');
  document.querySelectorAll('#options-nav-cluster .settings-nav-btn, #options-nav-cluster .filter-btn').forEach(b => b.classList.remove('active'));
  const target = document.getElementById(tabId);
  if (target) target.style.display = 'block';
  const btn = document.getElementById('btn-' + tabId);
  if (btn) btn.classList.add('active');
  if (tabId === 'opt-notifications' && typeof loadNotificationConfig === 'function') {
    loadNotificationConfig();
  }
}

function setOptionsViewMode(mode) {
  const normalized = mode === 'advanced' ? 'advanced' : 'essential';
  try { localStorage.setItem('pulsecore_opt_view_mode', normalized); } catch (err) { console.debug('[PulseCore] localStorage no disponible:', err); }
  const section = document.getElementById('tab-settings');
  if (section) section.dataset.optView = normalized;
  document.querySelectorAll('#opt-mode-toggle .opt-mode-btn').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-opt-mode') === normalized);
  });
}

function filterOptionsBySearch(rawQuery) {
  const q = String(rawQuery || '').trim().toLowerCase();
  const section = document.getElementById('tab-settings');
  if (!q) {
    if (section) delete section.dataset.optSearching;
    document.querySelectorAll('#tab-settings .opt-card').forEach(c => c.style.display = '');
    switchOptionsTab(activeOptionsTabId || 'opt-folders');
    return;
  }
  if (section) section.dataset.optSearching = 'true';
  document.querySelectorAll('.opt-subpanel').forEach(panel => {
    let anyCardVisible = false;
    panel.querySelectorAll('.opt-card').forEach(card => {
      const text = (card.textContent || '').toLowerCase();
      const match = text.includes(q);
      card.style.display = match ? '' : 'none';
      if (match) anyCardVisible = true;
    });
    panel.style.display = anyCardVisible ? 'block' : 'none';
  });
}

function setStorageLocationMode(mode) {
  const isNas = mode === 'nas';
  const localCard = document.getElementById('storage-mode-local');
  const nasCard = document.getElementById('storage-mode-nas');
  if (localCard) localCard.classList.toggle('active', !isNas);
  if (nasCard) nasCard.classList.toggle('active', isNas);

  const lbl = document.getElementById('lbl-nas-root-dir');
  const help = document.getElementById('help-nas-root-dir');
  const input = document.getElementById('pref-nas-root-dir');
  const nasWrap = document.getElementById('nas-only-settings-wrap');
  const reqNfs = document.getElementById('pref-nas-require-nfs');

  if (lbl) lbl.textContent = isNas ? 'Punto de montaje del NAS (NFS / SMB)' : 'Carpeta principal de tu Mediateca';
  if (help) {
    help.innerHTML = isNas
      ? 'Ruta donde está montado el NAS. Autodetecta subcarpetas como <code>Movies</code>/<code>Películas</code> y <code>Series</code>/<code>TV Shows</code>.'
      : 'PulseCore organizará aquí tus películas y series automáticamente. Detecta subcarpetas como <code>Movies</code>/<code>Películas</code> y <code>Series</code>/<code>TV Shows</code>.';
  }
  if (input) input.placeholder = isNas ? '/mnt/nas' : 'Ej. /mnt/hdd/Multimedia o D:\\Media';
  if (nasWrap) nasWrap.style.display = isNas ? 'block' : 'none';
  if (!isNas && reqNfs) reqNfs.checked = false;
}

function markPreferencesDirty() {
  const unsaved = document.getElementById('pref-unsaved-badge');
  const saved = document.getElementById('pref-saved-badge');
  if (unsaved) unsaved.style.display = 'inline-flex';
  if (saved) saved.style.display = 'none';
}

(function initOptionsEnhancements() {
  try {
    const savedMode = localStorage.getItem('pulsecore_opt_view_mode');
    setOptionsViewMode(savedMode === 'advanced' ? 'advanced' : 'essential');
  } catch (err) { console.debug('[PulseCore] localStorage no disponible:', err); }
  const settingsEl = document.getElementById('tab-settings');
  if (settingsEl) {
    settingsEl.addEventListener('input', (e) => {
      const id = e.target && e.target.id;
      if (id === 'opt-search-input' || id === 'mcp-tool-search') return;
      markPreferencesDirty();
    });
    settingsEl.addEventListener('change', (e) => {
      const id = e.target && e.target.id;
      if (id === 'opt-search-input' || id === 'mcp-tool-search') return;
      markPreferencesDirty();
    });
  }
})();

function updateRamProfileBadge() {
  const select = document.getElementById('pref-ram-profile');
  const badge = document.getElementById('ram-profile-badge');
  if (!select || !badge) return;
  const val = select.value;
  if (val === 'low') {
    badge.textContent = 'Bajo consumo (~25MB)';
    badge.style.background = 'rgba(34, 197, 94, 0.2)';
    badge.style.color = '#4ade80';
  } else if (val === 'high') {
    badge.textContent = 'Alto rendimiento (~150MB+)';
    badge.style.background = 'rgba(239, 68, 68, 0.2)';
    badge.style.color = '#f87171';
  } else {
    badge.textContent = 'Equilibrado (~60MB)';
    badge.style.background = 'rgba(99, 102, 241, 0.2)';
    badge.style.color = '#818cf8';
  }
}

function renderHardlinkStatus(hardlinkCapable) {
  const box = document.getElementById('hardlink-status-box');
  if (!box) return;
  if (hardlinkCapable === true) {
    box.style.display = 'block';
    box.style.borderColor = 'rgba(34, 197, 94, 0.38)';
    box.style.background = 'rgba(34, 197, 94, 0.10)';
    box.style.color = '#86efac';
    box.innerHTML = '🟢 <strong>Mismo volumen detectado:</strong> enlaces duros (<em>hardlinks</em>) instantáneos activos entre descargas y mediateca, sin duplicar espacio en disco.';
  } else if (hardlinkCapable === false) {
    box.style.display = 'block';
    box.style.borderColor = 'rgba(245, 158, 11, 0.45)';
    box.style.background = 'rgba(245, 158, 11, 0.10)';
    box.style.color = '#fde68a';
    box.innerHTML = '⚠️ <strong>Volúmenes o discos distintos (EXDEV):</strong> el organizador tendrá que copiar los archivos completos en lugar de crear enlaces duros. Si usas Docker o NAS, monta descargas y mediateca dentro del mismo volumen (p. ej. <code>/data</code>).';
  } else {
    box.style.display = 'none';
    box.innerHTML = '';
  }
}

function renderNfsMountStatus(nfsStatus, discoveredMounts) {
  const box = document.getElementById('nfs-mount-status-box');
  if (!box) return;

  let statusHtml = '';
  if (nfsStatus) {
    let icon = '🟡';
    let color = '#fbbf24';
    let title = `Almacenamiento local (${escapeHtml(nfsStatus.targetPath || '')})`;
    if (nfsStatus.unmountedFstabTrap) {
      icon = '🔴';
      color = '#f87171';
      const exp = nfsStatus.remoteExport ? ` (${escapeHtml(nfsStatus.remoteExport)})` : '';
      title = `Montaje NFS/red en /etc/fstab desmontado${exp} — escritura bloqueada para proteger el disco local`;
    } else if (nfsStatus.nfsPortReachable === false) {
      icon = '🔴';
      color = '#f87171';
      const host = nfsStatus.serverHost ? `${escapeHtml(nfsStatus.serverHost)}:2049` : 'puerto 2049';
      title = `Servidor NFS no responde en TCP ${host} — transferencias al NAS pausadas`;
    } else if (nfsStatus.isNetworkMount) {
      icon = '🟢';
      color = '#4ade80';
      const kind = escapeHtml((nfsStatus.fsKind || 'nfs').toUpperCase());
      const exp = escapeHtml(nfsStatus.remoteExport || nfsStatus.mountPoint || '');
      const portTag = nfsStatus.nfsPortReachable === true ? ' · TCP 2049 OK' : '';
      title = `Montaje ${kind} activo: ${exp}${portTag}`;
    }
    const subdirs = `<div style="color: #94a3b8; font-size: 0.74rem; margin-top: 4px; word-break: break-all;">🎬 Películas: <code>${escapeHtml(nfsStatus.resolvedMoviesDir || '')}</code> · 📺 Series: <code>${escapeHtml(nfsStatus.resolvedSeriesDir || '')}</code></div>`;
    statusHtml += `<div style="color: ${color}; font-weight: 600;">${icon} ${title}</div>${subdirs}`;
  }

  if (Array.isArray(discoveredMounts) && discoveredMounts.length > 0) {
    statusHtml += `<div style="margin-top: 8px; padding-top: 8px; border-top: 1px solid rgba(148,163,184,0.18);">
      <div style="color: #cbd5e1; font-size: 0.74rem; margin-bottom: 6px;">Montajes de red detectados en el sistema (clic para usar):</div>
      <div style="display: flex; flex-wrap: wrap; gap: 6px;">`;
    discoveredMounts.forEach((m, idx) => {
      const stateBadge = m.mounted ? '🟢' : '⚠️ Desmontado';
      statusHtml += `<button type="button" class="btn btn-sm btn-select-nfs-mount" data-mount-idx="${idx}" style="font-size: 0.74rem; padding: 4px 8px;">${stateBadge} Usar ${escapeHtml(m.mountPoint)} (${escapeHtml(m.remoteExport)} [${escapeHtml(m.fsType)}])</button>`;
    });
    statusHtml += `</div></div>`;
  }

  if (!statusHtml) {
    box.style.display = 'none';
    box.innerHTML = '';
    return;
  }
  box.style.display = 'block';
  box.innerHTML = statusHtml;

  box.querySelectorAll('.btn-select-nfs-mount').forEach(btn => {
    btn.addEventListener('click', () => {
      const idx = parseInt(btn.getAttribute('data-mount-idx'), 10);
      const mount = discoveredMounts[idx];
      if (!mount) return;
      const nasInput = document.getElementById('pref-nas-root-dir');
      const movInput = document.getElementById('pref-movies-dir');
      const serInput = document.getElementById('pref-series-dir');
      if (nasInput) nasInput.value = mount.mountPoint;
      if (movInput) {
        movInput.value = '';
        movInput.placeholder = `${mount.mountPoint.replace(/\/+$/, '')}/Movies (se autodetectará al guardar)`;
      }
      if (serInput) {
        serInput.value = '';
        serInput.placeholder = `${mount.mountPoint.replace(/\/+$/, '')}/Series (se autodetectará al guardar)`;
      }
    });
  });
}

async function loadPreferences() {
  try {
    const s = await client.call('settings.get');
    if (!s) return;

    const setVal = (id, val) => { const el = document.getElementById(id); if (el && val !== undefined && val !== null) el.value = val; };
    const setChk = (id, val) => { const el = document.getElementById(id); if (el && val !== undefined && val !== null) el.checked = !!val; };

    // Directorios
    window.__lastDefaultDownloadDir = s.defaultDownloadDir || '';
    setVal('pref-download-dir', s.defaultDownloadDir || '');
    setVal('pref-temp-dir', s.tempDir || '');
    setChk('pref-share-download-dir', s.shareDownloadDir !== false);
    setVal('pref-nas-root-dir', s.nasRootDir || '');
    setChk('pref-nas-require-nfs', !!s.nasRequireNfs);
    setVal('pref-movies-dir', s.moviesDir || '');
    setVal('pref-series-dir', s.seriesDir || '');
    const movEl = document.getElementById('pref-movies-dir');
    if (movEl) movEl.placeholder = s.effectiveMoviesDir || 'Autodetectada dentro de la raíz del NAS';
    const serEl = document.getElementById('pref-series-dir');
    if (serEl) serEl.placeholder = s.effectiveSeriesDir || 'Autodetectada dentro de la mediateca';
    const nasRootStr = String(s.nasRootDir || '').trim().toLowerCase();
    const initialStorageMode = (s.nasRequireNfs || (s.nfsStatus && s.nfsStatus.isNetworkMount) || nasRootStr.startsWith('/mnt/nas') || nasRootStr.startsWith('//')) ? 'nas' : 'local';
    setStorageLocationMode(initialStorageMode);
    renderNfsMountStatus(s.nfsStatus, s.discoveredMounts);
    renderHardlinkStatus(s.hardlinkCapable);
    if (typeof s.runningInDocker === 'boolean' && typeof window._applyDockerOptionsNotices === 'function') {
      window._applyDockerOptionsNotices(s.runningInDocker, s.workdirPath);
    }

    // Conexión & Caudal
    setVal('pref-cap-down', s.capacityDownloadKbs || '');
    setVal('pref-cap-up', s.capacityUploadKbs || '');
    setVal('pref-min-upload', s.minUploadKbs || 10);
    setChk('pref-uss-enabled', s.ussEnabled);
    setVal('pref-idle-upload-cap', s.idleUploadCapKbs || '');
    setVal('pref-streaming-upload-cap', s.streamingUploadCapKbs || '');
    setVal('pref-smart-slot-target', s.smartSlotTargetKbs || '');
    setVal('pref-quota-eco-upload', s.quotaEcoUploadKbs || '');
    setVal('pref-max-global', s.maxGlobalDownloadPeers || s.maxConnections || 500);
    setVal('pref-max-conn-min', s.maxConnPerMin || 60);
    setVal('pref-max-per-file', s.maxPeersPerDownload || 300);
    setVal('pref-max-active-downloads', s.maxActiveDownloads || 30);
    setVal('pref-max-upload-queue', s.maxUploadQueue || 1000);
    setVal('pref-daily-quota', s.dailyUploadQuotaGb || '');
    if (s.ramProfile) {
      setVal('pref-ram-profile', s.ramProfile);
      updateRamProfileBadge();
    }

    // Redes P2P
    setVal('pref-ed2k-port', s.ed2kPort || 4662);
    setVal('pref-kad-port', s.kadPort || 4672);
    setChk('pref-upnp-enabled', s.upnpEnabled);
    setChk('pref-udp-enabled', s.udpEnabled !== false);
    setChk('pref-server-reconnect', s.serverReconnect !== false);
    setChk('pref-server-autoconnect', !!s.serverAutoconnect);
    setChk('pref-server-autoremove-dead', s.serverAutoremoveDead !== false);
    setChk('pref-safe-connect', !!s.safeConnect);
    setVal('pref-server-autoupdate-path', s.serverAutoupdatePath || '');

    // Servarr
    selectOrganizeMode(s.organizeEnabled !== false);
    if (s.mediaServerProfile) setVal('pref-media-server-profile', s.mediaServerProfile);
    setChk('pref-generate-nfo-artwork', s.generateNfoAndArtwork !== false);
    setVal('pref-jellyfin-url', s.jellyfinUrl || '');
    setVal('pref-jellyfin-apikey', s.jellyfinApiKey || '');
    setVal('pref-plex-url', s.plexUrl || '');
    setVal('pref-plex-token', s.plexToken || '');
    setVal('pref-kodi-url', s.kodiUrl || '');
    setChk('pref-auto-grabber-enabled', s.autoGrabberEnabled !== false);
    setChk('pref-quality-upgrade-enabled', s.qualityUpgradeEnabled !== false);
    if (s.qualityMaxResolution) setVal('pref-quality-max-resolution', s.qualityMaxResolution);
    setChk('pref-scorer-anti-fake', s.scorerAntiFake !== false);
    setChk('pref-cleaner-enabled', s.cleanerEnabled !== false);
    setChk('pref-subtitles-auto', s.subtitlesAutoDownload !== false);
    if (s.subtitlesLanguage) setVal('pref-subtitles-language', s.subtitlesLanguage);

    // Seguridad & Privacidad
    setChk('pref-obf-enabled', !!s.obfuscationEnabled);
    setChk('pref-obf-required', !!s.obfuscationRequired);
    setChk('pref-paranoid-mode', !!s.paranoidMode);
    if (s.viewSharedFiles) setVal('pref-view-shared', s.viewSharedFiles);
    setVal('pref-identity-rotation', s.identityRotationHours || '');
    setChk('pref-ich-enabled', s.ichEnabled !== false);
    setChk('pref-swarm-healing', s.swarmHealing !== false);
    setChk('pref-remember-cancelled', s.rememberCancelled !== false);
    setChk('pref-remember-downloaded', s.rememberDownloaded !== false);
    setVal('pref-retention-days', s.historyRetentionDays || 30);

    const webuiPassEl = document.getElementById('pref-webui-password');
    if (webuiPassEl) webuiPassEl.value = '';
    const webuiBadge = document.getElementById('webui-pass-status-badge');
    const clearWebuiBtn = document.getElementById('btn-clear-webui-password');
    const hasWebuiPass = Boolean(s.webuiPasswordEnabled);
    if (webuiBadge) {
      webuiBadge.textContent = hasWebuiPass ? '🔒 Protegida en LAN' : 'Sin contraseña';
      webuiBadge.style.color = hasWebuiPass ? '#4ade80' : 'var(--text-muted)';
      webuiBadge.style.background = hasWebuiPass ? 'rgba(34, 197, 94, 0.14)' : 'rgba(148, 163, 184, 0.14)';
      webuiBadge.style.borderColor = hasWebuiPass ? 'rgba(34, 197, 94, 0.35)' : 'var(--border)';
    }
    if (clearWebuiBtn) {
      clearWebuiBtn.style.display = hasWebuiPass ? 'inline-flex' : 'none';
    }

    // Integración & API
    setVal('pref-jellyfin-url', s.jellyfinUrl || '');
    setVal('pref-nickname', s.nickname || 'eMuleRust');

    // Protocolo MCP & Agentes IA
    setChk('pref-mcp-enabled', s.mcpEnabled !== false);
    if (s.mcpToolProfile) setVal('pref-mcp-profile', s.mcpToolProfile);
    if (s.authToken) {
      window.mcpAuthToken = s.authToken;
    }
    updateMcpProfileExplainer();
    updateMcpSnippets();
    renderMcpTools();
  } catch (e) {
    console.error('Error loading preferences:', e);
  }
}

function selectOrganizeMode(enabled) {
  const isMediaCenter = !!enabled;
  const hidden = document.getElementById('pref-organize-enabled');
  if (hidden) hidden.value = isMediaCenter ? 'true' : 'false';
  document.getElementById('organize-mode-card-mediacenter')?.classList.toggle('active', isMediaCenter);
  document.getElementById('organize-mode-card-simple')?.classList.toggle('active', !isMediaCenter);
  const details = document.getElementById('opt-mediacenter-details');
  if (details) details.style.display = isMediaCenter ? 'block' : 'none';
}
window.selectOrganizeMode = selectOrganizeMode;

async function savePreferences() {
  const btn = document.getElementById('btn-save-preferences');
  const badge = document.getElementById('pref-saved-badge');
  if (btn) {
    btn.disabled = true;
    btn.textContent = 'Guardando...';
  }
  try {
    const payload = {};
    const getStr = id => { const el = document.getElementById(id); return el ? el.value.trim() : null; };
    const getInt = id => { const el = document.getElementById(id); if (!el) return null; const n = parseInt(el.value, 10); return isNaN(n) ? null : n; };
    const getBool = id => { const el = document.getElementById(id); return el ? el.checked : null; };

    // Directorios
    const dlDir = getStr('pref-download-dir');
    if (dlDir !== null) payload.defaultDownloadDir = dlDir;
    const tempDir = getStr('pref-temp-dir');
    if (tempDir !== null) payload.tempDir = tempDir;
    const shareDl = getBool('pref-share-download-dir');
    if (shareDl !== null) payload.shareDownloadDir = shareDl;
    const nasDir = getStr('pref-nas-root-dir');
    if (nasDir !== null) payload.nasRootDir = nasDir;
    const nasReqNfs = getBool('pref-nas-require-nfs');
    if (nasReqNfs !== null) payload.nasRequireNfs = nasReqNfs;
    let movDir = getStr('pref-movies-dir');
    let serDir = getStr('pref-series-dir');
    if (nasDir) {
      const baseLocal = (window.__lastDefaultDownloadDir || dlDir || '').replace(/\/+$/, '');
      if (movDir && baseLocal && movDir === `${baseLocal}/media/Movies`) movDir = '';
      if (serDir && baseLocal && serDir === `${baseLocal}/media/Series`) serDir = '';
    }
    if (movDir !== null) payload.moviesDir = movDir;
    if (serDir !== null) payload.seriesDir = serDir;

    // Conexión
    const capDown = getInt('pref-cap-down');
    if (capDown !== null) payload.capacityDownloadKbs = capDown;
    const capUp = getInt('pref-cap-up');
    if (capUp !== null) payload.capacityUploadKbs = capUp;
    const minUp = getInt('pref-min-upload');
    if (minUp !== null) payload.minUploadKbs = minUp;
    const uss = getBool('pref-uss-enabled');
    if (uss !== null) payload.ussEnabled = uss;
    const idleCap = getInt('pref-idle-upload-cap');
    payload.idleUploadCapKbs = idleCap !== null ? idleCap : 0;
    const streamCap = getInt('pref-streaming-upload-cap');
    payload.streamingUploadCapKbs = streamCap !== null ? streamCap : 0;
    const slotTarget = getInt('pref-smart-slot-target');
    payload.smartSlotTargetKbs = slotTarget !== null ? slotTarget : 0;
    const quotaEco = getInt('pref-quota-eco-upload');
    payload.quotaEcoUploadKbs = quotaEco !== null ? quotaEco : 0;
    const maxG = getInt('pref-max-global');
    if (maxG !== null) { payload.maxGlobalDownloadPeers = maxG; payload.maxConnections = maxG; }
    const maxCMin = getInt('pref-max-conn-min');
    if (maxCMin !== null) payload.maxConnPerMin = maxCMin;
    const maxF = getInt('pref-max-per-file');
    if (maxF !== null) payload.maxPeersPerDownload = maxF;
    const maxA = getInt('pref-max-active-downloads');
    if (maxA !== null && maxA > 0) payload.maxActiveDownloads = maxA;
    const maxQ = getInt('pref-max-upload-queue');
    if (maxQ !== null) payload.maxUploadQueue = maxQ;
    const dailyQ = getInt('pref-daily-quota');
    if (dailyQ !== null) payload.dailyUploadQuotaGb = dailyQ;
    const ramProf = getStr('pref-ram-profile');
    if (ramProf) payload.ramProfile = ramProf;

    // Redes P2P
    const ed2kP = getInt('pref-ed2k-port');
    if (ed2kP !== null && ed2kP > 0) payload.ed2kPort = ed2kP;
    const kadP = getInt('pref-kad-port');
    if (kadP !== null && kadP > 0) payload.kadPort = kadP;
    const upnp = getBool('pref-upnp-enabled');
    if (upnp !== null) payload.upnpEnabled = upnp;
    const udp = getBool('pref-udp-enabled');
    if (udp !== null) payload.udpEnabled = udp;
    const srvRec = getBool('pref-server-reconnect');
    if (srvRec !== null) payload.serverReconnect = srvRec;
    const srvAuto = getBool('pref-server-autoconnect');
    if (srvAuto !== null) payload.serverAutoconnect = srvAuto;
    const srvDead = getBool('pref-server-autoremove-dead');
    if (srvDead !== null) payload.serverAutoremoveDead = srvDead;
    const safeConn = getBool('pref-safe-connect');
    if (safeConn !== null) payload.safeConnect = safeConn;
    const srvUpd = getStr('pref-server-autoupdate-path');
    if (srvUpd !== null) payload.serverAutoupdatePath = srvUpd;

    // Servarr
    const orgEl = document.getElementById('pref-organize-enabled');
    if (orgEl) payload.organizeEnabled = orgEl.value !== 'false';
    const msProf = getStr('pref-media-server-profile');
    if (msProf) payload.mediaServerProfile = msProf;
    const nfoEn = getBool('pref-generate-nfo-artwork');
    if (nfoEn !== null) payload.generateNfoAndArtwork = nfoEn;
    const jfUrl = getStr('pref-jellyfin-url');
    if (jfUrl !== null) payload.jellyfinUrl = jfUrl;
    const jfKey = getStr('pref-jellyfin-apikey');
    if (jfKey !== null) payload.jellyfinApiKey = jfKey;
    const pxUrl = getStr('pref-plex-url');
    if (pxUrl !== null) payload.plexUrl = pxUrl;
    const pxTok = getStr('pref-plex-token');
    if (pxTok !== null) payload.plexToken = pxTok;
    const kdUrl = getStr('pref-kodi-url');
    if (kdUrl !== null) payload.kodiUrl = kdUrl;
    const autoGrab = getBool('pref-auto-grabber-enabled');
    if (autoGrab !== null) payload.autoGrabberEnabled = autoGrab;
    const upgEn = getBool('pref-quality-upgrade-enabled');
    if (upgEn !== null) payload.qualityUpgradeEnabled = upgEn;
    const maxRes = getStr('pref-quality-max-resolution');
    if (maxRes) payload.qualityMaxResolution = maxRes;
    const antiFake = getBool('pref-scorer-anti-fake');
    if (antiFake !== null) payload.scorerAntiFake = antiFake;
    const cleaner = getBool('pref-cleaner-enabled');
    if (cleaner !== null) payload.cleanerEnabled = cleaner;
    const subAuto = getBool('pref-subtitles-auto');
    if (subAuto !== null) payload.subtitlesAutoDownload = subAuto;
    const subLang = getStr('pref-subtitles-language');
    if (subLang) payload.subtitlesLanguage = subLang;

    // Seguridad & Privacidad
    const obfEn = getBool('pref-obf-enabled');
    if (obfEn !== null) payload.obfuscationEnabled = obfEn;
    const obfReq = getBool('pref-obf-required');
    if (obfReq !== null) payload.obfuscationRequired = obfReq;
    const paranoid = getBool('pref-paranoid-mode');
    if (paranoid !== null) payload.paranoidMode = paranoid;
    const viewShared = getStr('pref-view-shared');
    if (viewShared) payload.viewSharedFiles = viewShared;
    const idRot = getInt('pref-identity-rotation');
    if (idRot !== null) payload.identityRotationHours = idRot;
    const ich = getBool('pref-ich-enabled');
    if (ich !== null) payload.ichEnabled = ich;
    const healing = getBool('pref-swarm-healing');
    if (healing !== null) payload.swarmHealing = healing;
    const remCanc = getBool('pref-remember-cancelled');
    if (remCanc !== null) payload.rememberCancelled = remCanc;
    const remDl = getBool('pref-remember-downloaded');
    if (remDl !== null) payload.rememberDownloaded = remDl;
    const retDays = getInt('pref-retention-days');
    if (retDays !== null) payload.historyRetentionDays = retDays;

    const webuiPassEl = document.getElementById('pref-webui-password');
    const webuiPassVal = webuiPassEl ? webuiPassEl.value : '';
    if (webuiPassVal && webuiPassVal.length > 0) {
      payload.webuiPassword = webuiPassVal;
    }

    // Integración & API
    const jelly = getStr('pref-jellyfin-url');
    if (jelly !== null) payload.jellyfinUrl = jelly;
    const nick = getStr('pref-nickname');
    if (nick) payload.nickname = nick;

    // Protocolo MCP
    const mcpEn = getBool('pref-mcp-enabled');
    if (mcpEn !== null) payload.mcpEnabled = mcpEn;
    const mcpProf = getStr('pref-mcp-profile');
    if (mcpProf) payload.mcpToolProfile = mcpProf;

    await client.call('settings.set', payload);
    if (typeof saveNotificationConfig === 'function') {
      await saveNotificationConfig(true);
    }
    if (payload.webuiPassword) {
      const webuiPassInput = document.getElementById('pref-webui-password');
      if (webuiPassInput) webuiPassInput.value = '';
    }
    await loadPreferences();
    refreshStorage();
    const unsavedBadge = document.getElementById('pref-unsaved-badge');
    if (unsavedBadge) unsavedBadge.style.display = 'none';
    if (badge) {
      badge.style.display = 'inline-block';
      setTimeout(() => { badge.style.display = 'none'; }, 3500);
    }
  } catch (e) {
    alert('Error al guardar opciones: ' + (e.message || JSON.stringify(e)));
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '💾 Guardar Opciones';
    }
  }
}

if (document.getElementById('btn-save-preferences')) {
  document.getElementById('btn-save-preferences').onclick = savePreferences;
}

// === ESCÁNER DE ANCHO DE BANDA Y PERFILES INTELIGENTES DE VELOCIDAD ===
let lastBandwidthScanResult = null;
let selectedSmartProfileId = localStorage.getItem('pulsecore_smart_speed_profile') || '';

function getSmartScanBaseContext() {
  const hint = document.getElementById('bw-line-hint')?.value || 'auto';
  const hostMemBytes = lastDiskSpaceInfo?.host?.memTotalBytes || 0;
  const hostRamGb = hostMemBytes > 0 ? hostMemBytes / (1024 * 1024 * 1024) : 4.0;
  const hostOs = lastDiskSpaceInfo?.host?.os || 'linux';
  const isNas = Boolean(lastDiskSpaceInfo?.nas?.nfs?.isNetworkMount);

  if (lastBandwidthScanResult && hint === 'auto') {
    return lastBandwidthScanResult;
  }

  let downMbps = 300;
  let upMbps = 300;
  if (hint !== 'auto') {
    const n = parseInt(hint, 10) || 300;
    downMbps = n;
    upMbps = n;
  } else if (lastBandwidthScanResult) {
    downMbps = lastBandwidthScanResult.downMbps;
    upMbps = lastBandwidthScanResult.upMbps;
  }

  return {
    downMbps,
    upMbps,
    rttMs: lastBandwidthScanResult ? lastBandwidthScanResult.rttMs : 12,
    hostRamGb,
    hostOs,
    isNas,
    isLowRamDevice: hostRamGb > 0 && hostRamGb <= 4.5,
    measured: Boolean(lastBandwidthScanResult)
  };
}

function buildSmartSpeedProfiles(ctx) {
  const capDownKbs = Math.round((ctx.downMbps * 1000) / 8);
  const capUpKbs = Math.round((ctx.upMbps * 1000) / 8);
  const isLowRam = ctx.isLowRamDevice;

  return [
    {
      id: 'eco',
      icon: '🌱',
      title: 'Eco · 24/7 Silencioso',
      tag: isLowRam ? 'Ideal RPi / NAS' : 'Bajo consumo',
      desc: 'Prioriza estabilidad térmica, disco/NAS y navegación sin saturación.',
      capDownKbs,
      capUpKbs,
      dlLimitKbs: Math.max(512, Math.round(capDownKbs * 0.75)),
      ulLimitKbs: Math.max(128, Math.round(capUpKbs * 0.55)),
      ussEnabled: true,
      idleCapKbs: Math.max(128, Math.round(capUpKbs * 0.45)),
      streamCapKbs: Math.max(64, Math.round(capUpKbs * 0.30)),
      smartSlotKbs: 128,
      maxGlobal: 280,
      maxConnMin: 45,
      maxPerFile: 180,
      maxActive: 15,
      ramProfile: 'low'
    },
    {
      id: 'balanced',
      icon: '⚖️',
      title: 'Equilibrado Inteligente',
      tag: 'Recomendado',
      desc: 'Máxima bajada con USS/LEDBAT++ controlando la latencia de subida.',
      capDownKbs,
      capUpKbs,
      dlLimitKbs: 0,
      ulLimitKbs: Math.max(256, Math.round(capUpKbs * 0.80)),
      ussEnabled: true,
      idleCapKbs: 0,
      streamCapKbs: Math.max(128, Math.round(capUpKbs * 0.45)),
      smartSlotKbs: 256,
      maxGlobal: 500,
      maxConnMin: 60,
      maxPerFile: 300,
      maxActive: 25,
      ramProfile: isLowRam ? 'balanced' : 'balanced'
    },
    {
      id: 'streaming',
      icon: '🎬',
      title: 'Prioridad Streaming',
      tag: 'Latencia cero',
      desc: 'Recorta la subida automáticamente al reproducir en Jellyfin, Plex o Kodi.',
      capDownKbs,
      capUpKbs,
      dlLimitKbs: Math.max(1024, Math.round(capDownKbs * 0.85)),
      ulLimitKbs: Math.max(200, Math.round(capUpKbs * 0.70)),
      ussEnabled: true,
      idleCapKbs: Math.max(200, Math.round(capUpKbs * 0.70)),
      streamCapKbs: Math.max(64, Math.round(capUpKbs * 0.22)),
      smartSlotKbs: 192,
      maxGlobal: 400,
      maxConnMin: 50,
      maxPerFile: 240,
      maxActive: 20,
      ramProfile: 'balanced'
    },
    {
      id: 'turbo',
      icon: '🚀',
      title: 'Turbo Fibra · Enjambre',
      tag: 'Máximo caudal',
      desc: 'Exprime toda la línea para descargas grandes y compartición agresiva.',
      capDownKbs,
      capUpKbs,
      dlLimitKbs: 0,
      ulLimitKbs: Math.max(512, Math.round(capUpKbs * 0.92)),
      ussEnabled: true,
      idleCapKbs: 0,
      streamCapKbs: Math.max(256, Math.round(capUpKbs * 0.60)),
      smartSlotKbs: 512,
      maxGlobal: isLowRam ? 650 : 900,
      maxConnMin: 120,
      maxPerFile: 450,
      maxActive: 35,
      ramProfile: isLowRam ? 'balanced' : 'high'
    }
  ];
}

function renderSmartSpeedProfiles() {
  const grid = document.getElementById('smart-profile-grid');
  if (!grid) return;
  const ctx = getSmartScanBaseContext();
  const profiles = buildSmartSpeedProfiles(ctx);

  grid.innerHTML = profiles.map(p => {
    const isActive = selectedSmartProfileId === p.id;
    const dlText = p.dlLimitKbs === 0 ? 'Ilimitada' : `${(p.dlLimitKbs / 1024).toFixed(1)} MB/s`;
    const ulText = `${(p.ulLimitKbs / 1024).toFixed(1)} MB/s`;
    return `
      <button type="button" class="smart-profile-card ${isActive ? 'active' : ''}" onclick="applySmartSpeedProfile('${p.id}')">
        <div class="smart-profile-head">
          <span class="smart-profile-title">${p.icon} ${p.title}</span>
          <span class="smart-profile-tag">${p.tag}</span>
        </div>
        <div class="smart-profile-desc">${p.desc}</div>
        <div class="smart-profile-specs">
          <span>⬇ ${dlText}</span>
          <span>⬆ ${ulText}</span>
          <span>🔌 ${p.maxGlobal} conns</span>
          <span>🧠 RAM: ${p.ramProfile}</span>
        </div>
      </button>
    `;
  }).join('');
}

window.refreshSmartProfilesPreview = function() {
  renderSmartSpeedProfiles();
};

async function runSmartBandwidthScanner() {
  const btn = document.getElementById('btn-run-bw-scanner');
  const progWrap = document.getElementById('bw-scanner-progress-wrap');
  const stepEl = document.getElementById('bw-scanner-step');
  const pctEl = document.getElementById('bw-scanner-pct');
  const barEl = document.getElementById('bw-scanner-bar');
  const metricsEl = document.getElementById('bw-scanner-metrics');

  if (btn) {
    btn.disabled = true;
    btn.textContent = '⏳ Escaneando...';
  }
  if (progWrap) progWrap.style.display = 'block';

  const setProg = (pct, label) => {
    if (barEl) barEl.style.width = `${pct}%`;
    if (pctEl) pctEl.textContent = `${pct}%`;
    if (stepEl) stepEl.textContent = label;
  };

  try {
    setProg(18, 'Consultando hardware del servidor y almacenamiento...');
    const diskInfo = await client.call('system.diskSpace', {}).catch(() => lastDiskSpaceInfo);
    const limitsInfo = await client.call('limits.get', {}).catch(() => cachedLimitsInfo);

    setProg(46, 'Midiendo latencia RTT y estabilidad de enlace...');
    const rttSamples = [];
    for (let i = 0; i < 4; i++) {
      const t0 = performance.now();
      await client.call('system.status', {}).catch(() => null);
      rttSamples.push(performance.now() - t0);
    }
    const avgRpcMs = rttSamples.reduce((a, b) => a + b, 0) / Math.max(1, rttSamples.length);

    setProg(75, 'Midiendo caudal efectivo de transferencia...');
    let measuredDownMbps = 0;
    try {
      const tStart = performance.now();
      const resp = await fetch(`/assets/theme.css?scan=${Date.now()}`, { cache: 'no-store' });
      const buf = await resp.arrayBuffer();
      const dtSec = Math.max(0.004, (performance.now() - tStart) / 1000);
      const mbps = (buf.byteLength * 8) / (dtSec * 1_000_000);
      measuredDownMbps = Math.round(Math.max(25, Math.min(1000, mbps)));
    } catch (_) {
      measuredDownMbps = 300;
    }

    const hint = document.getElementById('bw-line-hint')?.value || 'auto';
    const peakSessionDownMbps = sessionPeakDown > 0 ? Math.round((sessionPeakDown * 8) / 1_000_000) : 0;
    const peakSessionUpMbps = sessionPeakUp > 0 ? Math.round((sessionPeakUp * 8) / 1_000_000) : 0;

    let finalDownMbps = hint !== 'auto' ? parseInt(hint, 10) : Math.max(measuredDownMbps, peakSessionDownMbps);
    if (!Number.isFinite(finalDownMbps) || finalDownMbps < 50) finalDownMbps = 300;
    // Normalizar a escalones habituales de fibra si está cerca
    const tiers = [100, 300, 500, 600, 1000];
    if (hint === 'auto') {
      const closest = tiers.find(t => Math.abs(t - finalDownMbps) <= 75);
      if (closest) finalDownMbps = closest;
    }
    const finalUpMbps = hint !== 'auto'
      ? finalDownMbps
      : Math.max(Math.round(finalDownMbps * 0.8), peakSessionUpMbps, 100);

    const ussPing = limitsInfo?.uss?.lowestPingMs || limitsInfo?.uss?.latencyMs || null;
    const rttMs = Math.round(ussPing || avgRpcMs || 10);
    const memBytes = diskInfo?.host?.memTotalBytes || 0;
    const hostRamGb = memBytes > 0 ? (memBytes / (1024 * 1024 * 1024)) : 4.0;
    const hostName = diskInfo?.host?.hostname || 'PulseCore';
    const isNas = Boolean(diskInfo?.nas?.nfs?.isNetworkMount);

    lastBandwidthScanResult = {
      downMbps: finalDownMbps,
      upMbps: finalUpMbps,
      rttMs,
      hostRamGb,
      hostOs: diskInfo?.host?.os || 'linux',
      hostName,
      isNas,
      isLowRamDevice: hostRamGb <= 4.5,
      measured: true
    };

    setProg(100, 'Escaneo completado · Perfiles recalculados a tu medida');

    if (metricsEl) {
      metricsEl.style.display = 'grid';
      metricsEl.innerHTML = `
        <div class="bw-scan-stat"><span>Bajada estimada</span><strong>${finalDownMbps} Mbps (${Math.round(finalDownMbps / 8)} MB/s)</strong></div>
        <div class="bw-scan-stat"><span>Subida estimada</span><strong>${finalUpMbps} Mbps (${Math.round(finalUpMbps / 8)} MB/s)</strong></div>
        <div class="bw-scan-stat"><span>Latencia RTT</span><strong>${rttMs} ms</strong></div>
        <div class="bw-scan-stat"><span>Equipo detectado</span><strong>${escapeHtml(hostName)} · ${hostRamGb.toFixed(1)} GB${isNas ? ' · NAS' : ''}</strong></div>
      `;
    }

    renderSmartSpeedProfiles();
    showToast('Escaneo completado: selecciona un perfil a medida', 'success', '📡');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '📡 Escanear Red y Equipo';
    }
  }
}

async function applySmartSpeedProfile(profileId) {
  const ctx = getSmartScanBaseContext();
  const profile = buildSmartSpeedProfiles(ctx).find(p => p.id === profileId);
  if (!profile) return;

  const setVal = (id, v) => { const el = document.getElementById(id); if (el) el.value = String(v); };
  const setChk = (id, v) => { const el = document.getElementById(id); if (el) el.checked = Boolean(v); };

  setVal('pref-cap-down', profile.capDownKbs);
  setVal('pref-cap-up', profile.capUpKbs);
  setChk('pref-uss-enabled', profile.ussEnabled);
  setVal('pref-idle-upload-cap', profile.idleCapKbs);
  setVal('pref-streaming-upload-cap', profile.streamCapKbs);
  setVal('pref-smart-slot-target', profile.smartSlotKbs);
  setVal('pref-max-global', profile.maxGlobal);
  setVal('pref-max-conn-min', profile.maxConnMin);
  setVal('pref-max-per-file', profile.maxPerFile);
  setVal('pref-max-active-downloads', profile.maxActive);
  setVal('pref-ram-profile', profile.ramProfile);
  updateRamProfileBadge();

  selectedSmartProfileId = profile.id;
  try { localStorage.setItem('pulsecore_smart_speed_profile', profile.id); } catch (_) { /* Ignorar */ }
  renderSmartSpeedProfiles();

  const badge = document.getElementById('smart-profile-active-badge');
  if (badge) {
    badge.textContent = `${profile.icon} ${profile.title}`;
    badge.style.display = 'inline-flex';
  }

  try {
    await client.call('limits.set', {
      downloadLimit: profile.dlLimitKbs,
      uploadLimit: profile.ulLimitKbs,
      connectionsPerMinute: profile.maxConnMin,
      smartSlotTargetKbs: profile.smartSlotKbs,
      uss: {
        enabled: profile.ussEnabled,
        maxUploadBytesPerSec: profile.ulLimitKbs * 1024,
        idleUploadCapBytesPerSec: profile.idleCapKbs * 1024,
        streamingUploadCapBytesPerSec: profile.streamCapKbs * 1024
      }
    });
    await savePreferences();
    showToast(`Perfil "${profile.title}" aplicado y guardado`, 'success', profile.icon);
  } catch (e) {
    showToast(`Error aplicando perfil: ${e.message || e}`, 'error', '❌');
  }
}

window.runSmartBandwidthScanner = runSmartBandwidthScanner;
window.applySmartSpeedProfile = applySmartSpeedProfile;
renderSmartSpeedProfiles();

