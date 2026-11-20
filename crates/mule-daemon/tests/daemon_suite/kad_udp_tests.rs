// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use tempfile::tempdir;
use tokio::net::UdpSocket;

use mule_core::kad::KadId;
use mule_protocol::kad::*;
use mule_test_support::kad_state;

#[tokio::test]
async fn test_kad_udp_nominal_ping_pong_and_contact_verification() {
    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    // 1. Iniciar Kad UDP en un puerto efímero de loopback
    let bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    assert!(bound_addr.port() > 0);

    // 2. Iniciar un socket de par Kad simulado en loopback
    let mock_peer_socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind mock peer");
    let mock_peer_addr = mock_peer_socket.local_addr().expect("mock addr");

    // Generar par simulado que responde con Pong
    let mock_task = tokio::spawn(async move {
        let mut buf = vec![0u8; 1024];
        let (len, from_addr) = mock_peer_socket.recv_from(&mut buf).await.expect("recv");
        let decoded = KadCodec::decode(&buf[..len]).expect("decode mock ping");
        match decoded {
            KadMessage::Ping(_) => {
                let pong = KadCodec::encode_pong(from_addr.port());
                mock_peer_socket
                    .send_to(&pong, from_addr)
                    .await
                    .expect("send pong");
            }
            _ => panic!("Expected Ping"),
        }
    });

    // 3. Hacer ping al par simulado desde el daemon
    let target_kad_id = KadId::random();
    let latency_ms = state
        .kad
        .ping_contact(
            &mock_peer_addr.ip().to_string(),
            mock_peer_addr.port(),
            Some(&target_kad_id.to_hex()),
        )
        .await
        .expect("ping should succeed");

    assert!(latency_ms < 2000);
    mock_task.await.expect("mock peer task finishes");

    // 4. Verificar que el contacto quede marcado como verificado en el estado y la lista
    let status = state.kad.get_status().await;
    assert_eq!(status.packets_sent, 1);
    assert_eq!(status.packets_received, 1);
    assert_eq!(status.total_contacts, 1);

    let contacts = state.kad.list_contacts(10).await;
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].kad_id, target_kad_id.to_hex());
    assert!(contacts[0].verified);
    assert_eq!(contacts[0].fail_count, 0);
}

#[tokio::test]
async fn test_kad_udp_timeout_and_transaction_cleanup() {
    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    let _bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    // Vincular par simulado que ignora los paquetes entrantes
    let mock_peer_socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind mock peer");
    let mock_peer_addr = mock_peer_socket.local_addr().expect("mock addr");

    // Hacer ping a un par que no responde
    let err = state
        .kad
        .ping_contact(
            &mock_peer_addr.ip().to_string(),
            mock_peer_addr.port(),
            None,
        )
        .await
        .unwrap_err();

    assert!(err.contains("timed out"));

    let snap = state.kad.udp().get_snapshot().await;
    assert_eq!(snap.active_transactions, 0);
}

#[tokio::test]
async fn test_kad_udp_kill_switch_teardown() {
    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    let _bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    // Activar el interruptor de corte (kill switch)
    state.network_policy.disconnect_all();

    // El ping saliente debe fallar de inmediato bajo la directiva
    let err = state
        .kad
        .ping_contact("127.0.0.1", 39737, None)
        .await
        .unwrap_err();

    assert!(err.contains("disabled") || err.contains("stopped") || err.contains("Policy denied"));

    let snap = state.kad.udp().get_snapshot().await;
    assert_eq!(snap.active_transactions, 0);
}

