// Pulse Core WebUI — Buscador global Spotlight (Ctrl+K) y catálogo interactivo de herramientas MCP.
// Extraído de web/index.html.

// === BUSCADOR GLOBAL UNIFICADO SPOTLIGHT / COMMAND PALETTE (CTRL+K) ===
let spotlightSelectedIndex = 0;
let spotlightFilteredItems = [];

function openSpotlight() {
  const el = document.getElementById('spotlight-overlay');
  const input = document.getElementById('spotlight-input');
  if (!el || !input) return;
  el.classList.add('active');
  input.value = '';
  input.focus();
  renderSpotlightResults('');
}

function closeSpotlight(e) {
  if (e && e.target && e.target.closest('.spotlight-modal')) return;
  const el = document.getElementById('spotlight-overlay');
  if (el) el.classList.remove('active');
}

function renderSpotlightResults(query) {
  const q = (query || '').toLowerCase().trim();
  const resultsEl = document.getElementById('spotlight-results');
  if (!resultsEl) return;

  spotlightFilteredItems = [];

  if (!q) {
    spotlightFilteredItems.push(
      { type: 'nav', tab: 'tab-media', subview: 'library', title: 'Ver Filmoteca (Cartelera)', sub: 'Explorar catálogo multimedia de películas y series', icon: '🎬', cat: 'Vistas' },
      { type: 'nav', tab: 'tab-media', subview: 'queue', title: 'Ver Cola de descargas', sub: 'Ir a la tabla técnica de transferencias eD2k y BitTorrent', icon: '📥', cat: 'Vistas' },
      { type: 'action', action: 'openMediaSearchModal', title: 'Añadir Película o Serie', sub: 'Buscar metadatos en TMDB y capturar fuentes', icon: '➕', cat: 'Filmoteca' },
      { type: 'action', action: 'openAddModal', title: 'Añadir enlace eD2k o Magnet', sub: 'Pegar enlaces ed2k:// o magnet:? directos', icon: '⚡', cat: 'Descargas' },
      { type: 'nav', tab: 'tab-servers', title: 'Ver Servidores eD2k', sub: 'Lista de servidores y estado de conexión', icon: '🌐', cat: 'Vistas' },
      { type: 'nav', tab: 'tab-logs', title: 'Ver Registro en Vivo', sub: 'Historial de eventos y registro del daemon', icon: '📜', cat: 'Vistas' }
    );
  } else {
    // 1. Buscar en descargas activas (cachedDownloads)
    if (typeof cachedDownloads !== 'undefined' && Array.isArray(cachedDownloads)) {
      cachedDownloads.forEach(item => {
        const dl = item.snapshot || item;
        const name = fixMojibake(dl.fileName || dl.display_name || dl.file_name || '');
        if (name.toLowerCase().includes(q)) {
          const size = formatBytes(dl.totalBytes || dl.total_bytes || dl.total_size || 0);
          const status = dl.status || 'Descargando';
          spotlightFilteredItems.push({
            type: 'download',
            id: dl.id,
            title: name,
            sub: `${size} • Estado: ${status}`,
            icon: '📥',
            cat: 'Descarga'
          });
        }
      });
    }

    // 2. Buscar en biblioteca Cine & Series (mediaLibraryItems)
    if (typeof mediaLibraryItems !== 'undefined' && Array.isArray(mediaLibraryItems)) {
      mediaLibraryItems.forEach(item => {
        const title = (item.title || '').toLowerCase();
        const orig = (item.originalTitle || '').toLowerCase();
        if (title.includes(q) || orig.includes(q)) {
          const isMovie = item.mediaType === 'movie';
          const yearStr = item.year ? `(${item.year})` : '';
          const statusStr = item.status === 'completed' ? 'En biblioteca' : (item.status === 'downloading' ? 'Descargando' : 'Buscada');
          spotlightFilteredItems.push({
            type: 'media',
            id: item.id,
            title: `${item.title} ${yearStr}`,
            sub: `${isMovie ? 'Película' : 'Serie TV'} • ${statusStr} • Calidad: ${item.qualityProfile || '1080p'}`,
            icon: isMovie ? '🎬' : '📺',
            cat: isMovie ? 'Película' : 'Serie'
          });
        }
      });
    }

    // 3. Opción para buscar externamente en TMDB
    spotlightFilteredItems.push({
      type: 'search_external',
      query: query,
      title: `Buscar "${query}" para añadir a la biblioteca...`,
      sub: 'Consultar TMDB y capturar lanzamientos en Kademlia y Torznab',
      icon: '🔍',
      cat: 'TMDB'
    });
  }

  if (spotlightFilteredItems.length === 0) {
    resultsEl.innerHTML = `<div style="text-align: center; padding: 24px; color: #64748b; font-size: 0.85rem;">No se encontraron resultados para "${escapeHtml(query)}"</div>`;
    return;
  }

  spotlightSelectedIndex = 0;
  renderSpotlightListHtml();
}

function renderSpotlightListHtml() {
  const resultsEl = document.getElementById('spotlight-results');
  if (!resultsEl) return;

  resultsEl.innerHTML = spotlightFilteredItems.slice(0, 15).map((item, idx) => {
    const isSel = idx === spotlightSelectedIndex;
    return `
      <div class="spotlight-item ${isSel ? 'selected' : ''}" onclick="executeSpotlightItem(${idx})">
        <div class="spotlight-item-icon">${item.icon}</div>
        <div class="spotlight-item-text">
          <div class="spotlight-item-title">${escapeHtml(item.title)}</div>
          <div class="spotlight-item-sub">${escapeHtml(item.sub)}</div>
        </div>
        <div class="spotlight-item-category">${item.cat}</div>
      </div>
    `;
  }).join('');
}

