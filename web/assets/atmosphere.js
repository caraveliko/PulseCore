/* ==========================================================================
   Controlador de atmósferas visuales y backdrop dinámico para Pulse Core
   Regla de idioma: todos los comentarios están redactados en español de España.
   ========================================================================== */

(() => {
  'use strict';

  const STORAGE_ATMOSPHERE_KEY = 'pulsecore_atmosphere';
  const STORAGE_INTENSITY_KEY = 'pulsecore_bg_intensity';
  const STORAGE_DYNAMIC_FX_KEY = 'pulsecore_dynamic_fx';
  const P2P_ENERGY_MAX_BPS = 10 * 1024 * 1024;
  const SPOTLIGHT_SELECTOR = [
    '.media-card',
    '.movie-card',
    '.opt-card',
    '.shares-kpi-card',
    '.live-dl-card',
    '.metric-card',
    '.bw-kpi-card',
    '.atmosphere-card',
    '.share-folder-card',
  ].join(', ');
  const VALID_ATMOSPHERES = new Set([
    'aurora',
    'cinema',
    'silk',
    'obsidian',
    'black',
    'graphite',
    'midnight',
    'emerald',
    'violet',
  ]);

  const THEME_COLORS = {
    aurora: '#050810',
    cinema: '#08070a',
    silk: '#060911',
    obsidian: '#05070d',
    black: '#000000',
    graphite: '#111318',
    midnight: '#0a1020',
    emerald: '#071412',
    violet: '#0f0d17',
  };

  let mediaBackdropTimer = null;
  let lastBackdropUrl = '';
  let spotlightRafId = null;
  let pendingPointer = null;
  let lastSpotlightCard = null;

  /**
   * Aplica la atmósfera seleccionada al documento y actualiza controles visuales.
   */
  function applyAtmosphere(name, persist = true) {
    const chosen = VALID_ATMOSPHERES.has(name) ? name : 'aurora';
    document.documentElement.dataset.atmosphere = chosen;

    const metaTheme = document.querySelector('meta[name="theme-color"]');
    if (metaTheme) {
      metaTheme.setAttribute('content', THEME_COLORS[chosen] || '#050810');
    }

    document.querySelectorAll('.atmosphere-card[data-atmosphere-choice]').forEach((card) => {
      const active = card.getAttribute('data-atmosphere-choice') === chosen;
      card.classList.toggle('active', active);
      card.setAttribute('aria-pressed', active ? 'true' : 'false');
    });

    if (persist) {
      try {
        localStorage.setItem(STORAGE_ATMOSPHERE_KEY, chosen);
      } catch (_) {
        /* Ignorar si el almacenamiento local está restringido */
      }
    }
  }

  /**
   * Ajusta la opacidad de la capa atmosférica de fondo (0 a 100 %).
   */
  function applyAtmosphereIntensity(rawValue, persist = true) {
    const parsed = Number.parseInt(String(rawValue), 10);
    const pct = Number.isFinite(parsed) ? Math.min(100, Math.max(0, parsed)) : 85;
    document.documentElement.style.setProperty('--ambient-opacity', (pct / 100).toFixed(2));

    const slider = document.getElementById('atmosphere-intensity-range');
    if (slider && slider.value !== String(pct)) {
      slider.value = String(pct);
    }
    const label = document.getElementById('atmosphere-intensity-val');
    if (label) {
      label.textContent = `${pct}%`;
    }

    if (persist) {
      try {
        localStorage.setItem(STORAGE_INTENSITY_KEY, String(pct));
      } catch (_) {
        /* Ignorar si el almacenamiento local está restringido */
      }
    }
  }

  /**
   * Activa o desactiva los efectos dinámicos ambientales y el pulso reactivo P2P.
   */
  function applyAtmosphereDynamicFx(enabled, persist = true) {
    const isOn = Boolean(enabled);
    document.documentElement.dataset.dynamicFx = isOn ? 'on' : 'off';

    const toggle = document.getElementById('atmosphere-dynamic-toggle');
    if (toggle && toggle.checked !== isOn) {
      toggle.checked = isOn;
    }

    if (!isOn && lastSpotlightCard) {
      lastSpotlightCard.style.removeProperty('--spot-x');
      lastSpotlightCard.style.removeProperty('--spot-y');
      lastSpotlightCard = null;
    }

    if (persist) {
      try {
        localStorage.setItem(STORAGE_DYNAMIC_FX_KEY, isOn ? 'on' : 'off');
      } catch (_) {
        /* Ignorar si el almacenamiento local está restringido */
      }
    }
  }

  /**
   * Actualiza la energía visual de la atmósfera según el caudal P2P actual (bytes/s).
   */
  function updateAtmosphereTelemetry(downBps = 0, upBps = 0) {
    const down = Math.max(0, Number(downBps) || 0);
    const up = Math.max(0, Number(upBps) || 0);
    const totalWeighted = down + up * 1.2;
    const energy = Math.min(1, Math.sqrt(totalWeighted / P2P_ENERGY_MAX_BPS));
    const pulseDuration = (7.0 - energy * 4.8).toFixed(2);
    const rootStyle = document.documentElement.style;

    rootStyle.setProperty('--p2p-energy', energy.toFixed(3));
    rootStyle.setProperty('--p2p-pulse-duration', `${pulseDuration}s`);
    document.documentElement.dataset.p2pActive = totalWeighted > 1024 ? 'true' : 'false';
  }

  /**
   * Procesa el seguimiento especular bajo el cursor con requestAnimationFrame.
   */
  function flushSpotlightFrame() {
    spotlightRafId = null;
    if (!pendingPointer) return;
    if (document.documentElement.dataset.dynamicFx === 'off') {
      pendingPointer = null;
      return;
    }

    const { clientX, clientY, target } = pendingPointer;
    pendingPointer = null;

    const vw = window.innerWidth || 1;
    const vh = window.innerHeight || 1;
    const pctX = Math.max(0, Math.min(100, (clientX / vw) * 100)).toFixed(1);
    const pctY = Math.max(0, Math.min(100, (clientY / vh) * 100)).toFixed(1);
    document.documentElement.style.setProperty('--ambient-cursor-x', `${pctX}%`);
    document.documentElement.style.setProperty('--ambient-cursor-y', `${pctY}%`);

    const card = target && target.closest ? target.closest(SPOTLIGHT_SELECTOR) : null;
    if (lastSpotlightCard && lastSpotlightCard !== card) {
      lastSpotlightCard.style.removeProperty('--spot-x');
      lastSpotlightCard.style.removeProperty('--spot-y');
    }
    lastSpotlightCard = card;

    if (card) {
      const rect = card.getBoundingClientRect();
      card.style.setProperty('--spot-x', `${(clientX - rect.left).toFixed(1)}px`);
      card.style.setProperty('--spot-y', `${(clientY - rect.top).toFixed(1)}px`);
    }
  }

  /**
   * Abre o cierra el selector emergente de atmósferas de cabecera.
   */
  function toggleAtmospherePopover(event) {
    if (event) event.stopPropagation();
    const popover = document.getElementById('atmosphere-popover');
    const trigger = document.getElementById('btn-header-atmosphere');
    if (!popover) return;

    const willOpen = !popover.classList.contains('open');
    const notifPopover = document.getElementById('notifications-popover');
    const notifBtn = document.getElementById('btn-header-notifications');
    if (notifPopover) notifPopover.classList.remove('open');
    if (notifBtn) notifBtn.classList.remove('open');

    popover.classList.toggle('open', willOpen);
    if (trigger) {
      trigger.classList.toggle('open', willOpen);
      trigger.setAttribute('aria-expanded', willOpen ? 'true' : 'false');
    }
  }

  /**
   * Oculta la capa difuminada del póster cinematográfico activo.
   */
  function clearMediaBackdrop() {
    if (mediaBackdropTimer) {
      clearTimeout(mediaBackdropTimer);
      mediaBackdropTimer = null;
    }
    const layer = document.getElementById('ambient-media-backdrop');
    if (layer) {
      layer.classList.remove('visible');
    }
    lastBackdropUrl = '';
  }

  /**
   * Difumina suavemente el póster de la ficha o tarjeta bajo el puntero.
   */
  function showMediaBackdrop(url) {
    const layer = document.getElementById('ambient-media-backdrop');
    if (!layer || !url) return;

    if (mediaBackdropTimer) {
      clearTimeout(mediaBackdropTimer);
    }
    mediaBackdropTimer = setTimeout(() => {
      if (lastBackdropUrl !== url) {
        const escaped = url.replace(/"/g, '\\"');
        layer.style.backgroundImage = `url("${escaped}")`;
        lastBackdropUrl = url;
      }
      layer.classList.add('visible');
    }, 120);
  }

  /**
   * Inicializa eventos de cierre y el backdrop ambiental de Cine & Series.
   */
  function initAtmosphereController() {
    let savedAtm = 'aurora';
    let savedIntensity = '85';
    let savedDynamicFx = 'on';
    try {
      savedAtm = localStorage.getItem(STORAGE_ATMOSPHERE_KEY) || 'aurora';
      savedIntensity = localStorage.getItem(STORAGE_INTENSITY_KEY) || '85';
      savedDynamicFx = localStorage.getItem(STORAGE_DYNAMIC_FX_KEY) || 'on';
    } catch (_) {
      /* Mantener valores predeterminados */
    }

    applyAtmosphere(savedAtm, false);
    applyAtmosphereIntensity(savedIntensity, false);
    applyAtmosphereDynamicFx(savedDynamicFx !== 'off', false);
    updateAtmosphereTelemetry(0, 0);

    document.addEventListener('pointermove', (ev) => {
      if (document.documentElement.dataset.dynamicFx === 'off') return;
      pendingPointer = {
        clientX: ev.clientX,
        clientY: ev.clientY,
        target: ev.target,
      };
      if (!spotlightRafId) {
        spotlightRafId = requestAnimationFrame(flushSpotlightFrame);
      }
    }, { passive: true });

    document.addEventListener('click', (ev) => {
      const container = document.getElementById('atmosphere-header-container');
      const popover = document.getElementById('atmosphere-popover');
      const trigger = document.getElementById('btn-header-atmosphere');
      if (popover && popover.classList.contains('open') && container && !container.contains(ev.target)) {
        popover.classList.remove('open');
        if (trigger) {
          trigger.classList.remove('open');
          trigger.setAttribute('aria-expanded', 'false');
        }
      }
    }, true);

    document.addEventListener('keydown', (ev) => {
      if (ev.key === 'Escape') {
        const popover = document.getElementById('atmosphere-popover');
        const trigger = document.getElementById('btn-header-atmosphere');
        if (popover && popover.classList.contains('open')) {
          popover.classList.remove('open');
          if (trigger) {
            trigger.classList.remove('open');
            trigger.setAttribute('aria-expanded', 'false');
          }
        }
      }
    });

    const mediaGrid = document.getElementById('media-grid-container');
    if (mediaGrid) {
      mediaGrid.addEventListener('mouseover', (ev) => {
        const card = ev.target && ev.target.closest ? ev.target.closest('.media-card') : null;
        if (!card) return;
        const img = card.querySelector('img.media-poster-img');
        if (img && img.currentSrc && img.style.display !== 'none') {
          showMediaBackdrop(img.currentSrc);
        } else if (img && img.getAttribute('src') && img.style.display !== 'none') {
          showMediaBackdrop(img.getAttribute('src'));
        }
      });

      mediaGrid.addEventListener('mouseleave', () => {
        clearMediaBackdrop();
      });
    }

    document.querySelectorAll('.nav-btn[data-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        if (btn.getAttribute('data-tab') !== 'tab-media') {
          clearMediaBackdrop();
        }
      });
    });
  }

  window.setAtmosphere = (name) => applyAtmosphere(name, true);
  window.setAtmosphereIntensity = (val) => applyAtmosphereIntensity(val, true);
  window.setAtmosphereDynamicFx = (enabled) => applyAtmosphereDynamicFx(Boolean(enabled), true);
  window.updateAtmosphereTelemetry = updateAtmosphereTelemetry;
  window.toggleAtmospherePopover = toggleAtmospherePopover;
  window.clearMediaAmbientBackdrop = clearMediaBackdrop;

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initAtmosphereController);
  } else {
    initAtmosphereController();
  }
})();
