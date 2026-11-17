// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_network_policy::{
    redact_socket_addr, AuditDecision, AuditEventType, ConnectionPurpose, NetworkPolicyManager,
    NetworkProfile, NetworkState, PolicyError, PolicyViolation, ResourceBudgets,
};
use std::net::SocketAddr;
use std::time::Duration;

#[test]
fn test_local_lab_profile_confinement() {
    let manager = NetworkPolicyManager::new(NetworkProfile::LocalLab, ResourceBudgets::default());

    // Loopback IPv4 y IPv6 permitidos
    let loopback_v4: SocketAddr = "127.0.0.1:39735".parse().unwrap();
    let permit = manager.request_tcp_connect(loopback_v4, ConnectionPurpose::Ed2kPeer);
    assert!(permit.is_ok());

    let loopback_v6: SocketAddr = "[::1]:39735".parse().unwrap();
    let permit_v6 = manager.request_tcp_connect(loopback_v6, ConnectionPurpose::Ed2kPeer);
    assert!(permit_v6.is_ok());

    // Pública no loopback rechazada
    let public_ip: SocketAddr = "91.121.162.140:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(public_ip, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::NonLoopbackDisallowedInLab(_))
    ));

    // LAN no loopback rechazada
    let lan_ip: SocketAddr = "192.168.1.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(lan_ip, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::NonLoopbackDisallowedInLab(_))
    ));
}

#[test]
fn test_public_preview_anti_ssrf_and_filtering() {
    let manager = NetworkPolicyManager::new(
        NetworkProfile::PublicEd2kPreview,
        ResourceBudgets::default(),
    );

    // IP pública enrutable permitida
    let public_ip: SocketAddr = "91.121.162.140:39735".parse().unwrap();
    assert!(manager
        .request_tcp_connect(public_ip, ConnectionPurpose::Ed2kPeer)
        .is_ok());

    // Privada RFC 1918 rechazada
    let lan_10: SocketAddr = "10.0.0.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(lan_10, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::PrivateLanDisallowed(_))
    ));

    let lan_172: SocketAddr = "172.20.0.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(lan_172, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::PrivateLanDisallowed(_))
    ));

    let lan_192: SocketAddr = "192.168.1.100:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(lan_192, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::PrivateLanDisallowed(_))
    ));

    // LAN privada IPv6 asignada a IPv4 rechazada
    let mapped_lan: SocketAddr = "[::ffff:192.168.1.1]:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(mapped_lan, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::PrivateLanDisallowed(_))
    ));

    // Loopback rechazada en perfil público
    let loopback: SocketAddr = "127.0.0.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(loopback, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::LoopbackDisallowedInPublicProfile(_))
    ));

    // Enlace local rechazada
    let link_local: SocketAddr = "169.254.10.20:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(link_local, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::LinkLocalDisallowed(_))
    ));

    // Multidifusión rechazada
    let multicast: SocketAddr = "224.0.0.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(multicast, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::MulticastDisallowed(_))
    ));

    // Difusión (broadcast) rechazada
    let broadcast: SocketAddr = "255.255.255.255:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(broadcast, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::BroadcastDisallowed(_))
    ));

    // Carrier-Grade NAT rechazada
    let cgnat: SocketAddr = "100.64.1.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_connect(cgnat, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::CarrierGradeNatDisallowed(_))
    ));

    // Puerto 0 rechazado
    let port_zero: SocketAddr = "91.121.162.140:0".parse().unwrap();
    let err = manager
        .request_tcp_connect(port_zero, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::InvalidPort(_))
    ));
}

#[test]
fn test_kad_udp_profile_enforcement() {
    let public_ip: SocketAddr = "91.121.162.140:39735".parse().unwrap();

    // En PublicEd2kPreview: KadNode no está permitido
    let manager = NetworkPolicyManager::new(
        NetworkProfile::PublicEd2kPreview,
        ResourceBudgets::default(),
    );
    let err = manager
        .request_tcp_connect(public_ip, ConnectionPurpose::KadNode)
        .unwrap_err();
    assert!(matches!(err, PolicyError::KadNotAllowedInProfile(_)));

    // En PublicEd2kKadPreview: KadNode está permitido
    manager.set_profile(NetworkProfile::PublicEd2kKadPreview);
    assert_eq!(manager.profile(), NetworkProfile::PublicEd2kKadPreview);
    let permit = manager.request_tcp_connect(public_ip, ConnectionPurpose::KadNode);
    assert!(permit.is_ok());
}