function executeSpotlightItem(idx) {
  const item = spotlightFilteredItems[idx];
  if (!item) return;

  closeSpotlight();

  if (item.type === 'nav') {
    const tabBtn = document.querySelector(`.nav-btn[data-tab="${item.tab}"]`);
    if (tabBtn) tabBtn.click();
    if (item.subview && typeof switchMediaSubView === 'function') {
      switchMediaSubView(item.subview);
    }
  } else if (item.type === 'action') {
    if (item.action === 'openMediaSearchModal') openMediaSearchModal();
    if (item.action === 'openAddModal') openAddModal();
  } else if (item.type === 'download') {
    openDownloadInQueue(item.id);
  } else if (item.type === 'media') {
    const tabBtn = document.querySelector(`.nav-btn[data-tab="tab-media"]`);
    if (tabBtn) tabBtn.click();
    if (typeof switchMediaSubView === 'function') switchMediaSubView('library');
    setTimeout(() => {
      if (typeof openMediaDetail === 'function') openMediaDetail(item.id);
    }, 120);
  } else if (item.type === 'search_external') {
    openMediaSearchModal();
    const sInput = document.getElementById('media-search-input');
    if (sInput) {
      sInput.value = item.query;
      if (typeof execMediaSearch === 'function') execMediaSearch();
    }
  }
}

// Atajo global Ctrl+K / Cmd+K y navegación con flechas
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && (e.key === 'k' || e.key === 'K')) {
    e.preventDefault();
    const el = document.getElementById('spotlight-overlay');
    if (el && el.classList.contains('active')) {
      closeSpotlight();
    } else {
      openSpotlight();
    }
    return;
  }

  const bwModal = document.getElementById('bandwidth-details-modal');
  if (bwModal && bwModal.classList.contains('active') && e.key === 'Escape') {
    e.preventDefault();
    closeBandwidthModal();
    return;
  }

  const overlay = document.getElementById('spotlight-overlay');
  if (overlay && overlay.classList.contains('active')) {
    if (e.key === 'Escape') {
      e.preventDefault();
      closeSpotlight();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      const max = Math.min(15, spotlightFilteredItems.length);
      if (max > 0) {
        spotlightSelectedIndex = (spotlightSelectedIndex + 1) % max;
        renderSpotlightListHtml();
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      const max = Math.min(15, spotlightFilteredItems.length);
      if (max > 0) {
        spotlightSelectedIndex = (spotlightSelectedIndex - 1 + max) % max;
        renderSpotlightListHtml();
      }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      executeSpotlightItem(spotlightSelectedIndex);
    }
  }
});

const spotlightInput = document.getElementById('spotlight-input');
if (spotlightInput) {
  spotlightInput.addEventListener('input', (e) => {
    renderSpotlightResults(e.target.value);
  });
}

client.connect();