/// Comprobación de cortafuegos UDP de extremo a extremo: un par eD2K simulado responde a nuestro OP_HELLO,
/// recibe OP_FWCHECKUDPREQ y envía KADEMLIA2_FIREWALLUDP a nuestro puerto
/// Kad UDP vinculado; la comprobación debe concluir como Abierta (Open).
#[tokio::test]
async fn test_udp_firewall_check_succeeds_via_mock_peer() {
    use bytes::BytesMut;
    use mule_daemon::services::UdpFirewallStatus;
    use mule_format::kad::{KadContactEntry, NodesDatFile};
    use mule_protocol::ed2k::obfuscation::WireObfuscationCipher;
    use mule_protocol::ed2k::{
        Ed2kEncoder, Ed2kHelloAnswer, OP_FWCHECKUDPREQ, OP_HELLO, PR_ED2K, PR_EMULE,
    };
    use mule_protocol::framing::PacketCodec;
    use mule_protocol::kad::MAGICVALUE_UDP_SYNC_CLIENT;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Reproduce `EncryptSendClient` de eMule (ruta reckey de Kad, sin relleno):
    /// permutación de cable con clave MD5(receiverKey:u16 rand) sobre todo lo posterior al
    /// byte 3.
    fn encrypt_kad_udp(plain: &[u8], receiver_key: u32, sender_key: u32) -> Vec<u8> {
        use md5::{Digest, Md5};
        let rand_part: u16 = 0xBEEF;
        let mut key_data = [0u8; 6];
        key_data[..4].copy_from_slice(&receiver_key.to_le_bytes());
        key_data[4..6].copy_from_slice(&rand_part.to_le_bytes());
        let md5_key: [u8; 16] = Md5::digest(key_data).into();
        let mut cipher = WireObfuscationCipher::new(&md5_key, true);
        let mut out = vec![0x46u8]; // random proto byte (not a valid Kad proto)
        out.extend_from_slice(&rand_part.to_le_bytes());
        let mut tail = Vec::new();
        tail.extend_from_slice(&MAGICVALUE_UDP_SYNC_CLIENT.to_le_bytes());
        tail.push(0u8);
        tail.extend_from_slice(&receiver_key.to_le_bytes());
        tail.extend_from_slice(&sender_key.to_le_bytes());
        tail.extend_from_slice(plain);
        cipher.process(&mut tail);
        out.extend_from_slice(&tail);
        out
    }

    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    // Socket UDP Kad del daemon = puerto interno bajo prueba.
    let bound = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");
    let intern_port = bound.port();

    // Par simulado: oyente TCP + socket UDP.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind tcp");
    let mock_tcp_port = listener.local_addr().unwrap().port();
    let mock_udp = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let mock_udp_port = mock_udp.local_addr().unwrap().port();

    let mock = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.expect("accept");
        let codec = PacketCodec::new();
        let encoder = Ed2kEncoder::default();
        let mut buf = BytesMut::new();
        let mut raw = [0u8; 4096];
        let mut pending_fw: Option<u32> = None;
        let mut hello_sent = false;
        loop {
            let n = tokio::time::timeout(std::time::Duration::from_secs(10), s.read(&mut raw))
                .await
                .expect("mock read timeout")
                .expect("read");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&raw[..n]);
            while let Ok(Some(pkt)) = codec.decode(&mut buf) {
                if pkt.protocol == PR_ED2K && pkt.opcode == OP_HELLO && !hello_sent {
                    hello_sent = true;
                    let ans = Ed2kHelloAnswer {
                        user_hash: [9u8; 16],
                        client_id: 0x0E00_0001,
                        tcp_port: mock_tcp_port,
                        tags: vec![],
                        unknown_tags: vec![],
                        server_ip: 0,
                        server_port: 0,
                    };
                    let p = encoder.encode_hello_answer(&ans).expect("encode answer");
                    let mut out = vec![p.protocol];
                    out.extend_from_slice(&(p.frame_len() as u32).to_le_bytes());
                    out.push(p.opcode);
                    out.extend_from_slice(&p.payload);
                    s.write_all(&out).await.expect("write answer");
                } else if pkt.protocol == PR_EMULE && pkt.opcode == OP_FWCHECKUDPREQ {
                    assert!(pkt.payload.len() >= 8, "fwcheck payload too short");
                    let intern = u16::from_le_bytes([pkt.payload[0], pkt.payload[1]]);
                    let extern_p = u16::from_le_bytes([pkt.payload[2], pkt.payload[3]]);
                    assert_eq!(intern, intern_port);
                    assert_eq!(extern_p, 0);
                    let sender_key = u32::from_le_bytes(pkt.payload[4..8].try_into().unwrap());
                    assert_ne!(sender_key, 0);
                    pending_fw = Some(sender_key);
                }
            }
            if let Some(sender_key) = pending_fw.take() {
                // Los pares eMule reales responden con el datagrama FIREWALLUDP
                // cifrado con RC4 bajo nuestra clave de remitente.
                let probe = KadCodec::encode_firewall_udp(0, intern_port);
                let enc = encrypt_kad_udp(&probe, sender_key, sender_key);
                mock_udp
                    .send_to(&enc, ("127.0.0.1", intern_port))
                    .await
                    .expect("send encrypted firewall udp");
            }
        }
    });

    // Registrar el simulador como un contacto Kad v8 verificado mediante la importación de nodes.dat.
    let contact = KadContactEntry::new(
        [7u8; 16],
        std::net::Ipv4Addr::LOCALHOST,
        mock_udp_port,
        mock_tcp_port,
        8,
    )
    .with_verified(true);
    let nodes = NodesDatFile::new(2, vec![contact]);
    let mut bytes = Vec::new();
    nodes.write(&mut bytes).unwrap();
    state
        .kad
        .import_nodes_dat(&bytes)
        .await
        .expect("import contact");

    let status = state.kad.run_udp_firewall_check().await;
    assert_eq!(status, UdpFirewallStatus::Open);

    // La instantánea de estado expone el estado verificado.
    let snap = state.kad.get_status().await;
    assert_eq!(snap.udp_firewall_status, "open");
    assert!(snap.udp_firewall_verified);
    assert_eq!(snap.effective_udp_port, intern_port);

    mock.abort();
}

