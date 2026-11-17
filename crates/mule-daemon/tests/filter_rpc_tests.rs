// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use serde_json::json;
use std::fs::File;
use std::io::Write;

use mule_daemon::rpc::methods::handle_request;
use mule_test_support::{make_req, rpc_state};

#[tokio::test]
async fn test_filter_import_status_and_clear_rpc() {
    let (state, dir) = rpc_state("filter_test_token_123");
    let mut authenticated = false;

    // 1. Autenticar con system.hello
    let hello_req = make_req(
        1,
        "system.hello",
        Some(json!({
            "clientName": "test-client",
            "clientVersion": "0.1.0",
            "protocolVersion": 1,
            "token": "filter_test_token_123"
        })),
    );
    let resp = handle_request(&state, hello_req, &mut authenticated)
        .await
        .unwrap();
    assert!(resp.error.is_none());
    let caps = resp.result.unwrap()["capabilities"]
        .as_array()
        .unwrap()
        .clone();
    assert!(caps.iter().any(|c| c.as_str() == Some("ip-filter")));
    assert!(caps.iter().any(|c| c.as_str() == Some("rate-limits")));

    // 2. Estado inicial del filtro -> vacío
    let status_req = make_req(2, "filter.getStatus", None);
    let resp = handle_request(&state, status_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["range_count"], 0);
    assert_eq!(res["enabled"], true);

    // 3. Crear un archivo temporal ipfilter.dat
    let filter_path = dir.path().join("ipfilter.dat");
    {
        let mut file = File::create(&filter_path).unwrap();
        writeln!(file, "# Test Header").unwrap();
        writeln!(file, "001.000.000.000 - 001.000.000.255 , 100 , Bad Host A").unwrap();
        writeln!(file, "001.000.001.000 - 001.000.001.255 , 100 , Bad Host B").unwrap();
        writeln!(
            file,
            "040.000.000.000 - 040.255.255.255 , 080 , Blocked ISP"
        )
        .unwrap();
    }

    // 4. Importar archivo mediante filter.importFile — misma estructura que getStatus
    //    (el informe de importación se expone mediante `last_import_report`).
    let import_req = make_req(
        3,
        "filter.importFile",
        Some(json!({ "filePath": filter_path.to_str().unwrap() })),
    );
    let resp = handle_request(&state, import_req, &mut authenticated)
        .await
        .unwrap();
    assert!(resp.error.is_none());
    let res = resp.result.unwrap();
    assert_eq!(res["enabled"], true);
    assert_eq!(res["range_count"], 2);
    assert_eq!(res["last_import_report"]["valid_entries_parsed"], 3);
    assert_eq!(res["last_import_report"]["coalesced_count"], 1); // 2 adjacent ranges merged
    assert_eq!(res["last_import_report"]["final_range_count"], 2);

    // 5. Comprobar estadísticas del filtro
    let stats_req = make_req(4, "filter.getStats", None);
    let resp = handle_request(&state, stats_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["range_count"], 2);

    // 6. Deshabilitar/habilitar filtro — los rangos y el informe sobreviven a la
    //    conmutación (regresión: la tarjeta en la UI desaparecía al volver a habilitar).
    let set_enabled_req = make_req(5, "filter.setEnabled", Some(json!({ "enabled": false })));
    let resp = handle_request(&state, set_enabled_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["enabled"], false);
    assert_eq!(res["range_count"], 2);
    assert_eq!(res["last_import_report"]["final_range_count"], 2);

    let set_enabled_req = make_req(7, "filter.setEnabled", Some(json!({ "enabled": true })));
    let resp = handle_request(&state, set_enabled_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["enabled"], true);
    assert_eq!(res["range_count"], 2);
    assert_eq!(res["last_import_report"]["final_range_count"], 2);

    // 7. Limpiar filtro
    let clear_req = make_req(6, "filter.clear", None);
    let resp = handle_request(&state, clear_req, &mut authenticated)
        .await
        .unwrap();
    assert_eq!(resp.result.unwrap()["range_count"], 0);
}

#[tokio::test]
async fn test_rate_limits_get_and_set_rpc() {
    let (state, _dir) = rpc_state("filter_test_token_123");
    let mut authenticated = true;

    // 1. Obtener límites iniciales
    let get_req = make_req(1, "limits.get", None);
    let resp = handle_request(&state, get_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["downloadBytesPerSec"], 0);
    assert_eq!(res["uploadBytesPerSec"], 0);
    assert_eq!(res["connectionsPerMinute"], 120);

    // 2. Establecer límites personalizados
    let set_req = make_req(
        2,
        "limits.set",
        Some(json!({
            "downloadBytesPerSec": 1024000,
            "uploadBytesPerSec": 512000,
            "connectionsPerMinute": 60,
            "serverRequestsPerSec": 5,
            "kadPacketsPerSec": 20
        })),
    );
    let resp = handle_request(&state, set_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["downloadBytesPerSec"], 1024000);
    assert_eq!(res["uploadBytesPerSec"], 512000);
    assert_eq!(res["connectionsPerMinute"], 60);
    assert_eq!(res["serverRequestsPerSec"], 5);
    assert_eq!(res["kadPacketsPerSec"], 20);
}

/// `GetMaxDownloadInBytesPerSec` de eMule («don't be a Lam3r :)»): cuando
/// el límite de subida baja de 20 KB/s, la descarga *efectiva* se
/// limita a coef×subida, pero el valor *configurado* persistido permanece
/// intacto — subir la subida restablece la descarga sin volver a introducirla.
#[tokio::test]
async fn test_ud_ratio_caps_download_when_upload_low() {
    let (state, _dir) = rpc_state("filter_test_token_123");
    let mut authenticated = true;

    // Subida 5 KB/s (coef 4) + descarga 1000 KB/s → efectivo 20 KB/s,
    // configurado conservado.
    let set_req = make_req(
        1,
        "limits.set",
        Some(json!({
            "downloadBytesPerSec": 1_024_000,
            "uploadBytesPerSec": 5120
        })),
    );
    let resp = handle_request(&state, set_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["downloadBytesPerSec"], 20_480);
    assert_eq!(res["uploadBytesPerSec"], 5120);
    assert_eq!(res["downloadConfiguredBytesPerSec"], 1_024_000);
    assert_eq!(res["downloadRatioApplied"], true);

    // Subir la subida a 50 KB/s elimina el tope sin volver a introducir la
    // descarga configurada.
    let set_req = make_req(
        2,
        "limits.set",
        Some(json!({ "uploadBytesPerSec": 51_200 })),
    );
    let resp = handle_request(&state, set_req, &mut authenticated)
        .await
        .unwrap();
    let res = resp.result.unwrap();
    assert_eq!(res["downloadBytesPerSec"], 1_024_000);
    assert_eq!(res["downloadRatioApplied"], false);

    // Subida ilimitada (0) → sin ratio.
    let set_req = make_req(3, "limits.set", Some(json!({ "uploadBytesPerSec": 0 })));
    let resp = handle_request(&state, set_req, &mut authenticated)
        .await
        .unwrap();
    assert_eq!(resp.result.unwrap()["downloadBytesPerSec"], 1_024_000);
}
