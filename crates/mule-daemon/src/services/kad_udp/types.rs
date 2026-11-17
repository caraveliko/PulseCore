// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex};

use mule_protocol::kad::KadMessage;

/// Tiempo de espera de transacción en segundos.
pub const KAD_TRANSACTION_TIMEOUT_SECS: u64 = 3;

/// Número máximo de transacciones en curso concurrentes para evitar el agotamiento de memoria.
pub const MAX_IN_FLIGHT_TRANSACTIONS: usize = 200;

/// Solicitudes máximas por segundo permitidas por IP de origen (anti-amplificación / límite de tasa).
pub const MAX_KAD_REQUESTS_PER_SEC_PER_IP: u32 = 120;

/// Limitador de tasa deslizante simple por IP.
#[derive(Debug, Default)]
pub struct IpRateLimiter {
    counts: HashMap<IpAddr, (u32, Instant)>,
}

impl IpRateLimiter {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    pub fn check_and_record(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        if self.counts.len() > 1000 {
            self.counts
                .retain(|_, (_, ts)| now.duration_since(*ts) < Duration::from_secs(10));
        }

        let entry = self.counts.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) >= Duration::from_secs(1) {
            *entry = (1, now);
            true
        } else if entry.0 < MAX_KAD_REQUESTS_PER_SEC_PER_IP {
            entry.0 += 1;
            true
        } else {
            false
        }
    }
}

/// Variante de respuesta esperada para una transacción en curso pendiente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KadExpectedResponse {
    Pong,
    HelloRes,
    BootstrapRes,
    FindNodeRes,
    SearchSourceRes,
    SearchKeywordRes,
    PublishSourceRes,
    PublishKeywordRes,
    SearchNotesRes,
    PublishNotesRes,
}

/// Estado de transacción en curso pendiente.
pub struct InFlightTransaction {
    pub target: SocketAddr,
    pub expected_response: KadExpectedResponse,
    pub expected_target_hash: Option<[u8; 16]>,
    pub created_at: Instant,
    /// Las respuestas multipaquete (SEARCH_*_RES llegan como varios lotes de
    /// ~50 entradas) necesitan un canal que permanezca abierto tras el primer paquete —
    /// por lo que se utiliza un mpsc sin límite en vez de oneshot.
    pub sender: mpsc::UnboundedSender<Result<KadMessage, String>>,
}

/// Guarda RAII para una transacción en curso registrada: elimina la entrada al
/// destruirse. Quienes llamen pueden envolver un futuro `send_*` en un tiempo de espera
/// externo o rama `select!` — la cancelación omitiría entonces el `remove` manual
/// de la rama de tiempo de espera, filtrando la entrada hacia `MAX_IN_FLIGHT_TRANSACTIONS`
/// hasta saturar el mapa y hacer fallar cada envío posterior. `try_lock` mantiene sincrónico
/// el `Drop`; un mapa con contención se sanea mediante el barrido de entradas obsoletas en
/// `insert_in_flight`. Eliminar un `tx_id` ya consumido no produce efecto.
pub struct InFlightTxGuard {
    map: InFlightMap,
    tx_id: u32,
}

impl InFlightTxGuard {
    /// Crea una guarda que elimina `tx_id` de `map` al destruirse.
    pub fn new(map: InFlightMap, tx_id: u32) -> Self {
        Self { map, tx_id }
    }
}

impl Drop for InFlightTxGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.map.try_lock() {
            map.remove(&self.tx_id);
        }
    }
}

/// Instantánea de estadísticas del servicio UDP Kad.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KadUdpSnapshot {
    pub is_active: bool,
    pub bind_address: Option<String>,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub active_transactions: usize,
}

pub type InFlightMap = Arc<Mutex<HashMap<u32, InFlightTransaction>>>;

/// Estado de cortafuegos/accesibilidad UDP para el puerto Kad local
/// (refleja `CUDPFirewallTester` de eMule: `m_bFirewalledUDP` + flag de verificación).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UdpFirewallStatus {
    /// No se ha ejecutado ninguna prueba concluyente todavía (en eMule `!m_bIsFWVerifiedUDP`).
    #[default]
    Unverified,
    /// Hay una comprobación en curso.
    Testing,
    /// Se recibió al menos un paquete `KADEMLIA2_FIREWALLUDP` → puerto abierto.
    Open,
    /// Todos los candidatos respondieron/fallaron sin un paquete positivo → tras cortafuegos (firewalled).
    Firewalled,
}

