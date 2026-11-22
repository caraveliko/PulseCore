/* ==========================================================================
   Controlador de atmósferas visuales y backdrop dinámico para Pulse Core
   Regla de idioma: todos los comentarios están redactados en español de España.
   ========================================================================== */

(() => {
  'use strict';

  const STORAGE_ATMOSPHERE_KEY = 'pulsecore_atmosphere';
  const STORAGE_INTENSITY_KEY = 'pulsecore_bg_intensity';
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
    try {
      savedAtm = localStorage.getItem(STORAGE_ATMOSPHERE_KEY) || 'aurora';
      savedIntensity = localStorage.getItem(STORAGE_INTENSITY_KEY) || '85';
    } catch (_) {
      /* Mantener valores predeterminados */
    }

    applyAtmosphere(savedAtm, false);
    applyAtmosphereIntensity(savedIntensity, false);

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
  window.toggleAtmospherePopover = toggleAtmospherePopover;
  window.clearMediaAmbientBackdrop = clearMediaBackdrop;

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initAtmosphereController);
  } else {
    initAtmosphereController();
  }
})();