/// Un candidato que se conecta pero nunca envía FIREWALLUDP no debe resolver una
/// entrada pendiente para una IP diferente (se ignoran los datagramas no solicitados).
#[tokio::test]
async fn test_udp_firewall_check_no_candidates_stays_unverified() {
    use mule_daemon::services::UdpFirewallStatus;

    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);
    state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    // Tabla de enrutamiento vacía → sin candidatos → vuelve a No verificado (Unverified).
    let status = state.kad.run_udp_firewall_check().await;
    assert_eq!(status, UdpFirewallStatus::Unverified);
}

/// Rol de retransmisión de compañero (buddy relay): FINDBUDDY_REQ de un par Kad se acepta solo si somos
/// un nodo verificado sin cortafuegos; el solicitante se registra como
/// compañero pendiente a la espera de su enlace TCP, y un CALLBACK_REQ sin un
/// enlace de compañero activo se descarta silenciosamente.
#[tokio::test]
async fn test_kad_buddy_findbuddy_accept_and_callback_drop() {
    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    let bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    // Simular que completamos la comprobación de cortafuegos UDP con éxito — eMule solo
    // acepta tareas de compañero en un nodo verificado sin cortafuegos.
    {
        let fw_tracker = state.kad.udp().firewall_tracker();
        let mut fw = fw_tracker.lock().await;
        fw.status = mule_daemon::services::UdpFirewallStatus::Open;
        fw.verified = true;
    }
    state.kad.udp().set_buddy_tcp_info(49662, 0);

    let mock = UdpSocket::bind("127.0.0.1:0").await.expect("bind mock");
    let mock_addr = mock.local_addr().unwrap();

    // 1. FINDBUDDY_REQ → se espera FINDBUDDY_RES reflejando nuestro buddy_id.
    let buddy_id = [0x77u8; 16];
    let user_id = [0x88u8; 16];
    let mut req = Vec::new();
    req.push(PR_KADEMLIA);
    req.push(KADEMLIA_FINDBUDDY_REQ);
    req.extend_from_slice(&buddy_id);
    req.extend_from_slice(&user_id);
    req.extend_from_slice(&5555u16.to_le_bytes());
    mock.send_to(&req, bound_addr)
        .await
        .expect("send findbuddy req");

    let mut buf = vec![0u8; 512];
    let (len, from) =
        tokio::time::timeout(std::time::Duration::from_secs(2), mock.recv_from(&mut buf))
            .await
            .expect("findbuddy res timeout")
            .expect("recv findbuddy res");
    assert_eq!(from, bound_addr);
    match KadCodec::decode(&buf[..len]).expect("decode res") {
        KadMessage::FindBuddyRes(r) => {
            assert_eq!(r.buddy_id, buddy_id);
            assert_eq!(r.tcp_port, 49662);
        }
        other => panic!("expected FindBuddyRes, got {other:?}"),
    }

    // Registro pendiente indexado por la IP del solicitante.
    let pending = state
        .kad
        .udp()
        .buddy_registry()
        .lock()
        .await
        .take_pending(&mock_addr.ip());
    let pb = pending.expect("pending buddy registered");
    assert_eq!(pb.buddy_id, buddy_id);
    assert_eq!(pb.user_id, user_id);
    assert_eq!(pb.tcp_port, 5555);

    // 2. CALLBACK_REQ sin enlace TCP de compañero activo → descartado, sin respuesta.
    let cb = KadCodec::encode_callback_req(&buddy_id, &[0x99u8; 16], 49662);
    mock.send_to(&cb, bound_addr)
        .await
        .expect("send callback req");
    let res = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        mock.recv_from(&mut buf),
    )
    .await;
    assert!(
        res.is_err(),
        "no datagram expected for dropped CALLBACK_REQ"
    );
}