#[test]
fn test_resource_budgets_exhaustion_and_drop() {
    let budgets = ResourceBudgets {
        max_active_peer_connections: 2,
        max_active_server_connections: 1,
        ..Default::default()
    };

    let manager = NetworkPolicyManager::new(NetworkProfile::LocalLab, budgets);
    let target1: SocketAddr = "127.0.0.1:39735".parse().unwrap();
    let target2: SocketAddr = "127.0.0.1:49663".parse().unwrap();
    let target3: SocketAddr = "127.0.0.1:49664".parse().unwrap();

    // Adquirir 2 permisos de pares
    let permit1 = manager
        .request_tcp_connect(target1, ConnectionPurpose::Ed2kPeer)
        .unwrap();
    let permit2 = manager
        .request_tcp_connect(target2, ConnectionPurpose::Ed2kPeer)
        .unwrap();
    assert_eq!(manager.active_peer_connections_count(), 2);
    assert_eq!(manager.state(), NetworkState::Connected);

    // El 3er intento supera el presupuesto
    let err = manager
        .request_tcp_connect(target3, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert_eq!(
        err,
        PolicyError::BudgetExhausted("max_active_peer_connections")
    );

    // El presupuesto de conexiones de servidor es independiente
    let server_permit = manager
        .request_tcp_connect(target1, ConnectionPurpose::Ed2kServer)
        .unwrap();
    assert_eq!(manager.active_server_connections_count(), 1);

    // El 2º intento de servidor supera el presupuesto
    let err_server = manager
        .request_tcp_connect(target2, ConnectionPurpose::Ed2kServer)
        .unwrap_err();
    assert_eq!(
        err_server,
        PolicyError::BudgetExhausted("max_active_server_connections")
    );

    // Descartar permit1 -> permite una nueva conexión de par
    drop(permit1);
    assert_eq!(manager.active_peer_connections_count(), 1);

    let permit3 = manager.request_tcp_connect(target3, ConnectionPurpose::Ed2kPeer);
    assert!(permit3.is_ok());

    // Limpieza
    drop(permit2);
    drop(permit3);
    drop(server_permit);
    assert_eq!(manager.active_peer_connections_count(), 0);
    assert_eq!(manager.active_server_connections_count(), 0);
    assert_eq!(manager.state(), NetworkState::PreviewActive);
}

#[tokio::test]
async fn test_kill_switch_atomic_cancellation_and_reenable() {
    let manager = NetworkPolicyManager::new(NetworkProfile::LocalLab, ResourceBudgets::default());

    let target: SocketAddr = "127.0.0.1:39735".parse().unwrap();
    let permit = manager
        .request_tcp_connect(target, ConnectionPurpose::Ed2kPeer)
        .unwrap();
    assert!(!permit.is_cancelled());

    // Generar una tarea esperando la cancelación del permiso
    let cancel_token = permit.cancellation_token();
    let task = tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => false,
            _ = cancel_token.cancelled() => true,
        }
    });

    // Invocar el interruptor de corte (Kill Switch)
    manager.disconnect_all();
    assert_eq!(manager.state(), NetworkState::Disabled);
    assert!(permit.is_cancelled());

    let was_cancelled = task.await.unwrap();
    assert!(
        was_cancelled,
        "Active socket task should be woken up by cancellation token"
    );

    // Cualquier nueva petición debe fallar inmediatamente con NetworkDisabled
    let err = manager
        .request_tcp_connect(target, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert_eq!(err, PolicyError::NetworkDisabled);

    // Descartar el permiso simula que la tarea del socket finaliza y libera su permiso
    drop(permit);
    assert_eq!(manager.active_peer_connections_count(), 0);

    // Volver a habilitar la red
    manager.enable();
    assert_eq!(manager.state(), NetworkState::PreviewActive);

    // La nueva petición tiene éxito
    let new_permit = manager.request_tcp_connect(target, ConnectionPurpose::Ed2kPeer);
    assert!(new_permit.is_ok());
}

