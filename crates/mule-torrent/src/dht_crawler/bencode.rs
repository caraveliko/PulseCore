// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Códec Bencode acotado en memoria y profundidad para tramas KRPC (BEP 5/51) y `ut_metadata` (BEP 9).

use std::collections::BTreeMap;

/// Profundidad máxima de anidamiento permitida al decodificar bencode (anti-desbordamiento de pila).
pub const MAX_BENCODE_DEPTH: usize = 32;
/// Tamaño máximo de un mensaje bencode aceptado por el analizador (1 MiB).
pub const MAX_BENCODE_BYTES: usize = 1_048_576;

/// Error de decodificación bencode acotada.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BencodeError {
    #[error("entrada bencode supera el límite máximo de {MAX_BENCODE_BYTES} bytes")]
    TooLarge,
    #[error("profundidad bencode supera el límite de {MAX_BENCODE_DEPTH}")]
    TooDeep,
    #[error("fin de entrada inesperado en trama bencode")]
    UnexpectedEof,
    #[error("sintaxis bencode inválida en desplazamiento {0}")]
    InvalidSyntax(usize),
}

/// Valor bencode en memoria con orden canónico en diccionarios (`BTreeMap`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BValue>),
    Dict(BTreeMap<Vec<u8>, BValue>),
}

impl BValue {
    /// Construye un valor de bytes a partir de una cadena UTF-8.
    pub fn str_bytes(s: &str) -> Self {
        Self::Bytes(s.as_bytes().to_vec())
    }

    /// Devuelve el diccionario subyacente si este valor es un `Dict`.
    pub fn as_dict(&self) -> Option<&BTreeMap<Vec<u8>, BValue>> {
        match self {
            Self::Dict(map) => Some(map),
            _ => None,
        }
    }

    /// Devuelve la lista subyacente si este valor es una `List`.
    pub fn as_list(&self) -> Option<&[BValue]> {
        match self {
            Self::List(list) => Some(list),
            _ => None,
        }
    }

    /// Devuelve el slice de bytes si este valor es `Bytes`.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(b) => Some(b.as_slice()),
            _ => None,
        }
    }

    /// Devuelve el valor como texto UTF-8 con pérdida si es `Bytes`.
    pub fn as_str_lossy(&self) -> Option<String> {
        self.as_bytes()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// Devuelve el entero si este valor es `Int`.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// Obtiene una clave de un diccionario por nombre ASCII/UTF-8.
    pub fn dict_get(&self, key: &[u8]) -> Option<&BValue> {
        self.as_dict().and_then(|d| d.get(key))
    }

    /// Serializa el valor en formato bencode canónico.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(n) => {
                out.push(b'i');
                out.extend_from_slice(n.to_string().as_bytes());
                out.push(b'e');
            }
            Self::Bytes(b) => {
                out.extend_from_slice(b.len().to_string().as_bytes());
                out.push(b':');
                out.extend_from_slice(b);
            }
            Self::List(items) => {
                out.push(b'l');
                for item in items {
                    item.encode_into(out);
                }
                out.push(b'e');
            }
            Self::Dict(map) => {
                out.push(b'd');
                for (k, v) in map {
                    out.extend_from_slice(k.len().to_string().as_bytes());
                    out.push(b':');
                    out.extend_from_slice(k);
                    v.encode_into(out);
                }
                out.push(b'e');
            }
        }
    }
}

/// Decodifica el primer valor bencode de `input` y devuelve `(valor, bytes_consumidos)`.
pub fn decode_prefix(input: &[u8]) -> Result<(BValue, usize), BencodeError> {
    if input.len() > MAX_BENCODE_BYTES {
        return Err(BencodeError::TooLarge);
    }
    let mut pos = 0usize;
    let val = parse_value(input, &mut pos, 0)?;
    Ok((val, pos))
}

