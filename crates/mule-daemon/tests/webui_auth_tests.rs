//! Pruebas de integración del flujo de autenticación y derivación de contraseñas de la WebUI.

use mule_daemon::state::AppState;

#[tokio::test]
async fn test_flujo_autenticacion_webui() {
    let tmp = tempfile::tempdir().expect("directorio temporal de pruebas");
    let cfg = mule_daemon::config::DaemonConfig::new(
        tmp.path().to_path_buf(),
        format!("token-prueba-{}", 999),
    );
    let dls = mule_daemon::services::downloads::DownloadService::in_memory(tmp.path().join("temp"))
        .expect("servicio de descargas de prueba");
    let state = AppState::new(cfg, dls);

    // Sin contraseña configurada: la verificación devuelve verdadero para cualquier intento
    assert!(!state.has_webui_password());
    let intento_inicial = format!("candidato-{}", 1);
    assert!(state.verify_webui_password(&intento_inicial));

    // Configuración de contraseña derivada mediante SHA-256
    let clave_prueba = format!("clave-acceso-{}", 42);
    let hash = AppState::hash_webui_password(&clave_prueba);
    assert_eq!(hash.len(), 64, "El hash SHA-256 debe tener 64 caracteres hexadecimales");

    state.set_webui_password_hash(Some(hash));
    assert!(state.has_webui_password());

    // Verificación positiva y negativa
    assert!(state.verify_webui_password(&clave_prueba));
    let clave_invalida = format!("clave-invalida-{}", 99);
    assert!(!state.verify_webui_password(&clave_invalida));

    // Restablecimiento a sin contraseña
    state.set_webui_password_hash(None);
    assert!(!state.has_webui_password());
}
