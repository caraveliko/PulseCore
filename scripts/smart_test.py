#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Corredor inteligente de pruebas y validación para Pulse Core (eMuleRust).

Calcula los crates y suites de prueba realmente afectados mediante `git` y el
grafo de dependencias de `cargo metadata`, evitando compilar y enlazar binarios
no relacionados en cada iteración.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST_PATH = ROOT / "scripts" / "dup_allowlist.txt"
FIND_DUPS_PATH = ROOT / "scripts" / "find_duplicates.py"
WEB_INDEX_PATH = ROOT / "web" / "index.html"

GLOBAL_TRIGGER_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/config.toml",
    "deny.toml",
}


@dataclass
class CrateInfo:
    """Metadatos mínimos de un paquete del workspace."""

    name: str
    rel_dir: str
    deps: Set[str] = field(default_factory=set)
    test_targets: Dict[str, str] = field(default_factory=dict)


@dataclass
class TestInvocation:
    """Objetivo específico de prueba dentro de un crate."""

    crate: str
    test_bin: Optional[str] = None
    filter_expr: Optional[str] = None


def run_capture(cmd: List[str]) -> Tuple[int, str, str]:
    """Ejecuta un comando capturando salida estándar y error."""
    proc = subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    return proc.returncode, proc.stdout, proc.stderr


def run_streaming(cmd: List[str], dry_run: bool) -> Tuple[int, float, str]:
    """Ejecuta un comando mostrando su salida en vivo y midiendo el tiempo."""
    cmd_str = " ".join(cmd)
    print(f"\n==> Ejecutando: {cmd_str}", flush=True)
    if dry_run:
        return 0, 0.0, ""

    t0 = time.monotonic()
    proc = subprocess.Popen(
        cmd,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )
    lines: List[str] = []
    assert proc.stdout is not None
    for line in proc.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        lines.append(line)
    rc = proc.wait()
    elapsed = time.monotonic() - t0
    return rc, elapsed, "".join(lines)


def load_workspace_graph() -> Tuple[Dict[str, CrateInfo], Dict[str, Set[str]]]:
    """Obtiene el grafo de crates y dependencias inversas mediante cargo metadata."""
    rc, out, err = run_capture(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"]
    )
    if rc != 0:
        raise RuntimeError(f"Fallo ejecutando cargo metadata: {err.strip()}")

    data = json.loads(out)
    packages = data.get("packages", [])
    ws_ids = set(data.get("workspace_members", []))

    crates: Dict[str, CrateInfo] = {}
    ws_names: Set[str] = set()
    for pkg in packages:
        if pkg.get("id") in ws_ids:
            ws_names.add(pkg["name"])

    for pkg in packages:
        if pkg.get("id") not in ws_ids:
            continue
        name = pkg["name"]
        manifest_path = Path(pkg["manifest_path"]).resolve()
        rel_dir = manifest_path.parent.relative_to(ROOT.resolve()).as_posix()

        deps: Set[str] = set()
        for dep in pkg.get("dependencies", []):
            dep_name = dep.get("name")
            if dep_name in ws_names and dep_name != name:
                deps.add(dep_name)

        test_targets: Dict[str, str] = {}
        for tgt in pkg.get("targets", []):
            if "test" in tgt.get("kind", []):
                src_rel = Path(tgt["src_path"]).resolve().relative_to(ROOT.resolve()).as_posix()
                test_targets[tgt["name"]] = src_rel

        crates[name] = CrateInfo(
            name=name,
            rel_dir=rel_dir,
            deps=deps,
            test_targets=test_targets,
        )

    reverse_deps: Dict[str, Set[str]] = {c: set() for c in crates}
    for cname, info in crates.items():
        for dep in info.deps:
            if dep in reverse_deps:
                reverse_deps[dep].add(cname)

    return crates, reverse_deps


def transitive_dependents(
    seeds: Set[str], reverse_deps: Dict[str, Set[str]]
) -> Set[str]:
    """Calcula la clausura transitiva de crates que dependen de `seeds`."""
    visited = set(seeds)
    queue = list(seeds)
    while queue:
        cur = queue.pop()
        for nxt in reverse_deps.get(cur, set()):
            if nxt not in visited:
                visited.add(nxt)
                queue.append(nxt)
    return visited