impl UdpFirewallStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Testing => "testing",
            Self::Open => "open",
            Self::Firewalled => "firewalled",
        }
    }
}

/// Resultado entregado a una prueba de cortafuegos pendiente cuando llega
/// un datagrama `KADEMLIA2_FIREWALLUDP` desde una IP candidata.
#[derive(Debug, Clone, Copy)]
pub struct FirewallUdpResult {
    pub error_code: u8,
    /// Puerto UDP que el par remoto sondeó (su vista de nuestro puerto interno/externo).
    pub port_tested: u16,
}

/// `KADEMLIAFIREWALLCHECKS` — Solicitudes de comprobación de cortafuegos TCP emitidas por
/// ronda de recomprobación (eMule `Opcodes.h:86`). Una respuesta `KADEMLIA_FIREWALLED_RES`
/// por solicitud; la ronda finaliza una vez que 4 contactos responden.
pub const TCP_FW_CHECKS_PER_ROUND: u32 = 4;

/// Número de confirmaciones (ACK) exitosas de conexión TCP de retorno (`OP_KAD_FWTCPCHECK_ACK` /
/// legado `KADEMLIA_FIREWALLED_ACK_RES`) necesarias dentro de una ronda para concluir
/// que nuestro puerto TCP es accesible (eMule `Prefs.cpp:166` — `m_uFirewalled < 2`).
pub const TCP_FW_ACK_THRESHOLD: u32 = 2;

/// TTL de una solicitud de conexión de retorno pendiente (eMule `ClientList.cpp:855`,
/// `SEC2MS(180)` — `listFirewallCheckRequests`).
pub const TCP_FW_CHECK_IP_TTL: Duration = Duration::from_secs(180);

/// Intervalo entre dos recomprobaciones de cortafuegos TCP (eMule `Kademlia.cpp:121`,
/// `m_tNextFirewallCheck = tNow + HR2S(1)`).
pub const TCP_FW_RECHECK_INTERVAL: Duration = Duration::from_secs(3600);

/// Rastrea las comprobaciones salientes pendientes de cortafuegos UDP y el último estado conocido.
/// Los candidatos pendientes están indexados por IPv4 para ignorar datagramas
/// `KADEMLIA2_FIREWALLUDP` no solicitados o dispersos (comprobación `bRequested` en eMule).
///
/// La estructura también contiene el veredicto **TCP** — eMule lo mantiene en `CPrefs`
/// (`m_uFirewalled`/`m_uRecheckip`/`m_bLastFirewallState` +
/// `CClientList::listFirewallCheckRequests`): mientras una ronda de recomprobación esté
/// activa, cada `HELLO_REQ`/`HELLO_RES` entrante desencadena un `FIREWALLED{2}_REQ`
/// de vuelta al remitente; ese contacto se conecta por TCP a nosotros y reconoce
/// con `OP_KAD_FWTCPCHECK_ACK` (Kad ≥ 7) o un `KADEMLIA_FIREWALLED_ACK_RES` UDP (< 7).
/// Dos ACK de conexión de retorno en una ronda demuestran que el puerto está abierto;
/// una ronda completada con menos ACK = tras cortafuegos (firewalled).
#[derive(Debug)]
pub struct FirewallTestTracker {
    pub status: UdpFirewallStatus,
    /// Verdadero una vez que una comprobación produjo un resultado concluyente (abierto o tras cortafuegos).
    pub verified: bool,
    /// Puerto Kad UDP externo aprendido a partir de un sondeo que alcanzó un puerto diferente
    /// del vinculado (mapeado por NAT). `use_extern` refleja la preferencia
    /// `GetUseExternKadPort` de eMule.
    pub extern_port: Option<u16>,
    pub use_extern: bool,
    /// Nuestra IPv4 pública según fue observada por un par (`KADEMLIA_FIREWALLED_RES`
    /// la devuelve en eco). Empleada como IP anunciada al publicarnos a nosotros mismos
    /// como fuente — la dirección de vinculación del socket puede ser `0.0.0.0` o estar tras NAT.
    pub observed_ip: Option<Ipv4Addr>,
    /// Momento de la última comprobación completada (para enfriamientos / reverificación).
    pub last_check: Option<Instant>,
    /// Pruebas pendientes por IP candidata: `(verify_key, result_sender)`. La clave de
    /// verificación es la `sender_key` que enviamos en `OP_FWCHECKUDPREQ` — los pares
    /// remotos cifran su respuesta `FIREWALLUDP` con ella.
    pending: HashMap<IpAddr, Vec<(u32, oneshot::Sender<FirewallUdpResult>)>>,
    /// Confirmaciones (ACK) de conexión de retorno recibidas durante la ronda actual
    /// (eMule `m_uFirewalled`).
    pub tcp_acks: u32,
    /// Respuestas `FIREWALLED_RES` recibidas durante la ronda actual
    /// (eMule `m_uRecheckip`) — la ronda permanece activa por debajo de
    /// [`TCP_FW_CHECKS_PER_ROUND`].
    pub tcp_answers: u32,
    /// Instantánea del veredicto al inicio de la ronda (eMule `m_bLastFirewallState`,
    /// inicial `true` — tras cortafuegos hasta demostrarse abierto).
    pub tcp_last_state: bool,
    /// Momento en que vence la próxima recomprobación horaria (eMule `m_tNextFirewallCheck`).
    pub next_tcp_recheck: Instant,
    /// IPs a las que pedimos conectarse por TCP de retorno hacia nosotros, con marca temporal
    /// (eMule `listFirewallCheckRequests` — `AddKadFirewallRequest`).
    pending_tcp_ips: Vec<(IpAddr, Instant)>,
}