/// Decodifica un mensaje bencode completo (tolerando relleno o datos posteriores si se usa `decode_prefix`).
pub fn decode(input: &[u8]) -> Result<BValue, BencodeError> {
    let (val, _) = decode_prefix(input)?;
    Ok(val)
}

fn parse_value(input: &[u8], pos: &mut usize, depth: usize) -> Result<BValue, BencodeError> {
    if depth > MAX_BENCODE_DEPTH {
        return Err(BencodeError::TooDeep);
    }
    let byte = *input.get(*pos).ok_or(BencodeError::UnexpectedEof)?;
    match byte {
        b'i' => {
            *pos += 1;
            let start = *pos;
            let end_rel = input[start..]
                .iter()
                .position(|&b| b == b'e')
                .ok_or(BencodeError::UnexpectedEof)?;
            let num_slice = &input[start..start + end_rel];
            let num_str = std::str::from_utf8(num_slice)
                .map_err(|_| BencodeError::InvalidSyntax(start))?;
            let n = num_str
                .parse::<i64>()
                .map_err(|_| BencodeError::InvalidSyntax(start))?;
            *pos = start + end_rel + 1;
            Ok(BValue::Int(n))
        }
        b'l' => {
            *pos += 1;
            let mut list = Vec::new();
            loop {
                let next = *input.get(*pos).ok_or(BencodeError::UnexpectedEof)?;
                if next == b'e' {
                    *pos += 1;
                    return Ok(BValue::List(list));
                }
                list.push(parse_value(input, pos, depth + 1)?);
            }
        }
        b'd' => {
            *pos += 1;
            let mut map = BTreeMap::new();
            loop {
                let next = *input.get(*pos).ok_or(BencodeError::UnexpectedEof)?;
                if next == b'e' {
                    *pos += 1;
                    return Ok(BValue::Dict(map));
                }
                let key = parse_bytes(input, pos)?;
                let val = parse_value(input, pos, depth + 1)?;
                map.insert(key, val);
            }
        }
        b'0'..=b'9' => Ok(BValue::Bytes(parse_bytes(input, pos)?)),
        _ => Err(BencodeError::InvalidSyntax(*pos)),
    }
}

fn parse_bytes(input: &[u8], pos: &mut usize) -> Result<Vec<u8>, BencodeError> {
    let start = *pos;
    let colon_rel = input[start..]
        .iter()
        .position(|&b| b == b':')
        .ok_or(BencodeError::UnexpectedEof)?;
    let len_str = std::str::from_utf8(&input[start..start + colon_rel])
        .map_err(|_| BencodeError::InvalidSyntax(start))?;
    let len = len_str
        .parse::<usize>()
        .map_err(|_| BencodeError::InvalidSyntax(start))?;
    let data_start = start + colon_rel + 1;
    let data_end = data_start
        .checked_add(len)
        .ok_or(BencodeError::InvalidSyntax(start))?;
    if data_end > input.len() {
        return Err(BencodeError::UnexpectedEof);
    }
    *pos = data_end;
    Ok(input[data_start..data_end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_bencode_dict_and_prefix() {
        let mut dict = BTreeMap::new();
        dict.insert(b"m".to_vec(), BValue::Int(42));
        dict.insert(b"s".to_vec(), BValue::str_bytes("hola"));
        dict.insert(
            b"l".to_vec(),
            BValue::List(vec![BValue::Int(-5), BValue::str_bytes("x")]),
        );
        let original = BValue::Dict(dict);
        let mut raw = original.encode();
        raw.extend_from_slice(b"PAYLOAD");

        let (decoded, consumed) = decode_prefix(&raw).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(&raw[consumed..], b"PAYLOAD");
    }

    #[test]
    fn rejects_excessive_nesting_depth() {
        let deep = vec![b'l'; MAX_BENCODE_DEPTH + 4];
        assert_eq!(decode(&deep), Err(BencodeError::TooDeep));
    }
}