// ==========================================
// PROTOCOLO MCP & CATÁLOGO DE HERRAMIENTAS IA
// ==========================================
const MCP_TOOLS = [{"name": "emule_status", "description": "Get overall eMuleRust daemon status, uptime, transfer speeds, active downloads count, server connection, and Kad status.", "category": "system", "risk": "read"}, {"name": "emule_get_settings", "description": "Get eMuleRust daemon configuration, default paths, limits, and retention policy.", "category": "system", "risk": "read"}, {"name": "emule_update_settings", "description": "Update daemon settings: default download folder, history retention, peer connection quotas.", "category": "system", "risk": "mutate"}, {"name": "emule_reset_download_dir", "description": "Reset the default download directory to the default data folder.", "category": "system", "risk": "mutate"}, {"name": "emule_set_limits", "description": "Configure global download and upload rate limits in KB/s (0 = unlimited).", "category": "system", "risk": "mutate"}, {"name": "emule_shutdown", "description": "Gracefully shut down the eMuleRust background daemon.", "category": "system", "risk": "destructive"}, {"name": "emule_get_network_policy", "description": "Get active network security policy, profile ('standard', 'vpn_only', 'tor_proxy', 'paranoid', 'local_lab'), budgets, and active connection counts.", "category": "system", "risk": "read"}, {"name": "emule_set_network_profile", "description": "Switch the network isolation profile ('standard', 'vpn_only', 'tor_proxy', 'paranoid', 'local_lab').", "category": "system", "risk": "mutate"}, {"name": "emule_network_disconnect_all", "description": "Emergency network killswitch: immediately disconnects all server, peer, and listener sockets.", "category": "system", "risk": "destructive"}, {"name": "emule_network_enable", "description": "Re-enable network communications after a killswitch disconnect.", "category": "system", "risk": "mutate"}, {"name": "emule_get_audit_log", "description": "Inspect network security audit log events (blocked connections, profile violations, rate limiting).", "category": "system", "risk": "read"}, {"name": "emule_set_network_budgets", "description": "Set network bandwidth and connection budgets dynamically.", "category": "system", "risk": "mutate"}, {"name": "emule_get_filter_status", "description": "Get IP filter security status, loaded rules count, and blocked connection statistics.", "category": "system", "risk": "read"}, {"name": "emule_import_ip_filter", "description": "Import IP blocklist file (ipfilter.dat, guarding.p2p, or peerguardian format) to block malicious peers.", "category": "system", "risk": "mutate"}, {"name": "emule_set_filter_enabled", "description": "Enable or disable IP blocklist filtering dynamically.", "category": "system", "risk": "mutate"}, {"name": "emule_clear_ip_filter", "description": "Clear all IP filter rules from memory.", "category": "system", "risk": "destructive"}, {"name": "emule_disk_space", "description": "Query local and NAS disk space usage (total, used, free bytes).", "category": "system", "risk": "read"}, {"name": "emule_nas_info", "description": "Query NAS mount status and media directory file counts.", "category": "system", "risk": "read"}, {"name": "emule_system_logs", "description": "Read recent daemon log lines with optional substring filter.", "category": "system", "risk": "read"}, {"name": "emule_media_list", "description": "List items in the native Servarr media library (movies and series).", "category": "system", "risk": "read"}, {"name": "emule_search_all", "description": "Unified multi-protocol search across eD2K servers, Kad DHT, and Torznab indexers.", "category": "system", "risk": "read"}, {"name": "emule_list_downloads", "description": "List all files currently in the download queue with their ID, hash, filename, progress, state, size, downloaded bytes, and speed.", "category": "downloads", "risk": "read"}, {"name": "emule_get_download_details", "description": "Get in-depth details for a download: completed parts map, part.met status, sources summary, destination, and transfer errors.", "category": "downloads", "risk": "read"}, {"name": "emule_add_link", "description": "Add an eD2K link (e.g. ed2k://|file|filename.iso|1234567|HASH|/) to the download queue.", "category": "downloads", "risk": "mutate"}, {"name": "emule_pause_download", "description": "Pause an active or pending download using its UUID or file hash.", "category": "downloads", "risk": "mutate"}, {"name": "emule_resume_download", "description": "Resume or start a paused download using its UUID or file hash.", "category": "downloads", "risk": "mutate"}, {"name": "emule_remove_download", "description": "Remove or cancel a download from queue, optionally deleting associated files on disk.", "category": "downloads", "risk": "destructive"}, {"name": "emule_rename_download", "description": "Rename a download's file name (in-progress or completed). For a completed download the file on disk and its share entry are renamed too; the .part temp file keeps its internal name.", "category": "downloads", "risk": "mutate"}, {"name": "emule_set_destination", "description": "Change the target directory where the completed file will be moved.", "category": "downloads", "risk": "mutate"}, {"name": "emule_download_set_category", "description": "Assign or clear the category for a specific download.", "category": "downloads", "risk": "mutate"}, {"name": "emule_retry_download", "description": "Force reconnection and re-query sources for a stalled, error, or waiting download.", "category": "downloads", "risk": "mutate"}, {"name": "emule_inspect_part_met", "description": "Inspect an existing eMule .part.met file on disk to examine its name, hash, size, parts, and corruption state.", "category": "downloads", "risk": "read"}, {"name": "emule_import_part_met", "description": "Import a legacy or foreign .part.met file into the active eMuleRust download queue.", "category": "downloads", "risk": "mutate"}, {"name": "emule_list_categories", "description": "List all download categories with their ID, name, color, destination, and priority.", "category": "downloads", "risk": "read"}, {"name": "emule_create_category", "description": "Create a new download category with custom color and default target folder.", "category": "downloads", "risk": "mutate"}, {"name": "emule_update_category", "description": "Update an existing category's properties.", "category": "downloads", "risk": "mutate"}, {"name": "emule_delete_category", "description": "Delete a category. Downloads previously assigned to it are automatically unassigned.", "category": "downloads", "risk": "destructive"}, {"name": "emule_reorder_categories", "description": "Reorder categories by providing an ordered list of category IDs.", "category": "downloads", "risk": "mutate"}, {"name": "emule_list_servers", "description": "List all known eD2K servers, connection status, IP filter state, ping, and preference.", "category": "servers", "risk": "read"}, {"name": "emule_connect_server", "description": "Connect to an eD2K server by address 'ip:port' or automatically pick the best allowed server.", "category": "servers", "risk": "mutate"}, {"name": "emule_disconnect_server", "description": "Disconnect from the currently connected eD2K server.", "category": "servers", "risk": "mutate"}, {"name": "emule_update_servers_from_url", "description": "Download and import server list from a remote server.met URL.", "category": "servers", "risk": "mutate"}, {"name": "emule_inspect_server_met", "description": "Inspect an offline server.met file on disk to examine its server list and IP filter status.", "category": "servers", "risk": "read"}, {"name": "emule_import_server_met", "description": "Import servers from a local server.met file into the daemon database.", "category": "servers", "risk": "mutate"}, {"name": "emule_set_server_enabled", "description": "Enable or disable a server in the local server list.", "category": "servers", "risk": "mutate"}, {"name": "emule_delete_server", "description": "Delete an eD2K server from the local server list.", "category": "servers", "risk": "destructive"}, {"name": "emule_clear_servers", "description": "Clear all servers from the local server list.", "category": "servers", "risk": "destructive"}, {"name": "emule_query_server_sources", "description": "Send a GetSources request to the connected server for a file hash.", "category": "servers", "risk": "read"}, {"name": "emule_search", "description": "Search for files on connected ED2K servers and Kad DHT network. Automatically waits for and returns search results including synthesized ed2k links ready for download.", "category": "servers", "risk": "read"}, {"name": "emule_list_sources", "description": "List all known peer sources for a specific file hash.", "category": "shares", "risk": "read"}, {"name": "emule_add_manual_source", "description": "Add a manual peer source by address 'ip:port' for a file hash.", "category": "shares", "risk": "mutate"}, {"name": "emule_connect_source", "description": "Force immediate connection to a specific peer source for a file.", "category": "shares", "risk": "mutate"}, {"name": "emule_disconnect_source", "description": "Disconnect from a specific peer source or the active peer.", "category": "shares", "risk": "mutate"}, {"name": "emule_clear_sources", "description": "Clear all discovered peer sources for a file hash.", "category": "shares", "risk": "destructive"}, {"name": "emule_swarm_status", "description": "Get full swarm status: active peers count, in-flight blocks, chunk distribution.", "category": "shares", "risk": "read"}, {"name": "emule_list_shares", "description": "List all files currently shared with the eD2K/Kad network.", "category": "shares", "risk": "read"}, {"name": "emule_add_share", "description": "Share a file with other eD2K/Kad peers.", "category": "shares", "risk": "mutate"}, {"name": "emule_remove_share", "description": "Unshare a file from the eD2K/Kad network, optionally deleting the file on disk.", "category": "shares", "risk": "destructive"}, {"name": "emule_rename_share", "description": "Rename a fully shared file: the file on disk and its share entry are renamed (and the linked completed download when one exists).", "category": "shares", "risk": "mutate"}, {"name": "emule_set_share_enabled", "description": "Enable or disable sharing for a file without removing it from the share list.", "category": "shares", "risk": "mutate"}, {"name": "emule_list_uploads", "description": "List active and queued upload transfers to remote peers.", "category": "shares", "risk": "read"}, {"name": "emule_cancel_upload", "description": "Cancel/release an active upload transfer slot for a remote client address.", "category": "shares", "risk": "destructive"}, {"name": "emule_start_listener", "description": "Start the TCP inbound peer connection listener.", "category": "shares", "risk": "mutate"}, {"name": "emule_stop_listener", "description": "Stop the TCP inbound peer connection listener.", "category": "shares", "risk": "mutate"}, {"name": "emule_listener_status", "description": "Get TCP inbound listener status, bound address, and active connection count.", "category": "shares", "risk": "read"}, {"name": "emule_upnp_status", "description": "Get UPnP automatic router port forwarding status, active mappings, and external IP.", "category": "shares", "risk": "read"}, {"name": "emule_upnp_map_ports", "description": "Request UPnP port forwarding on your router for TCP and UDP eMule ports.", "category": "shares", "risk": "mutate"}, {"name": "emule_upnp_unmap_ports", "description": "Remove UPnP port forwarding rules from the router.", "category": "shares", "risk": "mutate"}, {"name": "emule_upnp_set_enabled", "description": "Enable or disable automatic UPnP port forwarding.", "category": "shares", "risk": "mutate"}, {"name": "emule_get_identity", "description": "Get local user hash, secure identification status, and RSA public key fingerprint.", "category": "shares", "risk": "read"}, {"name": "emule_rotate_identity", "description": "Generate a new cryptographic user identity (user hash and RSA keypair).", "category": "shares", "risk": "destructive"}, {"name": "emule_list_credits", "description": "List earned peer credit records, showing remote clients, uploaded/downloaded bytes, and credit multipliers.", "category": "shares", "risk": "read"}, {"name": "emule_credits_stats", "description": "Get overall credits statistics: total credited peers, total upload/download volume.", "category": "shares", "risk": "read"}, {"name": "emule_list_history", "description": "Query download history of completed or cancelled downloads.", "category": "shares", "risk": "read"}, {"name": "emule_history_stats", "description": "Get aggregate historical stats: total files completed, total bytes downloaded and uploaded.", "category": "shares", "risk": "read"}, {"name": "emule_clear_history", "description": "Clear all past download history entries.", "category": "shares", "risk": "destructive"}, {"name": "emule_export_history", "description": "Export download history to JSON or CSV format string.", "category": "shares", "risk": "read"}, {"name": "emule_kad_status", "description": "Get Kad DHT network status: connection state, open/firewalled status, contacts count.", "category": "kad", "risk": "read"}, {"name": "emule_kad_bootstrap", "description": "Bootstrap the Kad DHT network from a known node IP:port or a nodes.dat URL.", "category": "kad", "risk": "mutate"}, {"name": "emule_kad_start_udp", "description": "Start the Kad DHT UDP socket listener.", "category": "kad", "risk": "mutate"}, {"name": "emule_kad_stop_udp", "description": "Stop the Kad DHT UDP socket listener.", "category": "kad", "risk": "mutate"}, {"name": "emule_kad_ping", "description": "Ping a Kad DHT contact node to verify connectivity and measure latency.", "category": "kad", "risk": "read"}, {"name": "emule_kad_find_node", "description": "Search Kad DHT routing tree for contacts closest to a 128-bit hex node ID.", "category": "kad", "risk": "read"}, {"name": "emule_kad_list_contacts", "description": "List contacts stored in the Kad DHT routing table (IP, port, Kad ID, version).", "category": "kad", "risk": "read"}, {"name": "emule_kad_clear_contacts", "description": "Clear all contacts from the Kad DHT routing table.", "category": "kad", "risk": "destructive"}, {"name": "emule_kad_search_sources", "description": "Query Kad DHT specifically for download sources of a file hash.", "category": "kad", "risk": "read"}, {"name": "emule_kad_publish_source", "description": "Publish ourselves as a source for a shared file to the Kad DHT.", "category": "kad", "risk": "mutate"}, {"name": "emule_kad_publish_keywords", "description": "Publish search keyword index entries for a shared file to the Kad DHT.", "category": "kad", "risk": "mutate"}, {"name": "emule_kad_search_notes", "description": "Query Kad DHT for file notes (user comments and ratings) indexed for a file hash.", "category": "kad", "risk": "read"}, {"name": "emule_kad_publish_note", "description": "Publish a comment/rating for a file to the Kad DHT notes index.", "category": "kad", "risk": "mutate"}, {"name": "emule_kad_inspect_nodes_dat", "description": "Inspect an offline nodes.dat file on disk to examine its Kad contacts before importing.", "category": "kad", "risk": "read"}, {"name": "emule_kad_import_nodes_dat", "description": "Import Kad contact nodes from a local nodes.dat file on disk.", "category": "kad", "risk": "mutate"}];
let currentMcpCategory = 'all';
let currentMcpClient = 'claude';
window.mcpAuthToken = '';