/// Cuando el nodo no ha superado su comprobación de cortafuegos UDP, FINDBUDDY_REQ se
/// rechaza silenciosamente (eMule exige compañeros verificados y sin cortafuegos).
#[tokio::test]
async fn test_kad_buddy_findbuddy_rejected_when_unverified() {
    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    let bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");
    state.kad.udp().set_buddy_tcp_info(49662, 0);

    let mock = UdpSocket::bind("127.0.0.1:0").await.expect("bind mock");

    let mut req = Vec::new();
    req.push(PR_KADEMLIA);
    req.push(KADEMLIA_FINDBUDDY_REQ);
    req.extend_from_slice(&[0x11u8; 16]);
    req.extend_from_slice(&[0x22u8; 16]);
    req.extend_from_slice(&5555u16.to_le_bytes());
    mock.send_to(&req, bound_addr).await.expect("send");

    let mut buf = vec![0u8; 512];
    let res = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        mock.recv_from(&mut buf),
    )
    .await;
    assert!(
        res.is_err(),
        "FINDBUDDY_REQ must be refused when unverified"
    );
}

/// Publicar una nota en el rol de nodo de almacenamiento del daemon, y luego recuperarla mediante un
/// SearchNotesReq — ciclo completo de ida y vuelta a través del servicio UDP activo.
#[tokio::test]
async fn test_kad_notes_publish_and_search_roundtrip() {
    use mule_protocol::ed2k::tags::{Ed2kTag, TagName, TagValue};

    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    let bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    let mock = UdpSocket::bind("127.0.0.1:0").await.expect("bind mock");
    // El destino de publicación debe situarse dentro de SEARCHTOLERANCE del
    // KadID del daemon — eMule descarta silenciosamente las publicaciones fuera de tolerancia. Reutilizar el
    // ID local otorga distancia 0 y mantiene la prueba determinista.
    let file_hash = state.kad.local_id().0;
    let source_id = [0x77u8; 16];

    // 1. PublishNotesReq → el daemon la almacena y responde 0x4B con la carga.
    let tags = vec![
        Ed2kTag {
            name: TagName::Id(TAG_NOTE_FILENAME),
            value: TagValue::String("movie.avi".to_string()),
        },
        Ed2kTag {
            name: TagName::Id(TAG_NOTE_FILESIZE),
            value: TagValue::UInt64(700_000),
        },
        Ed2kTag {
            name: TagName::Id(TAG_NOTE_DESCRIPTION),
            value: TagValue::String("good copy".to_string()),
        },
        Ed2kTag {
            name: TagName::Id(TAG_NOTE_FILERATING),
            value: TagValue::UInt8(4),
        },
    ];
    let pub_req = KadCodec::encode_publish_notes_req(&file_hash, &source_id, &tags)
        .expect("encode publish notes req");
    mock.send_to(&pub_req, bound_addr).await.expect("send");

    let mut buf = vec![0u8; 2048];
    let (len, _) =
        tokio::time::timeout(std::time::Duration::from_secs(2), mock.recv_from(&mut buf))
            .await
            .expect("expected publish res")
            .expect("recv");
    match KadCodec::decode(&buf[..len]).expect("decode publish res") {
        KadMessage::PublishSourceRes(r) => {
            assert_eq!(r.target_file_hash, file_hash);
            assert!(r.status <= 100); // load percentage
        }
        other => panic!("expected PublishSourceRes (0x4B), got {other:?}"),
    }

    // 2. SearchNotesReq → el daemon responde con la nota almacenada en 0x3B.
    let search_req = KadCodec::encode_search_notes_req(&file_hash, 700_000);
    mock.send_to(&search_req, bound_addr).await.expect("send");

    let (len, _) =
        tokio::time::timeout(std::time::Duration::from_secs(2), mock.recv_from(&mut buf))
            .await
            .expect("expected search notes res")
            .expect("recv");
    let packet = &buf[..len];
    assert_eq!(packet[1], KADEMLIA2_SEARCH_RES);
    match KadCodec::decode_search_notes_res(packet[0], packet[1], &packet[2..])
        .expect("decode notes res")
    {
        KadMessage::SearchNotesRes(r) => {
            assert_eq!(r.target_file_hash, file_hash);
            assert_eq!(r.notes.len(), 1);
            assert_eq!(r.notes[0].source_id, source_id);
            assert_eq!(r.notes[0].comment(), Some("good copy"));
            assert_eq!(r.notes[0].rating(), Some(4));
        }
        other => panic!("expected SearchNotesRes, got {other:?}"),
    }

    // 3. Tamaño de archivo incorrecto → sin respuesta (SendValidNoteResult filtra).
    let bad_search = KadCodec::encode_search_notes_req(&file_hash, 12345);
    mock.send_to(&bad_search, bound_addr).await.expect("send");
    let res = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        mock.recv_from(&mut buf),
    )
    .await;
    assert!(
        res.is_err(),
        "size-mismatched notes query must get no reply"
    );
}

