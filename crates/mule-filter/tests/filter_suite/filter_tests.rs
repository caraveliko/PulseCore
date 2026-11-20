// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use mule_filter::{parse_ipfilter, IpFilter};
use std::io::Cursor;
use std::net::IpAddr;

#[test]
fn test_parse_standard_emule_ipfilter() {
    let content = r#"
# Comment header
// Another comment
; Semicolon comment
001.000.000.000 - 001.000.000.255 , 100 , Bad Range 1
001.000.001.000 - 001.000.001.255 , 100 , Bad Range 2
002.000.000.000 - 002.255.255.255 , 050 , Suspicious ISP
192.168.001.001 - 192.168.001.001 , 000 , Single Host
"#;

    let (ranges, report) = parse_ipfilter(Cursor::new(content.as_bytes())).unwrap();

    assert_eq!(report.comment_or_empty_lines, 4);
    assert_eq!(report.valid_entries_parsed, 4);
    assert_eq!(report.invalid_lines, 0);

    // Los rangos 1 y 2 son adyacentes con el mismo nivel (100) -> fusionados en 001.000.000.000 - 001.000.001.255
    assert_eq!(report.coalesced_count, 1);
    assert_eq!(report.final_range_count, 3);
    assert_eq!(ranges.len(), 3);
}

#[test]
fn test_filter_binary_search_lookup() {
    let filter = IpFilter::new();
    let content = r#"
010.000.000.000 - 010.000.000.100 , 090 , Blocked Range A
020.000.000.000 - 020.000.000.200 , 110 , Blocked Range B
100.064.000.000 - 100.064.000.255 , 120 , CGNAT Block
"#;
    filter.load_from_str(content).unwrap();

    // Comprobar coincidencia dentro del Rango A
    let hit = filter.check_ip("10.0.0.50".parse().unwrap());
    assert!(hit.is_some());
    let r = hit.unwrap();
    assert_eq!(r.description.as_ref(), "Blocked Range A");
    assert_eq!(r.level, 90);

    // Comprobar coincidencia en el límite del Rango B
    assert!(filter.check_ip("20.0.0.0".parse().unwrap()).is_some());
    assert!(filter.check_ip("20.0.0.200".parse().unwrap()).is_some());

    // Comprobar no coincidencia justo fuera del Rango B
    assert!(filter.check_ip("20.0.0.201".parse().unwrap()).is_none());

    // Comprobar coincidencia con IPv6 asignada a IPv4
    let v6_mapped: IpAddr = "::ffff:10.0.0.50".parse().unwrap();
    let hit_v6 = filter.check_ip(v6_mapped);
    assert!(hit_v6.is_some());

    // Verificar estadísticas
    let stats = filter.stats();
    assert!(stats.enabled);
    assert_eq!(stats.range_count, 3);
    assert!(stats.hits_count >= 3);
}

#[test]
fn test_filter_disabled_allows_all() {
    let filter = IpFilter::new();
    let content = "010.000.000.000 - 010.000.000.255 , 100 , Test\n";
    filter.load_from_str(content).unwrap();

    assert!(filter.check_ip("10.0.0.5".parse().unwrap()).is_some());
    filter.set_enabled(false);
    assert!(filter.check_ip("10.0.0.5".parse().unwrap()).is_none());
    filter.set_enabled(true);
    assert!(filter.check_ip("10.0.0.5".parse().unwrap()).is_some());
}

#[test]
fn test_defensive_parser_bounds() {
    // Superar el límite de longitud de línea
    let too_long = "1.1.1.1 - 1.1.1.2 , 0 , ".to_string() + &"A".repeat(1024);
    let (_, report) = parse_ipfilter(Cursor::new(too_long.as_bytes())).unwrap();
    assert_eq!(report.invalid_lines, 1);
    assert_eq!(report.final_range_count, 0);
}