function updateMcpProfileExplainer() {
  const profileEl = document.getElementById('pref-mcp-profile');
  const explainerEl = document.getElementById('mcp-profile-explainer');
  const badgeEl = document.getElementById('mcp-profile-badge');
  if (!profileEl || !explainerEl) return;
  const val = profileEl.value || 'safe';

  if (val === 'safe') {
    if (badgeEl) badgeEl.textContent = 'Perfil: Safe (Recomendado)';
    explainerEl.innerHTML = `
      <div style="font-weight:600; color:#38bdf8; margin-bottom:4px;">🛡️ Modo Safe (Equilibrio Óptimo)</div>
      <div>• <b>79 herramientas activas</b> (37 lectura + 42 mutaciones controladas).</div>
      <div>• Permite a la IA buscar archivos, encolar enlaces ed2k, gestionar prioridades y pausar/reanudar descargas.</div>
      <div style="color:#f87171; margin-top:2px;">• <b>13 herramientas destructivas bloqueadas</b>: el daemon rechazará intentos de apagar el nodo, borrar archivos en disco o purgar contactos.</div>
    `;
  } else if (val === 'read') {
    if (badgeEl) badgeEl.textContent = 'Perfil: Read (Solo Lectura)';
    explainerEl.innerHTML = `
      <div style="font-weight:600; color:#4ade80; margin-bottom:4px;">👁️ Modo Read (Máxima Restricción)</div>
      <div>• <b>37 herramientas activas</b> únicamente de consulta.</div>
      <div>• La IA puede inspeccionar transferencias, velocidad, servidores e historial sin realizar ninguna mutación en el daemon.</div>
      <div style="color:#fbbf24; margin-top:2px;">• 55 herramientas de mutación y destructivas bloqueadas.</div>
    `;
  } else {
    if (badgeEl) badgeEl.textContent = 'Perfil: Full (Administrador)';
    explainerEl.innerHTML = `
      <div style="font-weight:600; color:#f87171; margin-bottom:4px;">⚠️ Modo Full (Acceso Total de Administrador)</div>
      <div>• <b>Las 92 herramientas están autorizadas sin restricción.</b></div>
      <div>• Permite a la IA realizar cualquier acción, incluyendo borrado definitivo de archivos en disco, purga de datos y apagado del daemon.</div>
    `;
  }
}