/// Veredicto de `CKademliaUDPListener::FirewalledCheck` + `Prefs` de eMule —
/// lado solicitante de la comprobación de cortafuegos TCP de Kad: mientras `m_uRecheckip < 4`,
/// un HELLO_REQ entrante desencadena un `FIREWALLED2_REQ` de vuelta; `FIREWALLED_RES`
/// avanza la ronda y `FIREWALLED_ACK_RES` cuenta un éxito de conexión de retorno.
#[tokio::test]
async fn test_kad_tcp_firewall_check_round() {
    let tmp = tempdir().expect("create tempdir");
    let state = kad_state(&tmp);
    state
        .network_policy
        .set_profile(mule_network_policy::NetworkProfile::LocalLab);

    // `FIREWALLED2_REQ` transporta nuestro puerto TCP eD2K + hash de usuario — ambos se
    // leen dinámicamente en tiempo de ejecución desde los atómicos compartidos de `KadUdpService`.
    state.kad.udp().set_buddy_tcp_info(49662, 0x03);
    state.kad.udp().set_fw_user_hash([0x42; 16]);

    let bound_addr = state
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start kad udp");

    // Un rastreador nuevo comienza a mitad de ronda (`m_uRecheckip = 0` en eMule).
    {
        let tracker = state.kad.udp().firewall_tracker();
        let fw = tracker.lock().await;
        assert!(fw.tcp_recheck_active());
        assert!(fw.tcp_is_firewalled());
    }

    let mock = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind mock peer");
    let mock_kad_id: [u8; 16] = [0xAB; 16];

    // HELLO_REQ v8 → respondemos HELLO_RES *y* FIREWALLED2_REQ.
    let hello = KadCodec::encode_hello_req(
        &mock_kad_id,
        mock.local_addr().unwrap().port(),
        8,
        49662,
        None,
    );
    mock.send_to(&hello, bound_addr).await.unwrap();

    let mut buf = vec![0u8; 2048];
    let mut saw_hello_res = false;
    let mut saw_fw_req2 = false;
    for _ in 0..4 {
        let (len, _) = tokio::time::timeout(std::time::Duration::from_secs(5), mock.recv_from(&mut buf))
            .await
            .expect("timed out waiting for HELLO_RES + FIREWALLED2_REQ")
            .unwrap();
        match KadCodec::decode(&buf[..len]).unwrap() {
            KadMessage::HelloRes(_) => saw_hello_res = true,
            KadMessage::FirewalledReq2(req) => {
                saw_fw_req2 = true;
                // Carga útil de eMule: nuestro puerto TCP eD2K + hash de usuario + opciones.
                assert_eq!(req.tcp_port, 49662);
                assert_eq!(req.user_hash, [0x42; 16]);
                assert_eq!(req.connect_options, 0x03);
            }
            other => panic!("unexpected datagram: {other:?}"),
        }
        if saw_hello_res && saw_fw_req2 {
            break;
        }
    }
    assert!(saw_hello_res && saw_fw_req2, "expected HELLO_RES + FIREWALLED2_REQ, got res={saw_hello_res} req={saw_fw_req2}");

    // La IP solicitante queda registrada (`AddKadFirewallRequest`) — su RES
    // avanza la ronda y registra la IP observada (cualquier u32 sirve aquí).
    let res = KadCodec::encode_firewalled_res(0x7F00_0001u32.to_le());
    mock.send_to(&res, bound_addr).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    {
        let tracker = state.kad.udp().firewall_tracker();
        let fw = tracker.lock().await;
        assert_eq!(fw.tcp_answers, 1);
        assert!(fw.observed_ip.is_some());
    }

    // Acuse de recibo UDP heredado (equivalente a < Kad7) cuenta un éxito de conexión de retorno.
    let ack = [mule_protocol::kad::PR_KADEMLIA, mule_protocol::kad::KADEMLIA_FIREWALLED_ACK_RES];
    mock.send_to(&ack, bound_addr).await.unwrap();
    mock.send_to(&ack, bound_addr).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    {
        let tracker = state.kad.udp().firewall_tracker();
        let fw = tracker.lock().await;
        assert_eq!(fw.tcp_acks, 2);
        assert!(!fw.tcp_is_firewalled(), "2 ACKs → TCP open verdict");
    }
}