#[test]
fn test_redacted_audit_log() {
    let manager = NetworkPolicyManager::new(
        NetworkProfile::PublicEd2kPreview,
        ResourceBudgets::default(),
    );

    let public_ip: SocketAddr = "91.121.162.140:39735".parse().unwrap();
    let private_ip: SocketAddr = "192.168.1.50:39735".parse().unwrap();

    let _ = manager.request_tcp_connect(public_ip, ConnectionPurpose::Ed2kPeer);
    let _ = manager.request_tcp_connect(private_ip, ConnectionPurpose::Ed2kPeer);

    let entries = manager.audit_entries(10);
    assert!(entries.len() >= 2);

    // Comprobar redacción
    assert_eq!(redact_socket_addr(&public_ip), "91.121.*.*:39735");
    assert_eq!(redact_socket_addr(&private_ip), "192.168.*.*:39735");

    let loopback: SocketAddr = "127.0.0.1:39735".parse().unwrap();
    assert_eq!(redact_socket_addr(&loopback), "127.0.0.1:39735");

    // Verificar campos de la entrada reciente del registro
    let last = &entries[0]; // más reciente
    assert_eq!(last.decision, AuditDecision::Denied);
    assert_eq!(last.event_type, AuditEventType::ConnectAttempt);
    assert_eq!(last.redacted_target, Some("192.168.*.*:39735".to_string()));
}

#[test]
fn test_listener_policy_and_budget() {
    let budgets = ResourceBudgets {
        max_active_listeners: 1,
        ..Default::default()
    };

    let manager = NetworkPolicyManager::new(NetworkProfile::LocalLab, budgets);

    // Vinculación a loopback permitida
    let loopback_bind: SocketAddr = "127.0.0.1:39735".parse().unwrap();
    let permit = manager.request_tcp_listen(loopback_bind, ConnectionPurpose::Listener);
    assert!(permit.is_ok());
    let permit = permit.unwrap();
    assert_eq!(manager.active_listeners_count(), 1);

    // Superación del presupuesto de escuchas
    let loopback_bind2: SocketAddr = "127.0.0.1:49663".parse().unwrap();
    let err = manager
        .request_tcp_listen(loopback_bind2, ConnectionPurpose::Listener)
        .unwrap_err();
    assert_eq!(err, PolicyError::BudgetExhausted("max_active_listeners"));

    // Descartar el permiso libera presupuesto
    drop(permit);
    assert_eq!(manager.active_listeners_count(), 0);

    // Vinculación a no loopback rechazada en perfil Lab
    let lan_bind: SocketAddr = "192.168.1.1:39735".parse().unwrap();
    let err = manager
        .request_tcp_listen(lan_bind, ConnectionPurpose::Listener)
        .unwrap_err();
    assert!(matches!(
        err,
        PolicyError::AddressViolation(PolicyViolation::ListenOnNonLoopbackDisallowed(_))
    ));
}

#[test]
fn test_budget_mutation_and_validation() {
    let manager = NetworkPolicyManager::new(NetworkProfile::LocalLab, ResourceBudgets::default());

    // Actualización de presupuesto válida
    let new_budgets = ResourceBudgets {
        max_active_peer_connections: 50,
        ..Default::default()
    };
    assert!(manager.set_budgets(new_budgets.clone()).is_ok());
    assert_eq!(manager.budgets().max_active_peer_connections, 50);

    // Actualización de presupuesto inválida (cero conexiones máx.)
    let invalid_budgets = ResourceBudgets {
        max_active_peer_connections: 0,
        ..Default::default()
    };
    let err = manager.set_budgets(invalid_budgets).unwrap_err();
    assert!(matches!(err, PolicyError::InvalidBudgets(_)));

    // Límite superior estilo eMule (65000 aceptado, más allá rechazado)
    let max_ok = ResourceBudgets {
        max_active_peer_connections: mule_network_policy::MAX_PEER_CONNECTIONS_CAP,
        ..Default::default()
    };
    assert!(manager.set_budgets(max_ok).is_ok());
    let over_cap = ResourceBudgets {
        max_active_peer_connections:
            mule_network_policy::MAX_PEER_CONNECTIONS_CAP + 1,
        ..Default::default()
    };
    let err = manager.set_budgets(over_cap).unwrap_err();
    assert!(matches!(err, PolicyError::InvalidBudgets(_)));

    // Actualización de presupuesto inválida (max_total_buffer_bytes < max_frame_size_bytes)
    let invalid_buf = ResourceBudgets {
        max_total_buffer_bytes: 100,
        max_frame_size_bytes: 1000,
        ..Default::default()
    };
    let err = manager.set_budgets(invalid_buf).unwrap_err();
    assert!(matches!(err, PolicyError::InvalidBudgets(_)));
}

