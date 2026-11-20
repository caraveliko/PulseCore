#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Detector de duplicación para el workspace Rust de eMuleRust (Pulse Core).

Tres ángulos de análisis, todos 100 % sin conexión:
  A. Símbolos homónimos (fn/struct/enum/trait/const definidos en >= 2 archivos)
  B. Cuerpos de funciones duplicados (hash del cuerpo normalizado: idéntico o
     idéntico tras renombrar identificadores)
  C. Ventanas de clones (bloques de >= WINDOW líneas limpias idénticas)

Uso: python3 scripts/find_duplicates.py [--window N] [--root crates]
Salida: informe de texto en stdout (redirigir a un archivo para archivarlo).
"""

import argparse
import hashlib
import re
import sys
from collections import defaultdict
from pathlib import Path

KEYWORDS = {
    "as", "async", "await", "break", "const", "continue", "crate", "dyn",
    "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "macro_rules", "Box", "Some", "None",
    "Ok", "Err", "Vec", "String", "Option", "Result", "Copy", "Clone",
}

SYMBOL_RE = re.compile(
    r"(?:^|\s)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(fn|struct|enum|trait|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)"
)

ATTR_LINE_RE = re.compile(r"^\s*#?\s*!\s*\[.*$")


def clean_source(text: str) -> str:
    """Sustituye comentarios y literales por espacios conservando la estructura sintáctica."""
    out = list(text)
    i, n = 0, len(text)
    state = "code"
    block_depth = 0
    raw_hashes = 0
    while i < n:
        c = text[i]
        if state == "code":
            if c == "/" and i + 1 < n and text[i + 1] == "/":
                j = text.find("\n", i)
                j = n if j == -1 else j
                for k in range(i, j):
                    out[k] = " "
                i = j
            elif c == "/" and i + 1 < n and text[i + 1] == "*":
                state = "block"
                block_depth = 1
                out[i] = out[i + 1] = " "
                i += 2
            elif c == "r" and i + 1 < n and text[i + 1] in ('"', "#"):
                j = i + 1
                hashes = 0
                while j < n and text[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and text[j] == '"':
                    state = "raw"
                    raw_hashes = hashes
                    for k in range(i, j + 1):
                        out[k] = " "
                    i = j + 1
                else:
                    i += 1
            elif c == '"':
                state = "str"
                out[i] = " "
                i += 1
            elif c == "'":
                # Literal de carácter 'x' o '\n' — distinguir del tiempo de vida 'a
                if i + 2 < n and text[i + 2] == "'":
                    out[i] = out[i + 1] = out[i + 2] = " "
                    i += 3
                elif i + 3 < n and text[i + 1] == "\\" and text[i + 3] == "'":
                    for k in range(i, i + 4):
                        out[k] = " "
                    i += 4
                else:
                    i += 1  # tiempo de vida (lifetime)
            else:
                i += 1
        elif state == "block":
            if c == "/" and i + 1 < n and text[i + 1] == "*":
                block_depth += 1
                out[i] = out[i + 1] = " "
                i += 2
            elif c == "*" and i + 1 < n and text[i + 1] == "/":
                block_depth -= 1
                out[i] = out[i + 1] = " "
                i += 2
                if block_depth == 0:
                    state = "code"
            else:
                if c != "\n":
                    out[i] = " "
                i += 1
        elif state == "str":
            if c == "\\" and i + 1 < n:
                out[i] = out[i + 1] = " "
                i += 2
            elif c == '"':
                out[i] = " "
                state = "code"
                i += 1
            else:
                if c != "\n":
                    out[i] = " "
                i += 1
        elif state == "raw":
            if c == '"':
                j = i + 1
                ok = True
                for h in range(raw_hashes):
                    if j + h >= n or text[j + h] != "#":
                        ok = False
                        break
                if ok:
                    for k in range(i, j + raw_hashes):
                        out[k] = " "
                    i = j + raw_hashes
                    state = "code"
                else:
                    if c != "\n":
                        out[i] = " "
                    i += 1
            else:
                if c != "\n":
                    out[i] = " "
                i += 1
    return "".join(out)


def find_matching_brace(cleaned: str, open_idx: int) -> int:
    depth = 0
    i = open_idx
    n = len(cleaned)
    while i < n:
        c = cleaned[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def extract_functions(cleaned: str):
    """Devuelve [(nombre, texto_cuerpo)] para cada fn con cuerpo (excluye declaraciones de trait)."""
    fns = []
    for m in re.finditer(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", cleaned):
        # Busca el final de la firma: primer '{' o ';' a profundidad 0
        i = m.end()
        depth_paren = depth_brack = depth_angle = 0
        brace_idx = -1
        while i < len(cleaned):
            c = cleaned[i]
            if c == "(":
                depth_paren += 1
            elif c == ")":
                depth_paren -= 1
            elif c == "[":
                depth_brack += 1
            elif c == "]":
                depth_brack -= 1
            elif c == "<":
                depth_angle += 1
            elif c == ">":
                depth_angle = max(0, depth_angle - 1)
            elif c == "{" and depth_paren == 0 and depth_brack == 0:
                brace_idx = i
                break
            elif c == ";" and depth_paren == 0 and depth_brack == 0:
                break  # declaración sin cuerpo
            i += 1
        if brace_idx == -1:
            continue
        end = find_matching_brace(cleaned, brace_idx)
        if end == -1:
            continue
        fns.append((m.group(1), cleaned[brace_idx:end + 1]))
    return fns


def norm_body(body: str, rename_idents: bool) -> str:
    toks = re.findall(r"[A-Za-z_][A-Za-z0-9_]*|[^A-Za-z_0-9\s]", body)
    if rename_idents:
        toks = [t if (not re.match(r"^[A-Za-z_]", t) or t in KEYWORDS) else "ID"
                for t in toks]
    return hashlib.sha1(" ".join(toks).encode()).hexdigest()


def collect_rs_files(root: Path):
    files = []
    for p in sorted(root.rglob("*.rs")):
        if "target" in p.parts:
            continue
        files.append(p)
    return files


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default="crates")
    ap.add_argument("--window", type=int, default=8,
                    help="tamaño de ventana en líneas limpias (por defecto 8)")
    ap.add_argument("--min-body-tokens", type=int, default=25,
                    help="tamaño mínimo de cuerpo de función (en tokens) para el hash")
    args = ap.parse_args()

    root = Path(args.root)
    files = collect_rs_files(root)
    total_lines = 0
    symbols = defaultdict(list)           # nombre -> [(archivo, línea, tipo)]
    fn_exact = defaultdict(list)          # hash -> [(archivo, nombre, línea)]
    fn_renamed = defaultdict(list)
    windows = defaultdict(list)           # hash -> [(archivo, índice_inicio)]
    clean_lines_of = {}                   # archivo -> [(núm_línea_orig, línea_limpia)]

    for p in files:
        text = p.read_text(encoding="utf-8", errors="replace")
        total_lines += text.count("\n") + 1
        cleaned = clean_source(text)
        rel = str(p).replace("\\", "/")
        clines = []
        for idx, line in enumerate(cleaned.splitlines(), start=1):
            s = " ".join(line.split())
            clines.append((idx, s))
        clean_lines_of[rel] = clines
        nonempty = [(n_, s) for n_, s in clines if s and s not in ("{", "}", "};", "{;", ")")]
        for i in range(0, max(0, len(nonempty) - args.window + 1)):
            chunk = nonempty[i:i + args.window]
            h = hashlib.sha1("\n".join(s for _, s in chunk).encode()).hexdigest()
            windows[h].append((rel, chunk[0][0]))

        for m in SYMBOL_RE.finditer(cleaned):
            kind, name = m.group(1), m.group(2)
            line = cleaned.count("\n", 0, m.start()) + 1
            symbols[name].append((rel, line, kind))

        for name, body in extract_functions(cleaned):
            ntok = len(re.findall(r"[A-Za-z_][A-Za-z0-9_]*|\S", body))
            if ntok < args.min_body_tokens:
                continue
            line = cleaned.count("\n", 0, cleaned.find(body)) + 1
            fn_exact[norm_body(body, False)].append((rel, name, line))
            fn_renamed[norm_body(body, True)].append((rel, name, line))

    print(f"== Baseline ==  {len(files)} archivos, {total_lines} líneas\n")

    print("== A. Símbolos homónimos (mismo nombre, >=2 archivos) ==")
    shown = 0
    for name in sorted(symbols, key=lambda k: -len({f for f, _, _ in symbols[k]})):
        locs = symbols[name]
        fset = sorted({f for f, _, _ in locs})
        if len(fset) < 2:
            continue
        if name in ("new", "default", "fmt", "drop", "from", "clone",
                    "as_str", "len", "is_empty", "fmt", "eq", "hash",
                    "serialize", "deserialize", "borrow", "deref", "into"):
            continue  # impls de traits estándar — ruido
        kinds0 = {k for _, _, k in locs}
        if kinds0 == {"mod"} and name in ("tests", "types"):
            continue  # idioma Rust: módulos tests/types por archivo — legítimo
        shown += 1
        kinds = {k for _, _, k in locs}
        print(f"  {name}  x{len(locs)} ({'/'.join(sorted(kinds))})")
        for f, ln, k in locs:
            print(f"      {f}:{ln} [{k}]")
    if not shown:
        print("  (ninguno)")

    print("\n== B1. Cuerpos de funciones estrictamente idénticos ==")
    shown = 0
    for h, locs in sorted(fn_exact.items(), key=lambda kv: -len(kv[1])):
        uniq = {(f, l) for f, _, l in locs}
        if len(uniq) < 2:
            continue
        shown += 1
        names = {n for _, n, _ in locs}
        print(f"  [{'/'.join(sorted(names))}] x{len(uniq)}")
        for f, n, l in locs:
            print(f"      {f}:{l}  fn {n}")
    if not shown:
        print("  (ninguno)")

    print("\n== B2. Cuerpos idénticos tras renombrar identificadores ==")
    shown = 0
    for h, locs in sorted(fn_renamed.items(), key=lambda kv: -len(kv[1])):
        uniq = {(f, l) for f, _, l in locs}
        if len(uniq) < 2:
            continue
        names = {n for _, n, _ in locs}
        if len(names) < 2 and len({f for f, _, _ in locs}) < 2:
            continue
        shown += 1
        print(f"  [{'/'.join(sorted(names))}] x{len(uniq)}")
        for f, n, l in locs:
            print(f"      {f}:{l}  fn {n}")
    if not shown:
        print("  (ninguno)")

    print(f"\n== C. Bloques clonados entre archivos (>={args.window} líneas limpias) ==")
    # Fusiona ventanas deslizantes en bloques: dos ventanas idénticas en
    # (f1,l1) y (f2,l2) pertenecen al mismo clon si mantienen el desplazamiento l2-l1.
    diagonals = defaultdict(set)  # (f1,f2,delta) -> conjunto de l1
    for occs in windows.values():
        locs = sorted(set(occs))
        for i in range(len(locs)):
            for j in range(i + 1, len(locs)):
                f1, l1 = locs[i]
                f2, l2 = locs[j]
                if f1 == f2:
                    continue
                diagonals[(f1, f2, l2 - l1)].add(l1)

    blocks = []
    for (f1, f2, delta), starts in diagonals.items():
        starts = sorted(starts)
        i = 0
        while i < len(starts):
            j = i
            while j + 1 < len(starts) and starts[j + 1] <= starts[j] + 1:
                j += 1
            nlines = (starts[j] - starts[i]) + args.window
            if nlines >= args.window + 2:  # al menos un bloque real fusionado
                blocks.append((nlines, f1, starts[i], f2, starts[i] + delta))
            i = j + 1

    blocks.sort(key=lambda b: -b[0])
    shown = 0
    seen = set()
    for nlines, f1, l1, f2, l2 in blocks:
        key = (f1, l1, f2, l2)
        if key in seen:
            continue
        seen.add(key)
        shown += 1
        print(f"  ~{nlines} líneas : {f1}:{l1}  <->  {f2}:{l2}")
        if shown >= 200:
            print("  ... (truncado)")
            break
    if not shown:
        print("  (ninguno)")


if __name__ == "__main__":
    main()
