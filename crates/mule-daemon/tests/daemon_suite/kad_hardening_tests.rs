// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Pruebas de endurecimiento Kad DHT (enlace compañero LowID saliente e intercambio ping/pong).

use std::time::Duration;

use mule_network_policy::{CancellationToken, NetworkProfile};
use mule_test_support::kad_state;
use tempfile::tempdir;

#[tokio::test]
async fn test_kad_outgoing_buddy_link_and_ping_pong_loopback() {
    let tmp_relay = tempdir().expect("relay tempdir");
    let relay = kad_state(&tmp_relay);
    relay.network_policy.set_profile(NetworkProfile::LocalLab);

    let relay_udp_addr = relay
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start relay kad udp");
    let relay_tcp_addr = relay
        .inbound_listener
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start relay inbound listener");

    {
        let tracker = relay.kad.udp().firewall_tracker();
        let mut fw = tracker.lock().await;
        fw.verified = true;
        fw.status = mule_daemon::services::UdpFirewallStatus::Open;
    }
    relay.kad.udp().set_buddy_tcp_info(relay_tcp_addr.port(), 0);

    let tmp_client = tempdir().expect("client tempdir");
    let client = kad_state(&tmp_client);
    client.network_policy.set_profile(NetworkProfile::LocalLab);

    let _client_udp_addr = client
        .kad
        .udp()
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start client kad udp");
    let client_tcp_addr = client
        .inbound_listener
        .start("127.0.0.1:0".parse().unwrap())
        .await
        .expect("start client inbound listener");
    client.kad.set_ed2k_tcp_port(client_tcp_addr.port());

    let cancel = CancellationToken::new();
    let connected_addr = client
        .kad
        .establish_buddy_link(relay_udp_addr, cancel.clone())
        .await
        .expect("el enlace buddy saliente debe establecerse en loopback");
    assert_eq!(connected_addr, relay_tcp_addr);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let relay_has_buddy = relay.kad.udp().buddy_registry().lock().await.active.is_some();
        let (client_addr, client_pong) = client.kad.outgoing_buddy_status().await;
        if relay_has_buddy && client_addr == Some(relay_tcp_addr) && client_pong.is_some() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "tiempo de espera agotado esperando OP_BUDDYPONG (relay_buddy={relay_has_buddy}, client_addr={client_addr:?}, client_pong={client_pong:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    cancel.cancel();
    client.inbound_listener.stop().await;
    relay.inbound_listener.stop().await;
    let _ = client.kad.udp().stop().await;
    let _ = relay.kad.udp().stop().await;
}