impl Default for FirewallTestTracker {
    fn default() -> Self {
        Self {
            status: UdpFirewallStatus::Unverified,
            verified: false,
            extern_port: None,
            use_extern: false,
            observed_ip: None,
            last_check: None,
            pending: HashMap::new(),
            // eMule arranca con `m_uRecheckip = 0` → una ronda se activa tan pronto
            // llega el primer HELLO («primera comprobación efectuada al conectar»).
            tcp_acks: 0,
            tcp_answers: 0,
            tcp_last_state: true,
            next_tcp_recheck: Instant::now() + TCP_FW_RECHECK_INTERVAL,
            pending_tcp_ips: Vec::new(),
        }
    }
}

impl FirewallTestTracker {
    /// Registra una prueba pendiente para `candidate_ip`. Devuelve un receptor que
    /// se resuelve con el primer datagrama `KADEMLIA2_FIREWALLUDP` coincidente.
    pub fn register_candidate(
        &mut self,
        candidate_ip: IpAddr,
        verify_key: u32,
    ) -> oneshot::Receiver<FirewallUdpResult> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .entry(candidate_ip)
            .or_default()
            .push((verify_key, tx));
        rx
    }

    /// Elimina todas las pruebas pendientes para una IP (ruta de tiempo de espera/cancelación).
    pub fn unregister_candidate(&mut self, candidate_ip: &IpAddr) {
        self.pending.remove(candidate_ip);
    }

    /// Claves de verificación UDP de pruebas pendientes para `ip` — utilizadas para intentar
    /// descifrar datagramas Kad ofuscados procedentes de ese par.
    pub fn pending_verify_keys(&self, ip: &IpAddr) -> Vec<u32> {
        self.pending
            .get(ip)
            .map(|v| v.iter().map(|(k, _)| *k).collect())
            .unwrap_or_default()
    }

    /// Enruta un datagrama FIREWALLUDP entrante hacia la prueba pendiente coincidente.
    /// Devuelve false cuando nunca se solicitó a la IP de origen (paquete no solicitado).
    pub fn notify_packet(&mut self, from_ip: IpAddr, result: FirewallUdpResult) -> bool {
        match self.pending.get_mut(&from_ip) {
            Some(v) if !v.is_empty() => {
                // Un datagrama exitoso resuelve la prueba; los datagramas adicionales
                // (par interno+externo) se entregan a los receptores restantes si los hubiera.
                let (_, tx) = v.remove(0);
                if v.is_empty() {
                    self.pending.remove(&from_ip);
                }
                tx.send(result).is_ok()
            }
            _ => false,
        }
    }

    /// Indica si alguna comprobación de cortafuegos está esperando datagramas actualmente.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// eMule `GetRecheckIP` (`m_uRecheckip < KADEMLIAFIREWALLCHECKS`):
    /// mientras sea verdadero, cada HELLO_REQ/HELLO_RES entrante desencadena
    /// un `FIREWALLED{2}_REQ` de vuelta al remitente.
    pub fn tcp_recheck_active(&self) -> bool {
        self.tcp_answers < TCP_FW_CHECKS_PER_ROUND
    }

    /// eMule `GetFirewalled` (`Prefs.cpp:166`): menos de
    /// [`TCP_FW_ACK_THRESHOLD`] ACK de conexión de retorno y la ronda ha terminado →
    /// tras cortafuegos (firewalled); mientras la ronda siga en curso, informa la instantánea
    /// tomada a su inicio (`m_bLastFirewallState`).
    pub fn tcp_is_firewalled(&self) -> bool {
        if self.tcp_acks < TCP_FW_ACK_THRESHOLD {
            !self.tcp_recheck_active() || self.tcp_last_state
        } else {
            false
        }
    }

    /// eMule `SetRecheckIP`/`SetFirewalled` — arma una nueva ronda: pone a cero el
    /// contador de respuestas y el recuento de ACK, tomando instantánea del veredicto previo.
    pub fn start_tcp_recheck(&mut self) {
        self.tcp_answers = 0;
        self.tcp_last_state = self.tcp_acks < TCP_FW_ACK_THRESHOLD;
        self.tcp_acks = 0;
        self.next_tcp_recheck = Instant::now() + TCP_FW_RECHECK_INTERVAL;
        self.pending_tcp_ips.clear();
    }

    /// eMule `AddKadFirewallRequest` — registra `ip` como solicitada para conectar de
    /// retorno (TTL [`TCP_FW_CHECK_IP_TTL`], purgado en cada acceso).
    pub fn add_tcp_check_ip(&mut self, ip: IpAddr) {
        let now = Instant::now();
        self.pending_tcp_ips
            .retain(|(_, ts)| now.duration_since(*ts) < TCP_FW_CHECK_IP_TTL);
        self.pending_tcp_ips.push((ip, now));
    }

    /// eMule `IsKadFirewallCheckIP` — verdadero mientras esté pendiente una solicitud
    /// no caducada para `ip`.
    pub fn is_tcp_check_ip(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        self.pending_tcp_ips
            .retain(|(_, ts)| now.duration_since(*ts) < TCP_FW_CHECK_IP_TTL);
        self.pending_tcp_ips.iter().any(|(a, _)| *a == ip)
    }

    /// Llegó una respuesta `KADEMLIA_FIREWALLED_RES` desde una IP solicitada —
    /// registra la IP pública observada y avanza el contador de ronda
    /// (eMule `SetIPAddress` + `IncRecheckIP`).
    pub fn note_firewalled_res(&mut self, observed_ip: Ipv4Addr) {
        self.observed_ip = Some(observed_ip);
        self.tcp_answers = self.tcp_answers.saturating_add(1);
    }

    /// Llegó un ACK de conexión de retorno — `OP_KAD_FWTCPCHECK_ACK` (TCP, Kad ≥ 7)
    /// o `KADEMLIA_FIREWALLED_ACK_RES` (UDP, legado) — eMule `IncFirewalled`.
    pub fn inc_tcp_acks(&mut self) {
        self.tcp_acks = self.tcp_acks.saturating_add(1);
    }

    /// Puerto UDP efectivo que otros pares deberían emplear para alcanzarnos en Kad.
    pub fn effective_port(&self, intern_port: u16) -> u16 {
        if self.use_extern {
            self.extern_port.unwrap_or(intern_port)
        } else {
            intern_port
        }
    }
}

