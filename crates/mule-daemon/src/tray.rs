// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Icono en la bandeja del sistema (System Tray) para Windows.
//!
//! En Windows, `mule-daemon` puede mostrar un icono discreto en el área de
//! notificación con las acciones principales del servidor:
//! - Abrir la interfaz web (`WebUI`) en el navegador predeterminado
//! - Abrir la carpeta de descargas o la carpeta de datos
//! - Activar o desactivar las notificaciones discretas (fin de descarga / caída de servidor)
//! - Activar o desactivar el inicio automático al arrancar Windows
//! - Detener el servidor de forma ordenada
//!
//! En Linux/macOS es un no-op sin dependencias gráficas.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, Notify};

/// Configuración para el icono de la bandeja del sistema.
#[derive(Debug)]
pub struct TrayConfig {
    /// URL completa de la interfaz web (ej. `http://127.0.0.1:4660/`).
    pub web_url: String,
    /// Carpeta de descargas activas/completadas.
    pub downloads_dir: PathBuf,
    /// Carpeta de datos y configuración del daemon.
    pub data_dir: PathBuf,
    /// Señal de apagado ordenado del servidor.
    pub shutdown_notify: Arc<Notify>,
    /// Receptor opcional de tramas JSON-RPC de eventos para avisos discretos.
    pub event_rx: Option<broadcast::Receiver<String>>,
}

