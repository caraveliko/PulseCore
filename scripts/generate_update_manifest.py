#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"

"""Utilidad de firma criptográfica Ed25519 y generación de manifiestos de actualización.

Permite generar el par de claves oficial de lanzamiento y crear el archivo
`manifest.json` con los hashes SHA-256 y firmas de los binarios compilados
para Windows x64, Linux x86_64 y Linux aarch64.
"""

import argparse
import datetime
import hashlib
import json
import os
import sys
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric import ed25519
from cryptography.hazmat.primitives import serialization

# Mapeo de prefijos o nombres de binarios conocidos a triples oficiales de Rust
TARGET_MAP = {
    "mule-daemon-windows-x64.exe": "x86_64-pc-windows-msvc",
    "mule-daemon-windows-x64.zip": "x86_64-pc-windows-msvc",
    "mule-daemon.exe": "x86_64-pc-windows-msvc",
    "mule-daemon-linux-x86_64": "x86_64-unknown-linux-gnu",
    "mule-daemon-linux-x86_64.tar.gz": "x86_64-unknown-linux-gnu",
    "mule-daemon-linux-aarch64": "aarch64-unknown-linux-gnu",
    "mule-daemon-linux-aarch64.tar.gz": "aarch64-unknown-linux-gnu",
}


def generate_key_pair(output_prefix: str = "release_ed25519"):
    """Genera un nuevo par de claves Ed25519 para firma de lanzamientos."""
    private_key = ed25519.Ed25519PrivateKey.generate()
    public_key = private_key.public_key()

    priv_bytes = private_key.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    pub_bytes = public_key.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )

    priv_hex = priv_bytes.hex()
    pub_hex = pub_bytes.hex()

    priv_path = f"{output_prefix}.key"
    with open(priv_path, "w", encoding="utf-8") as f:
        f.write(priv_hex + "\n")

    print(f"==> Par de claves Ed25519 generado:")
    print(f"    Clave privada guardada en: {priv_path} (¡PROTEGER DE FORMA SEGURA!)")
    print(f"    Clave pública (HEX):       {pub_hex}")
    print(f"\nCopie este valor de clave pública en 'DEFAULT_RELEASE_PUBKEY_HEX' en constants.rs:\n{pub_hex}\n")


def load_private_key(key_input: str) -> ed25519.Ed25519PrivateKey:
    """Carga la clave privada a partir de un archivo o cadena hexadecimal."""
    candidate_path = Path(key_input)
    if candidate_path.is_file():
        content = candidate_path.read_text(encoding="utf-8").strip()
    else:
        content = key_input.strip()

    raw_bytes = bytes.fromhex(content)
    return ed25519.Ed25519PrivateKey.from_private_bytes(raw_bytes)


def build_manifest(
    version: str,
    assets_dir: str,
    base_url: str,
    private_key: ed25519.Ed25519PrivateKey,
    output_path: str,
    channel: str = "stable",
    summary: str = "",
    changelog_url: str = "",
):
    """Firma cada archivo binario y ensambla el archivo manifest.json."""
    assets_path = Path(assets_dir)
    if not assets_path.is_dir():
        print(f"Error: El directorio de activos '{assets_dir}' no existe.", file=sys.stderr)
        sys.exit(1)

    targets = {}
    normalized_base_url = base_url.rstrip("/") + "/"

    for file_path in assets_path.iterdir():
        if not file_path.is_file():
            continue

        filename = file_path.name
        triple = TARGET_MAP.get(filename)
        if not triple:
            continue

        file_bytes = file_path.read_bytes()
        sha256_hash = hashlib.sha256(file_bytes).hexdigest()
        file_size = len(file_bytes)

        # Firma digital Ed25519 de los bytes exactos del archivo
        signature_bytes = private_key.sign(file_bytes)
        signature_hex = signature_bytes.hex()

        targets[triple] = {
            "url": f"{normalized_base_url}{filename}",
            "sha256": sha256_hash,
            "size": file_size,
            "signature": signature_hex,
        }
        print(f"[*] Firmado {filename} ({file_size} bytes) -> {triple}")

    if not targets:
        print(f"Aviso: No se encontraron archivos coincidentes con el mapa de plataformas en '{assets_dir}'.", file=sys.stderr)

    manifest_data = {
        "version": version,
        "releaseDate": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "channel": channel,
        "summary": summary if summary else f"Versión oficial {version} de eMuleRust",
        "changelogUrl": changelog_url if changelog_url else f"https://github.com/caraveliko/eMule-Rust/releases/tag/v{version}",
        "targets": targets,
    }

    out_file = Path(output_path)
    out_file.write_text(json.dumps(manifest_data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"\n==> Manifiesto de actualización generado con éxito en: {out_file.resolve()}")


def main():
    parser = argparse.ArgumentParser(description="Generador de manifiesto de actualización y firma Ed25519 para eMuleRust")
    parser.add_argument("--generate-keys", action="store_true", help="Generar un nuevo par de claves Ed25519")
    parser.add_argument("--key-prefix", default="release_ed25519", help="Prefijo para los archivos de clave generados")
    parser.add_argument("--private-key", help="Ruta al archivo con la clave privada o valor hexadecimal directo")
    parser.add_argument("--version", help="Versión de lanzamiento (ej. 0.3.1)")
    parser.add_argument("--assets-dir", default="target/releases", help="Directorio con los binarios compilados")
    parser.add_argument("--base-url", default="https://releases.emulerust.org/", help="URL base de descarga")
    parser.add_argument("--output", default="manifest.json", help="Ruta de destino del manifest.json")
    parser.add_argument("--channel", default="stable", help="Canal de publicación (stable, beta)")
    parser.add_argument("--summary", default="", help="Resumen descriptivo de la versión")

    args = parser.parse_args()

    if args.generate_keys:
        generate_key_pair(args.key_prefix)
        return

    if not args.version:
        parser.error("Se requiere especificar el argumento --version para generar el manifiesto.")

    if not args.private_key:
        parser.error("Se requiere especificar --private-key con la clave privada Ed25519 para firmar.")

    priv_key = load_private_key(args.private_key)
    build_manifest(
        version=args.version,
        assets_dir=args.assets_dir,
        base_url=args.base_url,
        private_key=priv_key,
        output_path=args.output,
        channel=args.channel,
        summary=args.summary,
    )


if __name__ == "__main__":
    main()