def collect_changed_files(base_ref: Optional[str]) -> Set[str]:
    """Recopila archivos modificados, nuevos o sin seguimiento desde Git."""
    changed: Set[str] = set()

    rc, out, _ = run_capture(["git", "status", "--porcelain"])
    if rc == 0:
        for raw_line in out.splitlines():
            if len(raw_line) < 4:
                continue
            path_part = raw_line[3:].strip()
            if " -> " in path_part:
                path_part = path_part.split(" -> ", 1)[1].strip()
            path_part = path_part.strip('"').replace("\\", "/")
            if path_part:
                changed.add(path_part)

    if base_ref:
        rc_diff, out_diff, err_diff = run_capture(
            ["git", "diff", "--name-only", base_ref]
        )
        if rc_diff != 0:
            raise RuntimeError(
                f"No se pudo comparar con la referencia '{base_ref}': {err_diff.strip()}"
            )
        for line in out_diff.splitlines():
            p = line.strip().replace("\\", "/")
            if p:
                changed.add(p)

    return changed


def resolve_test_target(
    crate_info: CrateInfo, rel_path: str
) -> Optional[Tuple[str, Optional[str]]]:
    """Mapea un archivo bajo `tests/` al binario de test y filtro de módulo."""
    tests_prefix = f"{crate_info.rel_dir}/tests/"
    if not rel_path.startswith(tests_prefix):
        return None
    sub = rel_path[len(tests_prefix) :]
    if not sub.endswith(".rs"):
        return None

    parts = sub.split("/")
    if len(parts) == 1:
        stem = Path(parts[0]).stem
        if stem in crate_info.test_targets:
            return stem, None
        return None

    suite_dir = parts[0]
    if suite_dir in crate_info.test_targets:
        file_stem = Path(parts[-1]).stem
        if file_stem in ("main", "mod"):
            return suite_dir, None
        return suite_dir, f"{file_stem}::"

    # Soporte para subdirectorios auxiliares como `tier3_external_reference/` o `common/`
    for tgt_name, src_rel in crate_info.test_targets.items():
        if src_rel == f"{tests_prefix}{suite_dir}/main.rs":
            return tgt_name, None
    return suite_dir if suite_dir in crate_info.test_targets else None, None


def validate_web_js(dry_run: bool) -> Tuple[bool, float]:
    """Valida la sintaxis de todos los bloques <script> en `web/index.html` y los módulos JS en `web/assets/`."""
    assets_dir = ROOT / "web" / "assets"
    external_js = sorted(assets_dir.rglob("*.js")) if assets_dir.is_dir() else []
    if not WEB_INDEX_PATH.is_file() and not external_js:
        return True, 0.0

    node_bin = shutil.which("node")
    print("\n==> Validando sintaxis JavaScript en web/index.html y web/assets/...", flush=True)
    if dry_run:
        return True, 0.0
    if not node_bin:
        print("  [AVISO] 'node' no está en PATH; se omite comprobación sintáctica JS.", flush=True)
        return True, 0.0

    t0 = time.monotonic()
    scripts: List[str] = []
    if WEB_INDEX_PATH.is_file():
        html = WEB_INDEX_PATH.read_text(encoding="utf-8")
        scripts = re.findall(r"<script[^>]*>(.*?)</script>", html, re.S)
    temp_paths: List[Path] = []
    inline_count = 0
    try:
        for idx, block in enumerate(scripts):
            if not block.strip():
                continue
            inline_count += 1
            with tempfile.NamedTemporaryFile(
                "w", suffix=f"_script_{idx}.js", encoding="utf-8", delete=False
            ) as tmp:
                tmp.write(block)
                temp_paths.append(Path(tmp.name))

            proc = subprocess.run(
                [node_bin, "--check", str(temp_paths[-1])],
                capture_output=True,
                text=True,
                encoding="utf-8",
            )
            if proc.returncode != 0:
                print(
                    f"  [ERROR] Error de sintaxis JS en bloque inline #{idx}:\n{proc.stderr}",
                    file=sys.stderr,
                )
                return False, time.monotonic() - t0

        for js_path in external_js:
            proc = subprocess.run(
                [node_bin, "--check", str(js_path)],
                capture_output=True,
                text=True,
                encoding="utf-8",
            )
            if proc.returncode != 0:
                print(
                    f"  [ERROR] Error de sintaxis JS en {js_path.relative_to(ROOT)}:\n{proc.stderr}",
                    file=sys.stderr,
                )
                return False, time.monotonic() - t0
    finally:
        for p in temp_paths:
            try:
                p.unlink()
            except OSError:
                pass

    elapsed = time.monotonic() - t0
    print(
        f"  [OK] {inline_count} bloque(s) inline y {len(external_js)} archivo(s) JS externos validados en {elapsed:.2f}s.",
        flush=True,
    )
    return True, elapsed