/// Estado de retransmisión de enlace de amigo (eMule `CClientList::m_pBuddy` — un único amigo
/// a la vez). El daemon puede actuar como *repetidor* (relay) para un cliente LowID:
/// `KADEMLIA_FINDBUDDY_REQ` registra una entrada pendiente, la conexión TCP del solicitante
/// (adoptada por el escuchador entrante) se convierte en el enlace activo, y los datagramas
/// `KADEMLIA_CALLBACK_REQ` / `OP_REASKCALLBACKUDP` se reenvían hacia él como
/// `OP_CALLBACK` / `OP_REASKCALLBACKTCP`.
#[derive(Debug, Default)]
pub struct BuddyRegistry {
    /// Remitentes de `FINDBUDDY_REQ` aceptados en espera de su enlace TCP de amigo.
    pending: HashMap<IpAddr, PendingBuddy>,
    /// El enlace de amigo activo (a lo sumo uno, reflejando eMule).
    pub active: Option<ActiveBuddy>,
}

/// Cuánto tiempo permanece válida una aceptación de `FINDBUDDY_REQ` esperando el enlace TCP.
pub const BUDDY_PENDING_TTL: Duration = Duration::from_secs(120);

/// Un `FINDBUDDY_REQ` que aceptamos; esperando la conexión TCP del solicitante.
#[derive(Debug, Clone)]
pub struct PendingBuddy {
    /// ID de amigo bajo el cual el solicitante pidió que lo registremos.
    pub buddy_id: [u8; 16],
    /// ID de cliente Kad del solicitante.
    pub user_id: [u8; 16],
    /// Puerto TCP anunciado por el solicitante.
    pub tcp_port: u16,
    /// Instante de expiración de este registro pendiente.
    pub expires: Instant,
}