function switchMcpClientSnippet(clientType) {
  currentMcpClient = clientType;
  document.querySelectorAll('#mcp-client-tabs .filter-btn').forEach(b => b.classList.remove('active'));
  const activeBtn = document.getElementById('mcp-tab-' + clientType);
  if (activeBtn) activeBtn.classList.add('active');
  updateMcpSnippets();
}

function updateMcpSnippets() {
  const codeEl = document.getElementById('mcp-snippet-code');
  const hintEl = document.getElementById('mcp-target-file-hint');
  const profileEl = document.getElementById('pref-mcp-profile');
  if (!codeEl) return;

  const profile = profileEl ? profileEl.value : 'safe';
  const host = window.location.hostname || '192.168.1.166';
  const token = window.mcpAuthToken || 'TU_TOKEN_AUTH';
  const rpcUrl = `ws://${host}:4660/rpc`;

  if (currentMcpClient === 'claude') {
    if (hintEl) hintEl.innerHTML = 'Archivo: <code>claude_desktop_config.json</code> en <code>~/.config/Claude/</code> o <code>%APPDATA%\\Claude\\</code>';
    const cfg = {
      mcpServers: {
        emulerust: {
          command: "/home/caraveliko/.local/bin/mule-mcp",
          args: [
            "--url", rpcUrl,
            "--token", token,
            "--tool-profile", profile
          ]
        }
      }
    };
    codeEl.textContent = JSON.stringify(cfg, null, 2);
  } else if (currentMcpClient === 'cursor') {
    if (hintEl) hintEl.innerHTML = 'Archivo: <code>.cursor/mcp.json</code> en la raíz de tu proyecto o workspace';
    const cfg = {
      mcpServers: {
        emulerust: {
          command: "/home/caraveliko/.local/bin/mule-mcp",
          args: [
            "--url", rpcUrl,
            "--token", token,
            "--tool-profile", profile
          ]
        }
      }
    };
    codeEl.textContent = JSON.stringify(cfg, null, 2);
  } else if (currentMcpClient === 'antigravity') {
    if (hintEl) hintEl.innerHTML = 'Configuración para <b>Google Antigravity / Cline / Roo Code / Windsurf</b>';
    const cfg = {
      mcpServers: {
        emulerust: {
          command: "/home/caraveliko/.local/bin/mule-mcp",
          args: [
            "--url", rpcUrl,
            "--token", token,
            "--tool-profile", profile
          ],
          disabled: false,
          autoApprove: []
        }
      }
    };
    codeEl.textContent = JSON.stringify(cfg, null, 2);
  } else if (currentMcpClient === 'cli') {
    if (hintEl) hintEl.innerHTML = 'Ejecución en consola (servidor MCP estándar vía stdio)';
    codeEl.textContent = `mule-mcp --url "${rpcUrl}" --token "${token}" --tool-profile ${profile}`;
  }
}

