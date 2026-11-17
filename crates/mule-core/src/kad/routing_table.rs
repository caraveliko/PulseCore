// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::contact::KadContact;
use super::kad_id::KadId;
use std::collections::VecDeque;

/// Capacidad de la cubeta — `K` de eMule (`kademlia/Defines.h:42` = 10; eMule usa un
/// árbol de zonas binario de contenedores de tamaño K en lugar de cubetas planas, pero la
/// capacidad por contenedor es la constante relevante para el cable).
pub const K_BUCKET_SIZE: usize = 10;
/// Capacidad de la caché de reemplazo por cubeta.
pub const REPLACEMENT_CACHE_SIZE: usize = 10;
/// Número total de cubetas K para un Kad ID de 128 bits.
pub const NUM_BUCKETS: usize = 128;
/// Número máximo de contactos permitidos de la misma subred IPv4 /24 por cubeta (anti-Sybil).
pub const MAX_CONTACTS_PER_SUBNET_24: usize = 2;

#[derive(Debug, Clone)]
pub struct KBucket {
    pub contacts: VecDeque<KadContact>,
    pub replacements: VecDeque<KadContact>,
}

impl Default for KBucket {
    fn default() -> Self {
        Self {
            contacts: VecDeque::with_capacity(K_BUCKET_SIZE),
            replacements: VecDeque::with_capacity(REPLACEMENT_CACHE_SIZE),
        }
    }
}

impl KBucket {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_full(&self) -> bool {
        self.contacts.len() >= K_BUCKET_SIZE
    }

    pub fn get(&self, kad_id: &KadId) -> Option<&KadContact> {
        self.contacts.iter().find(|c| &c.kad_id == kad_id)
    }

    pub fn insert_or_update(&mut self, contact: KadContact) -> Option<KadContact> {
        if let Some(pos) = self
            .contacts
            .iter()
            .position(|c| c.kad_id == contact.kad_id)
        {
            // Ya presente: mover al final (visto más recientemente) y actualizar
            if let Some(mut existing) = self.contacts.remove(pos) {
                existing.ip = contact.ip;
                existing.udp_port = contact.udp_port;
                existing.tcp_port = contact.tcp_port;
                existing.version = contact.version;
                existing.last_seen_unix_ms = contact.last_seen_unix_ms;
                if contact.verified {
                    existing.verified = true;
                }
                self.contacts.push_back(existing);
            }
            None
        } else if self.contacts.len() < K_BUCKET_SIZE {
            // Protección anti-Sybil: limitar el número máximo de contactos de la misma subred /24 en una cubeta
            if !contact.ip.is_loopback() {
                let subnet_count = self
                    .contacts
                    .iter()
                    .filter(|c| c.ip.octets()[..3] == contact.ip.octets()[..3])
                    .count();
                if subnet_count >= MAX_CONTACTS_PER_SUBNET_24 {
                    // No admitir más de la misma subred /24 en la cubeta activa
                    return None;
                }
            }
            // No llena: insertar al final
            self.contacts.push_back(contact);
            None
        } else {
            // Llena: añadir a la caché de reemplazo si no está ya presente
            if let Some(pos) = self
                .replacements
                .iter()
                .position(|c| c.kad_id == contact.kad_id)
            {
                self.replacements.remove(pos);
            } else if self.replacements.len() >= REPLACEMENT_CACHE_SIZE {
                self.replacements.pop_front();
            }
            self.replacements.push_back(contact.clone());
            // Devolver candidato para hacer ping/desalojar (la cabeza de la cubeta)
            self.contacts.front().cloned()
        }
    }

    pub fn remove(&mut self, kad_id: &KadId) -> Option<KadContact> {
        if let Some(pos) = self.contacts.iter().position(|c| &c.kad_id == kad_id) {
            let removed = self.contacts.remove(pos);
            // Promocionar candidato desde la caché de reemplazo
            if let Some(replacement) = self.replacements.pop_back() {
                self.contacts.push_back(replacement);
            }
            removed
        } else {
            if let Some(pos) = self.replacements.iter().position(|c| &c.kad_id == kad_id) {
                return self.replacements.remove(pos);
            }
            None
        }
    }
}

/// Tabla de enrutamiento Kademlia que contiene 128 cubetas.
#[derive(Debug, Clone)]
pub struct KBucketTable {
    local_id: KadId,
    buckets: Vec<KBucket>,
}

impl KBucketTable {
    pub fn new(local_id: KadId) -> Self {
        let mut buckets = Vec::with_capacity(NUM_BUCKETS);
        for _ in 0..NUM_BUCKETS {
            buckets.push(KBucket::new());
        }
        Self { local_id, buckets }
    }

    pub fn local_id(&self) -> &KadId {
        &self.local_id
    }

    pub fn bucket_index_for(&self, kad_id: &KadId) -> Option<usize> {
        self.local_id.bucket_index_for(kad_id)
    }

    pub fn insert_or_update(&mut self, contact: KadContact) -> Option<KadContact> {
        if contact.kad_id == self.local_id {
            return None; // No insertarse a sí mismo
        }
        if let Some(idx) = self.bucket_index_for(&contact.kad_id) {
            self.buckets[idx].insert_or_update(contact)
        } else {
            None
        }
    }

    pub fn remove(&mut self, kad_id: &KadId) -> Option<KadContact> {
        if let Some(idx) = self.bucket_index_for(kad_id) {
            self.buckets[idx].remove(kad_id)
        } else {
            None
        }
    }

    pub fn get(&self, kad_id: &KadId) -> Option<&KadContact> {
        if let Some(idx) = self.bucket_index_for(kad_id) {
            self.buckets[idx].get(kad_id)
        } else {
            None
        }
    }