/// Controlador del hilo de la bandeja del sistema.
pub struct TrayHandle {
    #[cfg(windows)]
    hwnd: Arc<std::sync::atomic::AtomicIsize>,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TrayHandle {
    /// Cierra el icono de la bandeja y espera a que el hilo de mensajes termine.
    pub fn stop(self) {
        #[cfg(windows)]
        {
            platform::stop_tray(self.hwnd, self.thread);
        }
    }
}

/// Inicia el icono de la bandeja del sistema en un hilo dedicado (solo Windows).
pub fn spawn(config: TrayConfig) -> TrayHandle {
    #[cfg(windows)]
    {
        platform::spawn_tray(config)
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        TrayHandle {}
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BalloonNotice {
    pub title: String,
    pub body: String,
    pub is_warning: bool,
}

#[cfg(any(windows, test))]
fn evaluate_event_frame(
    raw: &str,
    was_connected: &mut bool,
    last_server_label: &mut String,
) -> Option<BalloonNotice> {
    if !raw.contains("\"event.downloadCompleted\"") && !raw.contains("\"server.sessionUpdated\"") {
        return None;
    }

    let frame: serde_json::Value = serde_json::from_str(raw).ok()?;
    let method = frame.get("method")?.as_str()?;
    let params = frame.get("params")?;

    match method {
        "event.downloadCompleted" => {
            let file_name = params
                .get("download")
                .and_then(|d| d.get("fileName"))
                .and_then(|v| v.as_str())?
                .trim();
            if file_name.is_empty() {
                return None;
            }
            Some(BalloonNotice {
                title: "Descarga completada".to_string(),
                body: file_name.to_string(),
                is_warning: false,
            })
        }
        "server.sessionUpdated" => {
            let session = params.get("session")?;
            let status = session.get("status")?.as_str()?;
            match status {
                "connected" => {
                    *was_connected = true;
                    let name = session
                        .get("serverName")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    let addr = session
                        .get("serverAddress")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    *last_server_label = name
                        .or(addr)
                        .unwrap_or("el servidor eD2K")
                        .to_string();
                    None
                }
                "disconnected" if *was_connected => {
                    *was_connected = false;
                    Some(BalloonNotice {
                        title: "Conexión eD2K perdida".to_string(),
                        body: format!("Se ha desconectado de {last_server_label}."),
                        is_warning: true,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(windows)]
mod platform {
    use super::{evaluate_event_frame, BalloonNotice, TrayConfig, TrayHandle};
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::broadcast;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO,
        NIIF_NOSOUND, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIN_SELECT, NOTIFYICONDATAW,
        NOTIFYICONDATAW_0,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
        GetWindowLongPtrW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW,
        SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage, GWLP_USERDATA,
        HICON, HWND_MESSAGE, IDI_APPLICATION, LR_DEFAULTCOLOR, MF_CHECKED, MF_GRAYED,
        MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SW_SHOWNORMAL, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU,
        WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };

    const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/app_icon.ico");
    const TRAY_ICON_ID: u32 = 1;
    const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
    const WM_TRAY_NOTIFY_BALLOON: u32 = WM_APP + 2;
    const NIN_KEYSELECT: u32 = NIN_SELECT + 1;
    const BALLOON_TIMEOUT_MS: u32 = 5_000;
    const MAX_PENDING_BALLOONS: usize = 32;

    const IDM_STATUS: usize = 1000;
    const IDM_OPEN_WEBUI: usize = 1001;
    const IDM_OPEN_DOWNLOADS: usize = 1002;
    const IDM_OPEN_DATA: usize = 1003;
    const IDM_EXIT: usize = 1004;
    const IDM_AUTOSTART: usize = 1005;
    const IDM_NOTIFICATIONS: usize = 1006;

    const RUN_REG_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_REG_VALUE: &str = "PulseCore";
    const CREATE_NO_WINDOW_FLAG: u32 = 0x0800_0000;

    struct TrayState {
        config: TrayConfig,
        hicon: HICON,
        hwnd_slot: Arc<AtomicIsize>,
        notifications_enabled: Arc<AtomicBool>,
        pending_balloons: Arc<Mutex<VecDeque<BalloonNotice>>>,
    }

    pub(super) fn spawn_tray(config: TrayConfig) -> TrayHandle {
        let hwnd_slot = Arc::new(AtomicIsize::new(0));
        let hwnd_slot_thread = hwnd_slot.clone();

        let thread = std::thread::Builder::new()
            .name("pulse-tray".to_string())
            .spawn(move || {
                if let Err(err) = run_tray_loop(config, hwnd_slot_thread) {
                    tracing::warn!("No se pudo inicializar el icono de bandeja: {err}");
                }
            })
            .ok();

        TrayHandle {
            hwnd: hwnd_slot,
            thread,
        }
    }

    pub(super) fn stop_tray(
        hwnd_slot: Arc<AtomicIsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    ) {
        let raw = hwnd_slot.load(Ordering::Acquire);
        if raw != 0 {
            let hwnd = HWND(raw as *mut c_void);
            unsafe {
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = thread {
            let _ = handle.join();
        }
    }

    fn run_tray_loop(
        mut config: TrayConfig,
        hwnd_slot: Arc<AtomicIsize>,
    ) -> Result<(), String> {
        let event_rx = config.event_rx.take();
        let notifications_enabled = Arc::new(AtomicBool::new(true));
        let pending_balloons = Arc::new(Mutex::new(VecDeque::new()));
        let web_url = config.web_url.clone();

        unsafe {
            let hmodule = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
            let hinstance: HINSTANCE = hmodule.into();
            let class_name = w!("PulseCoreTrayWindowClass");

            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wc);

            let state = Box::new(TrayState {
                config,
                hicon: load_tray_icon(),
                hwnd_slot: hwnd_slot.clone(),
                notifications_enabled: notifications_enabled.clone(),
                pending_balloons: pending_balloons.clone(),
            });
            let state_ptr = Box::into_raw(state);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("Pulse Core Tray"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                None,
                hinstance,
                None,
            )
            .map_err(|e| {
                let reclaimed = Box::from_raw(state_ptr);
                let _ = DestroyIcon(reclaimed.hicon);
                format!("CreateWindowExW: {e}")
            })?;

            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
            hwnd_slot.store(hwnd.0 as isize, Ordering::Release);

            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: WM_TRAY_CALLBACK,
                hIcon: (*state_ptr).hicon,
                ..Default::default()
            };
            fill_utf16_buf(&mut nid.szTip, &format!("Pulse Core — {web_url}"));

            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                let _ = DestroyWindow(hwnd);
                return Err("Shell_NotifyIconW(NIM_ADD) devolvió false".to_string());
            }

            if let Some(rx) = event_rx {
                spawn_event_listener(rx, hwnd_slot.clone(), notifications_enabled, pending_balloons);
            }

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            hwnd_slot.store(0, Ordering::Release);
            Ok(())
        }
    }

    fn spawn_event_listener(
        mut rx: broadcast::Receiver<String>,
        hwnd_slot: Arc<AtomicIsize>,
        notifications_enabled: Arc<AtomicBool>,
        pending_balloons: Arc<Mutex<VecDeque<BalloonNotice>>>,
    ) {
        let _ = std::thread::Builder::new()
            .name("pulse-tray-events".to_string())
            .spawn(move || {
                let mut was_connected = false;
                let mut last_server_label = String::from("el servidor eD2K");

                loop {
                    let raw = match rx.blocking_recv() {
                        Ok(msg) => msg,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    };

                    let raw_hwnd = hwnd_slot.load(Ordering::Acquire);
                    if raw_hwnd == 0 {
                        break;
                    }

                    if let Some(notice) =
                        evaluate_event_frame(&raw, &mut was_connected, &mut last_server_label)
                    {
                        if !notifications_enabled.load(Ordering::Relaxed) {
                            continue;
                        }
                        if let Ok(mut queue) = pending_balloons.lock() {
                            while queue.len() >= MAX_PENDING_BALLOONS {
                                queue.pop_front();
                            }
                            queue.push_back(notice);
                        }
                        let hwnd = HWND(raw_hwnd as *mut c_void);
                        unsafe {
                            let _ =
                                PostMessageW(hwnd, WM_TRAY_NOTIFY_BALLOON, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            });
    }

    unsafe fn flush_pending_balloons(hwnd: HWND, state: &TrayState) {
        let enabled = state.notifications_enabled.load(Ordering::Relaxed);
        loop {
            let notice = match state.pending_balloons.lock() {
                Ok(mut queue) => {
                    if !enabled {
                        queue.clear();
                        return;
                    }
                    queue.pop_front()
                }
                Err(_) => return,
            };
            let Some(notice) = notice else {
                break;
            };
            let flags = if notice.is_warning {
                NIIF_WARNING | NIIF_NOSOUND
            } else {
                NIIF_INFO | NIIF_NOSOUND
            };
            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                uFlags: NIF_INFO,
                dwInfoFlags: flags,
                Anonymous: NOTIFYICONDATAW_0 {
                    uTimeout: BALLOON_TIMEOUT_MS,
                },
                ..Default::default()
            };
            fill_utf16_buf(&mut nid.szInfoTitle, &notice.title);
            fill_utf16_buf(&mut nid.szInfo, &notice.body);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayState;

        match msg {
            WM_TRAY_CALLBACK => {
                let event = (lparam.0 as u32) & 0xFFFF;
                if let Some(state) = state_ptr.as_ref() {
                    if event == WM_LBUTTONUP
                        || event == WM_LBUTTONDBLCLK
                        || event == NIN_SELECT
                        || event == NIN_KEYSELECT
                    {
                        open_target(hwnd, &state.config.web_url);
                    } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                        show_context_menu(hwnd, state);
                    }
                }
                LRESULT(0)
            }
            WM_TRAY_NOTIFY_BALLOON => {
                if let Some(state) = state_ptr.as_ref() {
                    flush_pending_balloons(hwnd, state);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let cmd = wparam.0 & 0xFFFF;
                if let Some(state) = state_ptr.as_ref() {
                    match cmd {
                        IDM_OPEN_WEBUI => open_target(hwnd, &state.config.web_url),
                        IDM_OPEN_DOWNLOADS => {
                            open_path(hwnd, &state.config.downloads_dir);
                        }
                        IDM_OPEN_DATA => {
                            open_path(hwnd, &state.config.data_dir);
                        }
                        IDM_NOTIFICATIONS => {
                            let prev = state.notifications_enabled.load(Ordering::Relaxed);
                            state.notifications_enabled.store(!prev, Ordering::Relaxed);
                        }
                        IDM_AUTOSTART => {
                            let next = !is_autostart_enabled();
                            set_autostart_enabled(next);
                        }
                        IDM_EXIT => {
                            state.config.shutdown_notify.notify_one();
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: TRAY_ICON_ID,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                if !state_ptr.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let state = Box::from_raw(state_ptr);
                    state.hwnd_slot.store(0, Ordering::Release);
                    let _ = DestroyIcon(state.hicon);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn show_context_menu(hwnd: HWND, state: &TrayState) {
        let Ok(hmenu) = CreatePopupMenu() else {
            return;
        };

        let status_label = to_wide(&format!("Pulse Core ({})", state.config.web_url));
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | MF_GRAYED,
            IDM_STATUS,
            PCWSTR(status_label.as_ptr()),
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_OPEN_WEBUI, w!("Abrir WebUI"));
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_OPEN_DOWNLOADS,
            w!("Abrir carpeta de descargas"),
        );
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_OPEN_DATA,
            w!("Abrir carpeta de datos"),
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let notif_flag = if state.notifications_enabled.load(Ordering::Relaxed) {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | notif_flag,
            IDM_NOTIFICATIONS,
            w!("Notificaciones discretas"),
        );
        let autostart_flag = if is_autostart_enabled() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | autostart_flag,
            IDM_AUTOSTART,
            w!("Iniciar con Windows"),
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_EXIT, w!("Detener servidor"));

        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_ok() {
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(
                hmenu,
                TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                0,
                hwnd,
                None,
            );
        }
        let _ = DestroyMenu(hmenu);
    }

    unsafe fn open_path(hwnd: HWND, path: &Path) {
        let target = path.to_string_lossy();
        open_target(hwnd, &target);
    }

    unsafe fn open_target(hwnd: HWND, target: &str) {
        let wide = to_wide(target);
        let _ = ShellExecuteW(
            hwnd,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }

    fn is_autostart_enabled() -> bool {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        Command::new("reg.exe")
            .args(["query", RUN_REG_KEY, "/v", RUN_REG_VALUE])
            .creation_flags(CREATE_NO_WINDOW_FLAG)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn set_autostart_enabled(enable: bool) {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        let mut cmd = Command::new("reg.exe");
        cmd.creation_flags(CREATE_NO_WINDOW_FLAG)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if enable {
            let Ok(exe) = std::env::current_exe() else {
                tracing::warn!(
                    "No se pudo obtener la ruta del ejecutable para activar el inicio automático"
                );
                return;
            };
            let quoted_exe = format!("\"{}\"", exe.display());
            match cmd
                .args([
                    "add",
                    RUN_REG_KEY,
                    "/v",
                    RUN_REG_VALUE,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &quoted_exe,
                    "/f",
                ])
                .status()
            {
                Ok(status) if status.success() => {
                    tracing::info!("Inicio automático con Windows activado ({quoted_exe})");
                }
                Ok(status) => {
                    tracing::warn!(
                        "No se pudo activar el inicio automático con Windows ({status})"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        "Error al ejecutar reg.exe para activar el inicio automático: {err}"
                    );
                }
            }
        } else {
            match cmd
                .args(["delete", RUN_REG_KEY, "/v", RUN_REG_VALUE, "/f"])
                .status()
            {
                Ok(status) if status.success() => {
                    tracing::info!("Inicio automático con Windows desactivado");
                }
                Ok(status) => {
                    tracing::warn!(
                        "No se pudo desactivar el inicio automático con Windows ({status})"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        "Error al ejecutar reg.exe para desactivar el inicio automático: {err}"
                    );
                }
            }
        }
    }

    unsafe fn load_tray_icon() -> HICON {
        if let Some(slice) = select_ico_image(APP_ICON_BYTES, 32) {
            if let Ok(icon) = CreateIconFromResourceEx(slice, true, 0x0003_0000, 0, 0, LR_DEFAULTCOLOR) {
                return icon;
            }
        }
        LoadIconW(HINSTANCE::default(), IDI_APPLICATION).unwrap_or_default()
    }

    /// Selecciona la mejor entrada de un archivo `.ico` para el tamaño objetivo.
    fn select_ico_image(ico: &[u8], target_size: u16) -> Option<&[u8]> {
        if ico.len() < 6 {
            return None;
        }
        let reserved = u16::from_le_bytes([ico[0], ico[1]]);
        let kind = u16::from_le_bytes([ico[2], ico[3]]);
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        if reserved != 0 || kind != 1 || count == 0 {
            return None;
        }

        let mut best: Option<(u16, &[u8])> = None;
        for idx in 0..count {
            let base = 6 + idx * 16;
            if base + 16 > ico.len() {
                break;
            }
            let w = if ico[base] == 0 { 256 } else { ico[base] as u16 };
            let size = u32::from_le_bytes([ico[base + 8], ico[base + 9], ico[base + 10], ico[base + 11]]) as usize;
            let offset = u32::from_le_bytes([ico[base + 12], ico[base + 13], ico[base + 14], ico[base + 15]]) as usize;
            let Some(end) = offset.checked_add(size) else {
                continue;
            };
            if end > ico.len() || size == 0 {
                continue;
            }
            let diff = w.abs_diff(target_size);
            if best.is_none_or(|(best_diff, _)| diff < best_diff) {
                best = Some((diff, &ico[offset..end]));
            }
        }
        best.map(|(_, slice)| slice)
    }

    fn fill_utf16_buf(buf: &mut [u16], text: &str) {
        if buf.is_empty() {
            return;
        }
        let mut idx = 0usize;
        for unit in text.encode_utf16() {
            if idx + 1 >= buf.len() {
                break;
            }
            buf[idx] = unit;
            idx += 1;
        }
        buf[idx] = 0;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::downloads::types::DownloadSnapshot;
    use crate::services::server_session::types::{ServerConnectionStatus, ServerSessionSnapshot};
    use crate::services::DaemonEvent;
    use uuid::Uuid;

    #[test]
    fn evaluate_event_frame_detects_download_completion_and_server_drop() {
        let mut was_connected = false;
        let mut last_server = String::from("el servidor eD2K");

        // Una desconexión previa sin haber estado conectado no genera aviso.
        let initial_drop = DaemonEvent::ServerSessionUpdated {
            sequence: 1,
            session: ServerSessionSnapshot {
                status: ServerConnectionStatus::Disconnected,
                ..Default::default()
            },
        }
        .to_wire_json()
        .unwrap();
        assert_eq!(
            evaluate_event_frame(&initial_drop, &mut was_connected, &mut last_server),
            None
        );

        // Conectar memoriza el nombre del servidor.
        let connected = DaemonEvent::ServerSessionUpdated {
            sequence: 2,
            session: ServerSessionSnapshot {
                status: ServerConnectionStatus::Connected,
                server_name: Some("eMule Security No1".to_string()),
                server_address: Some("1.2.3.4:4661".to_string()),
                ..Default::default()
            },
        }
        .to_wire_json()
        .unwrap();
        assert_eq!(
            evaluate_event_frame(&connected, &mut was_connected, &mut last_server),
            None
        );
        assert!(was_connected);

        // Caída posterior genera aviso único con el servidor afectado.
        let dropped = DaemonEvent::ServerSessionUpdated {
            sequence: 3,
            session: ServerSessionSnapshot {
                status: ServerConnectionStatus::Disconnected,
                ..Default::default()
            },
        }
        .to_wire_json()
        .unwrap();
        assert_eq!(
            evaluate_event_frame(&dropped, &mut was_connected, &mut last_server),
            Some(BalloonNotice {
                title: "Conexión eD2K perdida".to_string(),
                body: "Se ha desconectado de eMule Security No1.".to_string(),
                is_warning: true,
            })
        );
        assert!(!was_connected);

        // Descarga completada genera aviso informativo con el nombre de archivo.
        let done = DaemonEvent::DownloadCompleted {
            sequence: 4,
            download: DownloadSnapshot {
                id: Uuid::nil(),
                file_name: "PulseCore-Ubuntu-ARM64.iso".to_string(),
                total_bytes: 1024,
                received_bytes: 1024,
                verified_bytes: 1024,
                progress: 1.0,
                status: "completed".to_string(),
                priority: "normal".to_string(),
                source_count: 1,
                download_rate_bps: 0,
                uploaded_bytes: 0,
                created_at_unix_ms: 0,
                updated_at_unix_ms: 0,
                category_id: None,
                eta_seconds: None,
                ed2k_hash: "0123456789ABCDEF0123456789ABCDEF".to_string(),
                parts: Vec::new(),
                pending_ranges: Vec::new(),
                max_sources: None,
                nas_copy_active: None,
                protocol: Some("ed2k".to_string()),
            },
        }
        .to_wire_json()
        .unwrap();
        assert_eq!(
            evaluate_event_frame(&done, &mut was_connected, &mut last_server),
            Some(BalloonNotice {
                title: "Descarga completada".to_string(),
                body: "PulseCore-Ubuntu-ARM64.iso".to_string(),
                is_warning: false,
            })
        );
    }
}