function copyMcpSnippet() {
  const codeEl = document.getElementById('mcp-snippet-code');
  const btn = document.getElementById('btn-copy-snippet');
  const btnTop = document.getElementById('btn-copy-mcp-top');
  if (!codeEl) return;
  navigator.clipboard.writeText(codeEl.textContent).then(() => {
    if (btn) {
      const prev = btn.textContent;
      btn.textContent = '✓ Copiado';
      setTimeout(() => { btn.textContent = prev; }, 2000);
    }
    if (btnTop) {
      const prevTop = btnTop.textContent;
      btnTop.textContent = '✓ ¡Copiado al portapapeles!';
      setTimeout(() => { btnTop.textContent = prevTop; }, 2000);
    }
  }).catch(err => {
    console.error('Error al copiar snippet MCP:', err);
  });
}

function onMcpProfileChanged() {
  updateMcpProfileExplainer();
  updateMcpSnippets();
  renderMcpTools();
}

function onMcpSearchInput() {
  renderMcpTools();
}

function filterMcpTools(cat) {
  currentMcpCategory = cat;
  document.querySelectorAll('#mcp-category-filters .filter-btn').forEach(b => b.classList.remove('active'));
  const btn = document.getElementById('btn-mcp-cat-' + cat);
  if (btn) btn.classList.add('active');
  renderMcpTools();
}

function renderMcpTools() {
  const grid = document.getElementById('mcp-tools-grid');
  if (!grid) return;

  const profileEl = document.getElementById('pref-mcp-profile');
  const profile = profileEl ? profileEl.value : 'safe';
  const query = (document.getElementById('mcp-tool-search')?.value || '').toLowerCase().trim();

  let filtered = MCP_TOOLS;
  if (currentMcpCategory !== 'all') {
    filtered = filtered.filter(t => t.category === currentMcpCategory);
  }
  if (query) {
    filtered = filtered.filter(t => t.name.toLowerCase().includes(query) || t.description.toLowerCase().includes(query));
  }

  // Update KPI
  const countTotal = document.getElementById('mcp-count-total');
  if (countTotal) countTotal.textContent = `Mostrando: ${filtered.length} / 92`;

  const catNames = {
    system: 'Sistema & Red',
    downloads: 'Descargas & Partes',
    servers: 'Servidores eD2K',
    shares: 'Compartidos & Swarm',
    kad: 'Kademlia DHT'
  };

  const riskLabels = {
    read: { label: 'Lectura', bg: 'rgba(34, 197, 94, 0.15)', col: '#4ade80' },
    mutate: { label: 'Mutación', bg: 'rgba(245, 158, 11, 0.15)', col: '#fbbf24' },
    destructive: { label: 'Destructiva', bg: 'rgba(239, 68, 68, 0.15)', col: '#f87171' }
  };

  if (filtered.length === 0) {
    grid.innerHTML = '<div style="grid-column: 1 / -1; padding: 24px; text-align: center; color: var(--text-muted); font-size: 0.85rem;">No se encontraron herramientas con los filtros actuales.</div>';
    return;
  }

  grid.innerHTML = filtered.map(t => {
    let allowed = true;
    if (profile === 'read') {
      allowed = t.risk === 'read';
    } else if (profile === 'safe') {
      allowed = t.risk !== 'destructive';
    }

    const rInfo = riskLabels[t.risk] || riskLabels.read;
    const catLabel = catNames[t.category] || t.category;
    const statusBadge = allowed
      ? '<span class="badge badge-completed" style="font-size:0.65rem; padding: 2px 6px;">✓ Permitida</span>'
      : '<span class="badge" style="background: rgba(239, 68, 68, 0.2); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.3); font-size:0.65rem; padding: 2px 6px;">✕ Bloqueada</span>';

    return `
      <div class="mcp-tool-card" style="background: var(--card-sub); border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; display: flex; flex-direction: column; gap: 6px; ${allowed ? '' : 'opacity: 0.62; border-color: rgba(239,68,68,0.25);'}">
        <div style="display: flex; justify-content: space-between; align-items: center; gap: 6px;">
          <code style="font-size: 0.77rem; color: #a5b4fc; font-weight: 600; word-break: break-all;">${t.name}</code>
          <div style="display: flex; gap: 4px; align-items: center; flex-shrink: 0;">
            <span class="badge" style="background: ${rInfo.bg}; color: ${rInfo.col}; font-size: 0.65rem; padding: 2px 5px;">${rInfo.label}</span>
            ${statusBadge}
          </div>
        </div>
        <div style="font-size: 0.73rem; color: var(--text-muted); line-height: 1.35; flex-grow: 1;">
          ${escapeHtml(t.description)}
        </div>
        <div style="font-size: 0.67rem; color: #64748b; margin-top: 4px; display: flex; justify-content: space-between; border-top: 1px solid rgba(255,255,255,0.04); padding-top: 4px;">
          <span>${catLabel}</span>
          <span style="font-family: monospace;">mule-mcp</span>
        </div>
      </div>
    `;
  }).join('');
}

let folderBrowserTargetInputId = null;
let folderBrowserParentPath = null;

function isLocalBrowserHost() {
  const h = (window.location && window.location.hostname || '').toLowerCase();
  return h === '127.0.0.1' || h === 'localhost' || h === '::1';
}