    /// Registra una consulta fallida contra un contacto. Como en eMule (`CContact::Update`
    /// / `Expire`), un contacto que alcanza 2 fallos consecutivos es desalojado
    /// de la cubeta (y de su ranura de reemplazo). Devuelve `true` si el
    /// contacto fue desalojado.
    pub fn mark_failed(&mut self, kad_id: &KadId) -> bool {
        let Some(idx) = self.bucket_index_for(kad_id) else {
            return false;
        };
        let bucket = &mut self.buckets[idx];
        let pos = bucket.contacts.iter().position(|c| &c.kad_id == kad_id);
        match pos {
            Some(pos) => {
                let fails = {
                    let c = &mut bucket.contacts[pos];
                    c.mark_failed();
                    c.fail_count
                };
                if fails >= 2 {
                    self.remove(kad_id);
                    true
                } else {
                    false
                }
            }
            None => {
                if let Some(rpos) = bucket
                    .replacements
                    .iter()
                    .position(|c| &c.kad_id == kad_id)
                {
                    bucket.replacements.remove(rpos);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Registra una respuesta exitosa de un contacto: actualiza `last_seen`,
    /// reinicia `fail_count` y lo marca como verificado (`KadContact::mark_seen`).
    pub fn mark_seen(&mut self, kad_id: &KadId, now_ms: u64) {
        let Some(idx) = self.bucket_index_for(kad_id) else {
            return;
        };
        let bucket = &mut self.buckets[idx];
        for c in bucket.contacts.iter_mut().chain(bucket.replacements.iter_mut()) {
            if &c.kad_id == kad_id {
                c.mark_seen(now_ms);
                return;
            }
        }
    }

    pub fn total_contacts(&self) -> usize {
        self.buckets.iter().map(|b| b.contacts.len()).sum()
    }

    pub fn total_replacements(&self) -> usize {
        self.buckets.iter().map(|b| b.replacements.len()).sum()
    }

    /// Recopila todos los contactos conocidos (entradas de cubeta + caché de reemplazo).
    fn collect_all(&self) -> Vec<KadContact> {
        let mut all_contacts = Vec::new();
        for b in &self.buckets {
            all_contacts.extend(b.contacts.iter().cloned());
            all_contacts.extend(b.replacements.iter().cloned());
        }
        all_contacts
    }

    /// Encuentra hasta `count` contactos más cercanos a la clave objetivo dada.
    pub fn find_closest(&self, target: &KadId, count: usize) -> Vec<KadContact> {
        let mut all_contacts = self.collect_all();
        all_contacts.sort_by(|a, b| {
            let dist_a = a.kad_id.xor_distance(target);
            let dist_b = b.kad_id.xor_distance(target);
            dist_a.cmp(&dist_b)
        });
        all_contacts.truncate(count);
        all_contacts
    }

    /// Encuentra hasta `count` contactos más cercanos, priorizando los contactos verificados.
    pub fn find_closest_verified(&self, target: &KadId, count: usize) -> Vec<KadContact> {
        let mut all_contacts = self.collect_all();
        all_contacts.sort_by(|a, b| match (a.verified, b.verified) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let dist_a = a.kad_id.xor_distance(target);
                let dist_b = b.kad_id.xor_distance(target);
                dist_a.cmp(&dist_b)
            }
        });
        all_contacts.truncate(count);
        all_contacts
    }

    /// Encuentra hasta `count` contactos *verificados* más cercanos a `target`.
    ///
    /// Refleja `GetClosestTo(..., bOnlyVerified=true)` de eMule utilizado para responder
    /// a `KADEMLIA2_REQ`/`BOOTSTRAP_REQ`: solo los contactos cuya propiedad de IP fue
    /// demostrada (respondieron a una de nuestras solicitudes, o completaron el
    /// protocolo de enlace `HELLO_RES_ACK`) pueden entregarse a otros nodos — devolver
    /// contactos no verificados propagaría entradas potencialmente falseadas.
    pub fn find_verified_closest(&self, target: &KadId, count: usize) -> Vec<KadContact> {
        let mut verified: Vec<KadContact> = self
            .collect_all()
            .into_iter()
            .filter(|c| c.verified)
            .collect();
        verified.sort_by(|a, b| {
            let dist_a = a.kad_id.xor_distance(target);
            let dist_b = b.kad_id.xor_distance(target);
            dist_a.cmp(&dist_b)
        });
        verified.truncate(count);
        verified
    }

    /// Devuelve hasta `max` contactos verificados distribuidos a lo largo del espacio de claves
    /// (por turnos (round-robin) sobre cubetas no vacías), reflejando `GetBootstrapContacts`
    /// de eMule: una respuesta de bootstrap debe proporcionar al solicitante una
    /// muestra diversa de la DHT, no un grupo de contactos con el mismo prefijo.
    pub fn spread_verified(&self, max: usize) -> Vec<KadContact> {
        let mut out = Vec::new();
        let mut round = 0;
        loop {
            let before = out.len();
            for b in &self.buckets {
                if let Some(c) = b.contacts.iter().filter(|c| c.verified).nth(round) {
                    out.push(c.clone());
                    if out.len() >= max {
                        return out;
                    }
                }
            }
            if out.len() == before {
                break;
            }
            round += 1;
        }
        out
    }

    /// Devuelve una instantánea de las cubetas no vacías y sus recuentos de contactos.
    pub fn bucket_distribution(&self) -> Vec<(usize, usize)> {
        self.buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.contacts.is_empty())
            .map(|(i, b)| (i, b.contacts.len()))
            .collect()
    }
}