def run_duplication_guard(dry_run: bool) -> Tuple[bool, float]:
    """Ejecuta `scripts/find_duplicates.py` y verifica la sección B1."""
    print("\n==> Comprobando clones exactos en Rust (guardia B1)...", flush=True)
    if dry_run:
        return True, 0.0
    if not FIND_DUPS_PATH.is_file():
        print("  [AVISO] No se encontró scripts/find_duplicates.py.", flush=True)
        return True, 0.0

    t0 = time.monotonic()
    rc, out, err = run_capture([sys.executable, str(FIND_DUPS_PATH)])
    elapsed = time.monotonic() - t0
    if rc != 0:
        print(f"  [ERROR] find_duplicates.py falló:\n{err}", file=sys.stderr)
        return False, elapsed

    allowed: Set[str] = set()
    if ALLOWLIST_PATH.is_file():
        for line in ALLOWLIST_PATH.read_text(encoding="utf-8").splitlines():
            clean = line.strip()
            if clean and not clean.startswith("#"):
                allowed.add(clean)

    in_b1 = False
    new_groups: List[str] = []
    known_count = 0
    for line in out.splitlines():
        if line.startswith("== B1."):
            in_b1 = True
            continue
        if line.startswith("== B2."):
            break
        if in_b1:
            m = re.match(r"^\s*\[(.+?)\]\s+x\d+", line)
            if m:
                group = m.group(1)
                if group in allowed:
                    known_count += 1
                else:
                    new_groups.append(group)

    if new_groups:
        print("  [ERROR] Nuevos clones exactos (B1) detectados:", file=sys.stderr)
        for g in new_groups:
            print(f"    - [{g}]", file=sys.stderr)
        return False, elapsed

    print(
        f"  [OK] Sin nuevos clones exactos ({known_count} grupo(s) en lista blanca) en {elapsed:.2f}s.",
        flush=True,
    )
    return True, elapsed


def print_slowest_summary(all_outputs: List[str], timings: List[Tuple[str, float]]) -> None:
    """Muestra el resumen de tiempos por fase y las 10 suites más lentas."""
    print("\n==================================================")
    print("  Resumen de ejecución y tiempos")
    print("==================================================")
    total = 0.0
    for label, secs in timings:
        total += secs
        print(f"  {label:<32} {secs:6.2f}s")
    print(f"  {'TOTAL':<32} {total:6.2f}s")

    combined = "\n".join(all_outputs)
    suites = re.findall(
        r"Running ([^\n]+)\n(?:(?!Running )[^\n]*\n)*?test result:[^\n]*finished in ([0-9.]+)s",
        combined,
    )
    if suites:
        ranked = sorted(((float(t), name.strip()) for name, t in suites), reverse=True)
        print("\n  Top 10 binarios de prueba más lentos:")
        for t, name in ranked[:10]:
            print(f"    {t:6.2f}s  {name}")