async function openFolderBrowser(inputId, title) {
  const input = document.getElementById(inputId);
  const initialPath = input ? input.value.trim() : '';
  folderBrowserTargetInputId = inputId;

  if (isLocalBrowserHost()) {
    try {
      const picked = await client.call('settings.pickFolder', {
        initialPath: initialPath || undefined
      });
      if (picked && picked.supported) {
        if (!picked.cancelled && picked.path && input) {
          input.value = picked.path;
        }
        return;
      }
    } catch (e) {
      console.warn('Selector nativo no disponible, usando explorador web:', e);
    }
  }

  const titleEl = document.getElementById('folder-browser-title');
  if (titleEl) {
    titleEl.textContent = title ? `Seleccionar: ${title}` : 'Seleccionar carpeta';
  }
  const modal = document.getElementById('folder-browser-modal');
  if (modal) modal.classList.add('active');
  await loadFolderBrowserPath(initialPath);
}

async function loadFolderBrowserPath(path) {
  const listEl = document.getElementById('folder-browser-list');
  const pathInput = document.getElementById('folder-browser-path');
  const rootsEl = document.getElementById('folder-browser-roots');
  const upBtn = document.getElementById('folder-browser-up');
  if (listEl) {
    listEl.innerHTML = '<div style="padding: 20px; text-align: center; color: var(--text-muted);">Cargando carpetas...</div>';
  }
  try {
    const res = await client.call('settings.listDirs', {
      path: (path || '').trim() || undefined
    });
    if (pathInput) pathInput.value = res.currentPath || '';
    folderBrowserParentPath = res.parentPath || null;
    if (upBtn) upBtn.disabled = !folderBrowserParentPath;

    if (rootsEl) {
      rootsEl.innerHTML = '';
      (res.roots || []).forEach(root => {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'btn btn-sm';
        btn.textContent = `💾 ${root}`;
        btn.style.cssText = 'padding: 3px 8px; font-size: 0.78rem;';
        btn.addEventListener('click', () => loadFolderBrowserPath(root));
        rootsEl.appendChild(btn);
      });
    }

    if (listEl) {
      listEl.innerHTML = '';
      const dirs = res.directories || [];
      if (dirs.length === 0) {
        const empty = document.createElement('div');
        empty.style.cssText = 'padding: 20px; text-align: center; color: var(--text-muted); font-size: 0.85rem;';
        empty.textContent = 'Esta carpeta no contiene subcarpetas.';
        listEl.appendChild(empty);
      } else {
        dirs.forEach(dir => {
          const row = document.createElement('button');
          row.type = 'button';
          row.className = 'btn btn-sm';
          row.style.cssText = 'display: flex; align-items: center; gap: 8px; width: 100%; text-align: left; justify-content: flex-start; padding: 7px 10px; background: transparent; border: 1px solid transparent;';
          row.textContent = `📁 ${dir.name}`;
          row.title = dir.path;
          row.addEventListener('click', () => loadFolderBrowserPath(dir.path));
          listEl.appendChild(row);
        });
      }
    }
  } catch (e) {
    if (listEl) {
      listEl.innerHTML = `<div style="padding: 16px; color: #fca5a5;">Error al listar carpetas: ${escapeHtml(e.message || String(e))}</div>`;
    }
  }
}

function loadFolderBrowserParent() {
  if (folderBrowserParentPath) {
    loadFolderBrowserPath(folderBrowserParentPath);
  }
}

function confirmFolderBrowserSelection() {
  const pathInput = document.getElementById('folder-browser-path');
  const target = folderBrowserTargetInputId ? document.getElementById(folderBrowserTargetInputId) : null;
  if (target && pathInput && pathInput.value.trim()) {
    target.value = pathInput.value.trim();
  }
  closeFolderBrowser();
}

function closeFolderBrowser() {
  closeModal('folder-browser-modal');
  folderBrowserTargetInputId = null;
}

function openWebuiLoginModal() {
  const errEl = document.getElementById('webui-login-error');
  const passEl = document.getElementById('webui-login-password');
  if (errEl) {
    errEl.style.display = 'none';
    errEl.textContent = '';
  }
  if (passEl) {
    passEl.value = '';
  }
  openModal('webui-login-modal');
  setTimeout(() => {
    if (passEl) passEl.focus();
  }, 60);
}