/// Un enlace TCP de amigo activo; `tx` acepta paquetes de red eD2K completamente
/// entramados (`proto + len:u32 + opcode + payload`) para escribir en el socket del amigo.
#[derive(Debug)]
pub struct ActiveBuddy {
    /// ID de amigo registrado en el momento de `FINDBUDDY_REQ`.
    pub buddy_id: [u8; 16],
    /// ID de Kad / hash de usuario del cliente amigo.
    pub user_id: [u8; 16],
    /// Dirección de socket remota del enlace de amigo.
    pub remote_addr: SocketAddr,
    /// Sumidero de paquetes salientes para la tarea escritora del socket de amigo.
    pub tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl BuddyRegistry {
    /// Registra un candidato a amigo aceptado. Quien llame debe haber comprobado la
    /// elegibilidad (nodo verificado no tras cortafuegos, sin amigo activo).
    pub fn register_pending(
        &mut self,
        ip: IpAddr,
        buddy_id: [u8; 16],
        user_id: [u8; 16],
        tcp_port: u16,
    ) {
        self.pending.retain(|_, p| p.expires > Instant::now());
        self.pending.insert(
            ip,
            PendingBuddy {
                buddy_id,
                user_id,
                tcp_port,
                expires: Instant::now() + BUDDY_PENDING_TTL,
            },
        );
    }

    /// Consume el registro pendiente para `ip` cuando llega su enlace TCP.
    pub fn take_pending(&mut self, ip: &IpAddr) -> Option<PendingBuddy> {
        self.pending.retain(|_, p| p.expires > Instant::now());
        self.pending.remove(ip)
    }

    /// Instala el enlace de amigo activo. Devuelve false si ya hay un amigo
    /// conectado (eMule acepta un único amigo).
    pub fn set_active(&mut self, buddy: ActiveBuddy) -> bool {
        if self.active.is_some() {
            return false;
        }
        self.active = Some(buddy);
        true
    }

    /// Limpia el amigo activo si pertenece a `addr`.
    pub fn clear_active(&mut self, addr: &SocketAddr) {
        if self
            .active
            .as_ref()
            .map(|b| b.remote_addr == *addr)
            .unwrap_or(false)
        {
            self.active = None;
        }
    }

    /// Envía un paquete entramado al amigo activo. Devuelve false si no existe
    /// enlace de amigo o si su tarea escritora ya finalizó.
    pub fn send_to_buddy(&self, packet: Vec<u8>) -> bool {
        match &self.active {
            Some(b) => b.tx.send(packet).is_ok(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn pending_candidate_receives_result() {
        let mut t = FirewallTestTracker::default();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let mut rx = t.register_candidate(ip, 0xABCD);
        assert!(t.has_pending());
        assert_eq!(t.pending_verify_keys(&ip), vec![0xABCD]);
        assert!(t.notify_packet(
            ip,
            FirewallUdpResult {
                error_code: 0,
                port_tested: 49672
            }
        ));
        let res = rx.try_recv().expect("result delivered");
        assert_eq!(res.error_code, 0);
        assert_eq!(res.port_tested, 49672);
        assert!(!t.has_pending());
    }

    #[test]
    fn unsolicited_packet_is_ignored() {
        let mut t = FirewallTestTracker::default();
        let stranger = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));
        assert!(!t.notify_packet(
            stranger,
            FirewallUdpResult {
                error_code: 0,
                port_tested: 49672
            }
        ));
    }

    #[test]
    fn packet_from_wrong_ip_does_not_resolve() {
        let mut t = FirewallTestTracker::default();
        let asked = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let other = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        let mut rx = t.register_candidate(asked, 1);
        assert!(!t.notify_packet(
            other,
            FirewallUdpResult {
                error_code: 0,
                port_tested: 49672
            }
        ));
        assert!(rx.try_recv().is_err());
        assert!(t.has_pending());
        t.unregister_candidate(&asked);
        assert!(!t.has_pending());
    }

    #[test]
    fn buddy_pending_lifecycle() {
        let mut r = BuddyRegistry::default();
        let ip = IpAddr::V4(Ipv4Addr::new(9, 8, 7, 6));
        r.register_pending(ip, [1u8; 16], [2u8; 16], 49662);
        let p = r.take_pending(&ip).expect("pending registered");
        assert_eq!(p.buddy_id, [1u8; 16]);
        assert_eq!(p.tcp_port, 49662);
        assert!(r.take_pending(&ip).is_none());
    }

    #[test]
    fn buddy_single_active() {
        let mut r = BuddyRegistry::default();
        let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        let a1 = ActiveBuddy {
            buddy_id: [1u8; 16],
            user_id: [2u8; 16],
            remote_addr: "1.2.3.4:49662".parse().unwrap(),
            tx: tx1,
        };
        let a2 = ActiveBuddy {
            buddy_id: [3u8; 16],
            user_id: [4u8; 16],
            remote_addr: "5.6.7.8:49662".parse().unwrap(),
            tx: tx2,
        };
        assert!(r.set_active(a1));
        assert!(!r.set_active(a2));
        assert!(r.send_to_buddy(vec![0xC5]));
        r.clear_active(&"9.9.9.9:1".parse().unwrap());
        assert!(r.send_to_buddy(vec![0xC5]));
        r.clear_active(&"1.2.3.4:49662".parse().unwrap());
        assert!(!r.send_to_buddy(vec![0xC5]));
    }

    #[test]
    fn effective_port_prefers_intern_unless_extern_verified() {
        let mut t = FirewallTestTracker::default();
        assert_eq!(t.effective_port(49672), 49672);
        t.extern_port = Some(13370);
        assert_eq!(t.effective_port(49672), 49672);
        t.use_extern = true;
        assert_eq!(t.effective_port(49672), 13370);
    }

    #[test]
    fn tcp_recheck_round_lifecycle() {
        // eMule Prefs.cpp: `m_uRecheckip = 0` en el arranque → una ronda se activa
        // desde el primer HELLO; el veredicto permanece en el último estado conocido
        // mientras se ejecuta (`m_bLastFirewallState`, inicial true).
        let mut t = FirewallTestTracker::default();
        assert!(t.tcp_recheck_active());
        assert!(t.tcp_is_firewalled(), "boot state: firewalled until proven");

        // 4 respuestas FIREWALLED_RES sin un solo ACK → la ronda finaliza,
        // veredicto = tras cortafuegos (firewalled).
        for _ in 0..TCP_FW_CHECKS_PER_ROUND {
            t.note_firewalled_res(Ipv4Addr::new(9, 9, 9, 9));
        }
        assert!(!t.tcp_recheck_active());
        assert!(t.tcp_is_firewalled());
    }

    #[test]
    fn tcp_acks_flip_verdict_and_recheck_snapshots() {
        let mut t = FirewallTestTracker::default();
        // Dos ACK de conexión de retorno → abierto, incluso a mitad de ronda.
        t.inc_tcp_acks();
        t.inc_tcp_acks();
        assert!(!t.tcp_is_firewalled());

        // La nueva ronda toma instantánea de "abierto" — el veredicto permanece abierto mientras se ejecuta.
        t.start_tcp_recheck();
        assert!(t.tcp_recheck_active());
        assert!(!t.tcp_is_firewalled());
        assert_eq!(t.tcp_acks, 0);

        // La ronda finaliza sin suficientes ACK → vuelve a tras cortafuegos.
        for _ in 0..TCP_FW_CHECKS_PER_ROUND {
            t.note_firewalled_res(Ipv4Addr::new(9, 9, 9, 9));
        }
        assert!(t.tcp_is_firewalled());
    }

    #[test]
    fn tcp_check_ip_ttl_and_single_ask() {
        let mut t = FirewallTestTracker::default();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        assert!(!t.is_tcp_check_ip(ip));
        t.add_tcp_check_ip(ip);
        assert!(t.is_tcp_check_ip(ip));
        let other = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        assert!(!t.is_tcp_check_ip(other));

        // Una entrada caducada deja de coincidir — el par fue consultado hace >180 s.
        t.pending_tcp_ips.clear();
        t.pending_tcp_ips
            .push((ip, Instant::now() - TCP_FW_CHECK_IP_TTL - Duration::from_secs(1)));
        assert!(!t.is_tcp_check_ip(ip));
    }
}