def build_feature_flags(all_features: bool) -> List[str]:
    return ["--all-features"] if all_features else []


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Corredor inteligente de pruebas y validación para Pulse Core."
    )
    parser.add_argument(
        "--mode",
        choices=["smart", "fast", "full"],
        default="smart",
        help="smart: solo lo afectado; fast: unitarias rápidas (--lib); full: validación completa.",
    )
    parser.add_argument(
        "--base",
        help="Referencia Git adicional para comparar (ej. HEAD~1 o origin/main).",
    )
    parser.add_argument(
        "--crate",
        action="append",
        default=[],
        help="Fuerza la ejecución sobre un crate concreto (repetible).",
    )
    parser.add_argument(
        "--filter",
        help="Filtro de nombre de prueba pasado tras '--' a cargo test.",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Incluye todos los crates del workspace en modos smart o fast.",
    )
    parser.add_argument(
        "--no-dependents",
        action="store_true",
        help="En modo smart, no expande los crates dependientes inversos.",
    )
    parser.add_argument(
        "--no-all-features",
        action="store_true",
        help="No añade --all-features en las invocaciones de cargo.",
    )
    parser.add_argument(
        "--skip-clippy",
        action="store_true",
        help="Omite la fase de cargo clippy.",
    )
    parser.add_argument(
        "--skip-dups",
        action="store_true",
        help="Omite la guardia de clones exactos (find_duplicates.py).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Muestra el plan de ejecución calculado sin ejecutar comandos pesados.",
    )
    args = parser.parse_args()

    all_features = not args.no_all_features
    feat_flags = build_feature_flags(all_features)
    crates, reverse_deps = load_workspace_graph()

    for c in args.crate:
        if c not in crates:
            print(f"[ERROR] Crate desconocido en el workspace: {c}", file=sys.stderr)
            return 2

    timings: List[Tuple[str, float]] = []
    outputs: List[str] = []

    if args.mode == "full":
        ok_js, t_js = validate_web_js(args.dry_run)
        timings.append(("Validación JS (web/index.html)", t_js))
        if not ok_js:
            return 1

        if not args.skip_clippy:
            cmd = ["cargo", "clippy", "--workspace", "--all-targets", *feat_flags, "--", "-D", "warnings"]
            rc, t_clippy, out = run_streaming(cmd, args.dry_run)
            timings.append(("Clippy (workspace completo)", t_clippy))
            outputs.append(out)
            if rc != 0:
                return rc

        if not args.skip_dups:
            ok_dups, t_dups = run_duplication_guard(args.dry_run)
            timings.append(("Guardia de duplicados (B1)", t_dups))
            if not ok_dups:
                return 1

        test_cmd = ["cargo", "test", "--workspace", *feat_flags, "--lib", "--tests"]
        if args.filter:
            test_cmd.extend(["--", args.filter])
        rc, t_test, out = run_streaming(test_cmd, args.dry_run)
        timings.append(("Pruebas (workspace completo)", t_test))
        outputs.append(out)
        print_slowest_summary(outputs, timings)
        return rc

    # Analizar cambios locales de Git para modos `smart` y `fast`
    changed_files = collect_changed_files(args.base)
    global_change = args.all or any(p in GLOBAL_TRIGGER_FILES for p in changed_files)
    web_changed = any(p.startswith("web/") for p in changed_files)

    src_crates: Set[str] = set(args.crate)
    targeted_tests: List[TestInvocation] = []
    rust_test_crates: Set[str] = set()

    for path in sorted(changed_files):
        for cname, cinfo in crates.items():
            prefix = f"{cinfo.rel_dir}/"
            if path.startswith(prefix):
                if path.startswith(f"{prefix}tests/"):
                    rust_test_crates.add(cname)
                    resolved = resolve_test_target(cinfo, path)
                    if resolved:
                        bin_name, mod_filter = resolved
                        targeted_tests.append(
                            TestInvocation(
                                crate=cname,
                                test_bin=bin_name,
                                filter_expr=args.filter or mod_filter,
                            )
                        )
                    else:
                        targeted_tests.append(TestInvocation(crate=cname))
                elif path.endswith(".rs") or path.endswith("Cargo.toml") or path.endswith("build.rs"):
                    src_crates.add(cname)
                break

    if args.mode == "fast":
        if global_change or (not src_crates and not rust_test_crates):
            fast_crates = sorted(crates.keys())
        else:
            seeds = src_crates | rust_test_crates
            fast_crates = sorted(
                seeds if args.no_dependents else transitive_dependents(seeds, reverse_deps)
            )
        print(f"==> Modo rápido (--lib) sobre crates: {', '.join(fast_crates)}")
        pkg_flags: List[str] = []
        for c in fast_crates:
            pkg_flags.extend(["-p", c])
        cmd = ["cargo", "test", *pkg_flags, *feat_flags, "--lib"]
        if args.filter:
            cmd.extend(["--", args.filter])
        rc, t_fast, out = run_streaming(cmd, args.dry_run)
        timings.append(("Pruebas unitarias rápidas", t_fast))
        outputs.append(out)
        print_slowest_summary(outputs, timings)
        return rc

    # Modo `smart`
    if not global_change and not src_crates and not targeted_tests and not web_changed:
        print(
            "==> No se detectaron cambios locales en Git. Usa `--mode fast --all` o `--mode full` para ejecutar pruebas igualmente."
        )
        return 0

    if web_changed:
        ok_js, t_js = validate_web_js(args.dry_run)
        timings.append(("Validación JS (web/index.html)", t_js))
        if not ok_js:
            return 1

    # Caso 1: Cambio exclusivo en web/ sin cambios Rust
    if web_changed and not global_change and not src_crates and not targeted_tests:
        rc, t_chk, out = run_streaming(
            ["cargo", "check", "-p", "mule-daemon", *feat_flags], args.dry_run
        )
        timings.append(("Check mule-daemon (embebido web)", t_chk))
        outputs.append(out)
        print_slowest_summary(outputs, timings)
        return rc

    # Caso 2: Cambio global -> todo el workspace
    if global_change:
        if not args.skip_clippy:
            rc, t_clip, out = run_streaming(
                ["cargo", "clippy", "--workspace", "--all-targets", *feat_flags, "--", "-D", "warnings"],
                args.dry_run,
            )
            timings.append(("Clippy (workspace completo)", t_clip))
            outputs.append(out)
            if rc != 0:
                return rc
        if not args.skip_dups:
            ok_dups, t_dups = run_duplication_guard(args.dry_run)
            timings.append(("Guardia de duplicados (B1)", t_dups))
            if not ok_dups:
                return 1
        cmd = ["cargo", "test", "--workspace", *feat_flags, "--lib", "--tests"]
        if args.filter:
            cmd.extend(["--", args.filter])
        rc, t_test, out = run_streaming(cmd, args.dry_run)
        timings.append(("Pruebas (workspace completo)", t_test))
        outputs.append(out)
        print_slowest_summary(outputs, timings)
        return rc

    # Caso 3: Solo cambiaron archivos de prueba concretos (sin src/)
    if not src_crates and targeted_tests:
        affected = sorted(rust_test_crates)
        print(f"==> Modo inteligente: solo suites de prueba modificadas en {', '.join(affected)}")
        if not args.skip_clippy:
            pkg_flags: List[str] = []
            for c in affected:
                pkg_flags.extend(["-p", c])
            rc, t_clip, out = run_streaming(
                ["cargo", "clippy", *pkg_flags, "--tests", *feat_flags, "--", "-D", "warnings"],
                args.dry_run,
            )
            timings.append(("Clippy (--tests focalizado)", t_clip))
            outputs.append(out)
            if rc != 0:
                return rc

        if not args.skip_dups:
            ok_dups, t_dups = run_duplication_guard(args.dry_run)
            timings.append(("Guardia de duplicados (B1)", t_dups))
            if not ok_dups:
                return 1

        # Deduplicar invocaciones; si algún cambio afecta a todo un binario (filter None), prevalece sobre módulos hijos
        by_bin: Dict[Tuple[str, Optional[str]], Optional[List[str]]] = {}
        for inv in targeted_tests:
            key = (inv.crate, inv.test_bin)
            if inv.filter_expr is None:
                by_bin[key] = None
            elif key not in by_bin:
                by_bin[key] = [inv.filter_expr]
            elif by_bin[key] is not None:
                assert by_bin[key] is not None
                if inv.filter_expr not in by_bin[key]:
                    by_bin[key].append(inv.filter_expr)

        for (crate, test_bin), filters in by_bin.items():
            cmd = ["cargo", "test", "-p", crate]
            if test_bin:
                cmd.extend(["--test", test_bin])
            else:
                cmd.append("--tests")
            cmd.extend(feat_flags)
            if filters and len(filters) == 1:
                cmd.extend(["--", filters[0]])
            elif args.filter:
                cmd.extend(["--", args.filter])

            label = f"Test {crate}::{test_bin or 'tests'}"
            rc, t_test, out = run_streaming(cmd, args.dry_run)
            timings.append((label, t_test))
            outputs.append(out)
            if rc != 0:
                print_slowest_summary(outputs, timings)
                return rc

        print_slowest_summary(outputs, timings)
        return 0

    # Caso 4: Cambios en src/ de crates -> clausura de dependientes + tests afectados
    affected_crates = set(
        src_crates if args.no_dependents else transitive_dependents(src_crates, reverse_deps)
    )
    affected_crates |= rust_test_crates
    if web_changed and "mule-daemon" in crates:
        affected_crates.add("mule-daemon")

    ordered_crates = sorted(affected_crates)
    print(f"==> Modo inteligente: crates afectados -> {', '.join(ordered_crates)}")

    pkg_flags = []
    for c in ordered_crates:
        pkg_flags.extend(["-p", c])

    if not args.skip_clippy:
        rc, t_clip, out = run_streaming(
            ["cargo", "clippy", *pkg_flags, "--all-targets", *feat_flags, "--", "-D", "warnings"],
            args.dry_run,
        )
        timings.append(("Clippy (crates afectados)", t_clip))
        outputs.append(out)
        if rc != 0:
            return rc

    if not args.skip_dups:
        ok_dups, t_dups = run_duplication_guard(args.dry_run)
        timings.append(("Guardia de duplicados (B1)", t_dups))
        if not ok_dups:
            return 1

    test_cmd = ["cargo", "test", *pkg_flags, *feat_flags, "--lib", "--tests"]
    if args.filter:
        test_cmd.extend(["--", args.filter])
    rc, t_test, out = run_streaming(test_cmd, args.dry_run)
    timings.append(("Pruebas (crates afectados)", t_test))
    outputs.append(out)
    print_slowest_summary(outputs, timings)
    return rc


if __name__ == "__main__":
    sys.exit(main())