#[test]
fn test_ipv6_redaction_and_profile_parsing() {
    let ipv6_pub: SocketAddr = "[2001:db8:85a3::8a2e:370:7334]:39735".parse().unwrap();
    let redacted = redact_socket_addr(&ipv6_pub);
    assert_eq!(redacted, "[2001:db8:*:*:*:*:*:*]:39735");

    let ipv6_loopback: SocketAddr = "[::1]:39735".parse().unwrap();
    assert_eq!(redact_socket_addr(&ipv6_loopback), "[::1]:39735");

    // from_str y display de perfil
    assert_eq!(
        "local-lab".parse::<NetworkProfile>().unwrap(),
        NetworkProfile::LocalLab
    );
    assert_eq!(
        "public-ed2k-preview".parse::<NetworkProfile>().unwrap(),
        NetworkProfile::PublicEd2kPreview
    );
    assert_eq!(
        "public-ed2k-kad-preview".parse::<NetworkProfile>().unwrap(),
        NetworkProfile::PublicEd2kKadPreview
    );
    assert!("invalid-profile".parse::<NetworkProfile>().is_err());
    assert_eq!(NetworkProfile::LocalLab.to_string(), "local-lab");
}

#[test]
fn test_policy_ip_filter_blocking_and_pseudonyms() {
    let manager = NetworkPolicyManager::new(
        NetworkProfile::PublicEd2kPreview,
        ResourceBudgets::default(),
    );

    // Alimentar el filtro IP con IP bloqueada
    let filter_content = "198.051.100.000 - 198.051.100.255 , 100 , Malicious Range\n";
    manager.ip_filter().load_from_str(filter_content).unwrap();

    let blocked_target: SocketAddr = "198.51.100.10:39735".parse().unwrap();
    let allowed_target: SocketAddr = "198.51.101.10:39735".parse().unwrap();

    // Destino bloqueado devuelve PolicyError::IpFiltered
    let err = manager
        .request_tcp_connect(blocked_target, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    match err {
        PolicyError::IpFiltered(ip, level, desc) => {
            assert_eq!(ip, "198.51.100.10");
            assert_eq!(level, 100);
            assert_eq!(desc, "Malicious Range");
        }
        other => panic!("Expected IpFiltered error, got {:?}", other),
    }

    // Destino permitido tiene éxito
    let permit = manager.request_tcp_connect(allowed_target, ConnectionPurpose::Ed2kPeer);
    assert!(permit.is_ok());

    // Comprobar entradas de auditoría para bloqueo de filtro y seudonimización
    let entries = manager.audit_entries(10);
    let filter_entry = entries
        .iter()
        .find(|e| e.event_type == mule_network_policy::audit::AuditEventType::FilterBlocked)
        .unwrap();
    assert_eq!(
        filter_entry.decision,
        mule_network_policy::audit::AuditDecision::Denied
    );
    assert_eq!(
        filter_entry.pseudonymized_target,
        Some("peer-01:39735".to_string())
    );
    assert_eq!(
        filter_entry.redacted_target,
        Some("198.51.*.*:39735".to_string())
    );
}

#[test]
fn test_policy_profile_atomic_transition_disconnects_sockets() {
    let manager = NetworkPolicyManager::new(
        NetworkProfile::PublicEd2kPreview,
        ResourceBudgets::default(),
    );

    let target: SocketAddr = "198.51.101.10:39735".parse().unwrap();
    let permit = manager
        .request_tcp_connect(target, ConnectionPurpose::Ed2kPeer)
        .unwrap();
    assert!(!permit.is_cancelled());

    // Cambiar de nuevo a local-lab debe activar la cancelación de los permisos existentes
    manager.set_profile(NetworkProfile::LocalLab);
    assert!(permit.is_cancelled());
    assert_eq!(manager.profile(), NetworkProfile::LocalLab);

    // Nuevos permisos bajo local-lab para direcciones externas son rechazados por anti-SSRF
    let err = manager
        .request_tcp_connect(target, ConnectionPurpose::Ed2kPeer)
        .unwrap_err();
    assert!(matches!(err, PolicyError::AddressViolation(_)));
}
