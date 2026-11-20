// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Pruebas de integración que verifican la compatibilidad con Sonarr / Radarr mediante Transmission RPC sobre HTTP.

use mule_daemon::WorkDirMode;
use mule_test_support::spawn_test_daemon_with_mode;
use reqwest::StatusCode;
use serde_json::{json, Value};

#[tokio::test]
async fn test_sonarr_transmission_handshake_and_lifecycle() {
    let (addr, _token, _dir) =
        spawn_test_daemon_with_mode("trans_test_token_456", WorkDirMode::Portable).await;
    let rpc_url = format!("http://{addr}/transmission/rpc");
    let client = reqwest::Client::new();

    // 1. Petición inicial sin token CSRF -> DEBE devolver 409 Conflict con cabecera X-Transmission-Session-Id
    let res = client
        .post(&rpc_url)
        .header("content-type", "application/json")
        .body(json!({ "method": "session-get" }).to_string())
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::CONFLICT);

    let session_header = res
        .headers()
        .get("x-transmission-session-id")
        .expect("Must have x-transmission-session-id header on 409")
        .to_str()
        .unwrap()
        .to_string();

    assert!(!session_header.is_empty());

    // 2. Reintento con cabecera de sesión capturada -> DEBE devolver 200 OK y detalles de sesión (comprobación del botón "Test" de Sonarr)
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(json!({ "method": "session-get", "tag": 100 }).to_string())
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(val["result"], "success");
    assert_eq!(val["tag"], 100);
    assert_eq!(val["arguments"]["rpc-version"], 17);
    assert!(val["arguments"]["download-dir"].as_str().is_some());

    // 3. Test `free-space`
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(json!({ "method": "free-space", "arguments": {}, "tag": 101 }).to_string())
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(val["result"], "success");
    assert!(val["arguments"]["size-bytes"].as_u64().unwrap() > 0);

    // 4. Test `torrent-add` with ed2k URI (Prowlarr / Sonarr release push)
    let ed2k_link = "ed2k://|file|Big_Buck_Bunny.1080p.mkv|10485760|31d6cfe0d16ae931b73c59d7e0c089c0|/";
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-add",
                "arguments": {
                    "filename": ed2k_link,
                    "labels": ["tv"]
                },
                "tag": 102
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(val["result"], "success");
    let added = &val["arguments"]["torrent-added"];
    assert_eq!(added["name"], "Big_Buck_Bunny.1080p.mkv");
    assert_eq!(
        added["hashString"],
        "31d6cfe0d16ae931b73c59d7e0c089c0"
    );
    let torrent_id = added["id"].as_i64().unwrap();

    // 5. Test `torrent-get` (Sonarr periodic queue polling)
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-get",
                "arguments": {
                    "fields": ["id", "name", "status", "percentDone", "labels", "downloadDir", "files"]
                },
                "tag": 103
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(val["result"], "success");
    let torrents = val["arguments"]["torrents"].as_array().unwrap();
    assert_eq!(torrents.len(), 1);
    assert_eq!(torrents[0]["id"], torrent_id);
    assert_eq!(torrents[0]["name"], "Big_Buck_Bunny.1080p.mkv");
    assert_eq!(torrents[0]["labels"][0], "tv");

    // 5b. Test `torrent-add` con un torrent de vídeo mínimo offline codificado en base64 (`metainfo`)
    let mut torrent_bytes = Vec::new();
    torrent_bytes.extend_from_slice(
        b"d8:announce25:http://127.0.0.1/announce4:infod6:lengthi1024e4:name16:sample_video.mkv12:piece lengthi16384e6:pieces20:",
    );
    torrent_bytes.extend_from_slice(&[0x41u8; 20]);
    torrent_bytes.extend_from_slice(b"ee");
    let metainfo_b64 = data_encoding::BASE64.encode(&torrent_bytes);
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-add",
                "arguments": {
                    "metainfo": metainfo_b64,
                    "labels": ["movies"]
                },
                "tag": 1031
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(val["result"], "success");
    let added_torrent = &val["arguments"]["torrent-added"];
    assert_eq!(added_torrent["name"], "sample_video.mkv");
    assert_eq!(
        added_torrent["hashString"]
            .as_str()
            .expect("hashString")
            .len(),
        40
    );
    let bittorrent_id = added_torrent["id"].as_i64().unwrap();

    // Verify torrent-get now reports BOTH protocols (2 items)
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-get",
                "arguments": { "fields": ["id", "hashString"] },
                "tag": 1032
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    let torrents = val["arguments"]["torrents"].as_array().unwrap();
    assert_eq!(torrents.len(), 2, "Expected 2 torrents (1 eD2k + 1 BitTorrent)");

    // 6. Test `torrent-stop` (Pause)
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-stop",
                "arguments": { "ids": [torrent_id] },
                "tag": 104
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    // 7. Test `torrent-start` (Resume)
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-start",
                "arguments": { "ids": [torrent_id] },
                "tag": 105
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    // 8. Test `torrent-remove` (Clean removal after import)
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-remove",
                "arguments": { "ids": [torrent_id, bittorrent_id], "delete-local-data": false },
                "tag": 106
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    // 9. Verify queue is now empty
    let res = client
        .post(&rpc_url)
        .header("x-transmission-session-id", &session_header)
        .header("content-type", "application/json")
        .body(
            json!({
                "method": "torrent-get",
                "arguments": { "fields": ["id"] },
                "tag": 107
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();

    let text = res.text().await.unwrap();
    let val: Value = serde_json::from_str(&text).unwrap();
    let torrents = val["arguments"]["torrents"].as_array().unwrap();
    assert_eq!(torrents.len(), 0);
}

#[tokio::test]
async fn test_embedded_web_ui_serving() {
    let (addr, _token, _dir) =
        spawn_test_daemon_with_mode("web_test_token_123", WorkDirMode::Portable).await;
    let client = reqwest::Client::new();

    // 1. GET / debe devolver index.html con 200 OK
    let res = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text().await.unwrap();
    assert!(html.contains("eMule-Rust"));

    // 2. GET /session-config.js debe devolver 200 OK con window.MULE_SESSION
    let res = client
        .get(format!("http://{addr}/session-config.js"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let js = res.text().await.unwrap();
    assert!(js.contains("window.MULE_SESSION="));

    // 3. SPA fallback on unknown route
    let res = client
        .get(format!("http://{addr}/downloads/view"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text().await.unwrap();
    assert!(html.contains("eMule-Rust"));
}