async function submitWebuiLogin() {
  const passEl = document.getElementById('webui-login-password');
  const errEl = document.getElementById('webui-login-error');
  const submitBtn = document.getElementById('webui-login-submit-btn');
  const password = passEl ? passEl.value : '';

  if (errEl) {
    errEl.style.display = 'none';
    errEl.textContent = '';
  }
  if (submitBtn) submitBtn.disabled = true;

  try {
    const res = await fetch('/api/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ password }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok || !data.ok || !data.token) {
      if (errEl) {
        errEl.textContent = data.error || 'Contraseña incorrecta. Inténtalo de nuevo.';
        errEl.style.display = 'block';
      }
      if (passEl) {
        passEl.select();
        passEl.focus();
      }
      return;
    }
    sessionStorage.setItem('mule_webui_token', data.token);
    client.token = data.token;
    client.awaitingPassword = false;
    closeModal('webui-login-modal');
    client.connect();
  } catch (e) {
    if (errEl) {
      errEl.textContent = 'Error de conexión con el servidor: ' + (e.message || e);
      errEl.style.display = 'block';
    }
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
}

async function clearWebuiPassword() {
  if (!confirm('¿Deseas quitar la contraseña de la WebUI para que la red local acceda sin login?')) {
    return;
  }
  try {
    await client.call('settings.set', { webuiPassword: '' });
    sessionStorage.removeItem('mule_webui_token');
    const passEl = document.getElementById('pref-webui-password');
    if (passEl) passEl.value = '';
    await loadPreferences();
    if (typeof showToast === 'function') {
      showToast('Contraseña WebUI eliminada', 'info', '🔓');
    }
  } catch (e) {
    if (typeof showToast === 'function') {
      showToast('Error al quitar la contraseña: ' + (e.message || e), 'error', '❌');
    }
  }
}

async function exportFullBackup() {
  const btn = document.getElementById('btn-export-backup');
  const statusEl = document.getElementById('backup-status-msg');
  if (btn) btn.disabled = true;
  if (statusEl) statusEl.textContent = 'Generando copia de seguridad portátil...';

  try {
    const res = await client.call('settings.exportBackup');
    if (!res || !res.zipBase64) {
      throw new Error('El servidor no devolvió el archivo ZIP');
    }
    const binStr = atob(res.zipBase64);
    const bytes = new Uint8Array(binStr.length);
    for (let i = 0; i < binStr.length; i++) {
      bytes[i] = binStr.charCodeAt(i);
    }
    const blob = new Blob([bytes], { type: 'application/zip' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = res.fileName || 'pulse-core-backup.zip';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);

    const sum = res.summary || {};
    const sizeKb = Math.max(1, Math.round((res.sizeBytes || bytes.length) / 1024));
    const detail = `Copia generada (${sizeKb} KiB): ${sum.creditsCount || 0} créditos, ${sum.serversCount || 0} servidores, ${sum.kadContactsCount || 0} nodos Kad, ${sum.mediaCount || 0} títulos.`;
    if (statusEl) statusEl.textContent = '✅ ' + detail;
    if (typeof showToast === 'function') {
      showToast(detail, 'success', '📦');
    }
  } catch (e) {
    const msg = e.message || String(e);
    if (statusEl) statusEl.textContent = '❌ Error exportando copia: ' + msg;
    if (typeof showToast === 'function') {
      showToast('Error exportando copia: ' + msg, 'error', '❌');
    }
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function restoreFullBackupFromFile(inputEl) {
  const file = inputEl && inputEl.files && inputEl.files[0];
  if (!file) return;

  const btn = document.getElementById('btn-restore-backup');
  const statusEl = document.getElementById('backup-status-msg');
  const maxZipBytes = 64 * 1024 * 1024;
  if (file.size > maxZipBytes) {
    if (statusEl) statusEl.textContent = '❌ El archivo supera el límite máximo de 64 MiB.';
    inputEl.value = '';
    return;
  }
  if (!confirm(`¿Restaurar la copia "${file.name}"? Se importará la identidad RSA, créditos de pares, amigos, categorías, servidores, nodos Kad y ajustes compatibles.`)) {
    inputEl.value = '';
    return;
  }

  if (btn) btn.disabled = true;
  if (statusEl) statusEl.textContent = 'Subiendo y restaurando copia de seguridad...';

  try {
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    const chunkSize = 8192;
    let binary = '';
    for (let i = 0; i < bytes.length; i += chunkSize) {
      binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunkSize));
    }
    const zipBase64 = btoa(binary);

    const res = await client.call('settings.restoreBackup', { zipBase64 });
    const skippedMsg = res.settingsSkippedIncompatiblePaths > 0
      ? ` (${res.settingsSkippedIncompatiblePaths} rutas de otro SO omitidas)`
      : '';
    const summaryMsg = `Restauración completada${skippedMsg}: ${res.creditsRestored || 0} créditos, ${res.serversRestored || 0} servidores, ${res.kadContactsRestored || 0} nodos Kad${res.identityRestored ? ', identidad RSA actualizada' : ''}.`;
    if (statusEl) statusEl.textContent = '✅ ' + summaryMsg;
    if (typeof showToast === 'function') {
      showToast(summaryMsg, 'success', '✅');
    }

    if (typeof loadPreferences === 'function') await loadPreferences();
    if (typeof refreshServers === 'function') refreshServers();
    if (typeof refreshKad === 'function') refreshKad();
    if (typeof loadMediaLibrary === 'function') loadMediaLibrary();
  } catch (e) {
    const msg = e.message || String(e);
    if (statusEl) statusEl.textContent = '❌ Error restaurando copia: ' + msg;
    if (typeof showToast === 'function') {
      showToast('Error restaurando copia: ' + msg, 'error', '❌');
    }
  } finally {
    if (btn) btn.disabled = false;
    inputEl.value = '';
  }
}

window.openFolderBrowser = openFolderBrowser;
window.loadFolderBrowserPath = loadFolderBrowserPath;
window.loadFolderBrowserParent = loadFolderBrowserParent;
window.confirmFolderBrowserSelection = confirmFolderBrowserSelection;
window.closeFolderBrowser = closeFolderBrowser;
window.openWebuiLoginModal = openWebuiLoginModal;
window.submitWebuiLogin = submitWebuiLogin;
window.clearWebuiPassword = clearWebuiPassword;
window.exportFullBackup = exportFullBackup;
window.restoreFullBackupFromFile = restoreFullBackupFromFile;
window.toggleNotificationsPopover = toggleNotificationsPopover;
window.setNotificationFilter = setNotificationFilter;
window.markNotificationRead = markNotificationRead;
window.markAllNotificationsRead = markAllNotificationsRead;
window.clearNotificationsHistory = clearNotificationsHistory;
window.openNotificationSettingsTab = openNotificationSettingsTab;
window.toggleQuickBrowserNotifications = toggleQuickBrowserNotifications;
window.toggleQuickSoundNotifications = toggleQuickSoundNotifications;
window.enableBrowserNotifications = enableBrowserNotifications;
window.playNotificationChime = playNotificationChime;
window.saveNotificationConfig = saveNotificationConfig;
window.clearNotificationSecret = clearNotificationSecret;
window.testNotificationChannel = testNotificationChannel;

