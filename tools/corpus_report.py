#!/usr/bin/env python3
"""Informe de clusters del corpus de tests de CPython.

Analiza los logs de la última pasada de `make test-cpython` y produce:

1. Estado por archivo (PASS/SKIP/FAIL/TIMEOUT/PARSE_ERROR) y los fallos
   restantes por archivo (`failures=` + `errors=` de la línea final de
   unittest), ordenados de menos a más — los "near-miss" son los que más
   rápido convierten a PASS.
2. Un índice de CLUSTERS por firma de error normalizada (tipo de excepción
   + plantilla de mensaje), ordenado por número de archivos afectados: un
   cluster que toca 30 archivos es un fix que saca 30 archivos a PASS mucho
   más rápido que 30 fixes de un archivo cada uno.
3. "Gate blockers": archivos que ni siquiera llegan a ejecutar sus tests
   (error de import al cargar el módulo, error de parseo del test).

Uso:
    python3 tools/corpus_report.py [--log-dir DIR] [--top N] [--json OUT]

Por defecto lee /tmp/rustpython-test-logs/cpython (donde `make test-cpython`
deja los `.log` por archivo y `summary/<archivo>.txt` con la línea de estado).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from collections import Counter, defaultdict

DEFAULT_LOG_DIR = "/tmp/rustpython-test-logs/cpython"

# Líneas de estado que emite tests/cpython_runner.sh:
#   PASS|test_name|123ms|45MB   SKIP|...   FAIL|...   TIMEOUT|...   PARSE_ERROR|...
STATUS_RE = re.compile(r"^(PASS|SKIP|FAIL|TIMEOUT|PARSE_ERROR)\|([^|]+)\|(\d+)ms\|(\d+)MB")

# Línea final de unittest: "FAILED (failures=3, errors=1, skipped=2)".
UNITTEST_SUMMARY_RE = re.compile(
    r"(?:FAILED|OK)\s*\((.*)\)\s*$", re.MULTILINE
)
COUNT_RE = re.compile(r"(failures|errors)=(\d+)")

# Cabecera de cada caso fallido/error dentro de un .log de unittest:
#   FAIL: test_name (modulo.Clase.test_name)   [subTest args]
#   ERROR: test_name (modulo.Clase.test_name)
CASE_RE = re.compile(r"^(FAIL|ERROR): (.+?) \((\S+?)\)(.*)$", re.MULTILINE)

# Última línea de excepción/error de un bloque. Las variantes con Raising de
# unittest ("During handling of the above exception...") ya se limpian abajo.
EXC_RE = re.compile(
    r"^(?:[A-Za-z_][\w.]*)?(?:Error|Exception|Exit|Warning|Interrupt|Failure)"
    r"(?::[ \t]+(.*))?$"
)

# Normalizaciones de la firma: lo que cambia entre archivos pero NO
# describe un bug distinto (direcciones, rutas, números de línea, índices
# de具体的 subtest). Los NOMBRES entrecomillados se conservan a propósito:
# `no attribute 'verify'` y `no attribute '_types_native'` son bugs
# distintos, y aplanarlos a 'NAME' destruye justamente la señal que sirve
# para priorizar.
NORMALIZERS = (
    (re.compile(r"0x[0-9a-fA-F]{4,}"), "0xADDR"),
    (re.compile(r"'[^']*(?:/|\\)[^']*'"), "'PATH'"),
    (re.compile(r"\bline \d+\b"), "line N"),
    (re.compile(r"\(\d+\)"), "(N)"),
    (re.compile(r"\b\d{3,}\b"), "N"),
    (re.compile(r"\s+"), " "),
)

# Mensajes que son demasiado genéricos para agrupar: se afinan por el test
# que los produce (el nombre del test SÍ discrimina).
GENERIC_MESSAGES = {"", "False is not true", "True is not false", "None is not true"}


def normalize_signature(text: str) -> str:
    text = text.strip()
    for pattern, replacement in NORMALIZERS:
        text = pattern.sub(replacement, text)
    return text[:160]


def read_text(path: str) -> str:
    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        return handle.read()


def parse_status(summary_dir: str) -> dict[str, dict]:
    """Última línea de estado por archivo (los .txt se anexan en cada pasada)."""
    statuses: dict[str, dict] = {}
    if not os.path.isdir(summary_dir):
        return statuses
    for entry in sorted(os.listdir(summary_dir)):
        if not entry.endswith(".txt"):
            continue
        name = entry[:-4]
        match = None
        for line in reversed(read_text(os.path.join(summary_dir, entry)).splitlines()):
            match = STATUS_RE.match(line.strip())
            if match:
                break
        if match:
            statuses[name] = {
                "status": match.group(1),
                "ms": int(match.group(3)),
                "mb": int(match.group(4)),
            }
    return statuses


def parse_remaining(log: str) -> tuple[int, int]:
    """(failures, errors) de la última línea de resumen de unittest."""
    matches = UNITTEST_SUMMARY_RE.findall(log)
    if not matches:
        # Sin línea de resumen: el archivo murió antes (timeout/parse error,
        # import error, o un crash del intérprete). Se cuenta aparte como
        # CRASH en vez de aparecer como "0 fallos" (que parece un near-miss
        # cuando en realidad el archivo no llegó a ejecutar sus tests).
        return 0, 0
    tail = matches[-1]
    failures = errors = 0
    for key, value in COUNT_RE.findall(tail):
        if key == "failures":
            failures = int(value)
        else:
            errors = int(value)
    return (failures, errors)


def crash_reason(log: str) -> str | None:
    """Motivo del crash cuando no hay resumen de unittest."""
    for raw in log.splitlines():
        line = raw.strip()
        if not line or line.startswith(("Traceback", "File \"", "During handling", "JIT:")):
            continue
        if re.match(r"^[A-Za-z_][\w.]*(Error|Exception|Exit|Failure|Interrupt)", line) or \
                line.startswith(("Parse error", "Exception:", "assert", "thread '")):
            return normalize_signature(line)
    tail = [l.strip() for l in log.splitlines() if l.strip()]
    return normalize_signature(tail[-1]) if tail else "(log vacío)"


def iter_cases(log: str):
    """Yield (kind, case_name, exception_type, normalized_message) por caso."""
    blocks = re.split(r"\n={70}\n", log)
    for block in blocks:
        header = None
        for line in block.splitlines():
            match = CASE_RE.match(line.strip())
            if match and header is None:
                header = match
            if not line.strip():
                continue
        if header is None:
            continue
        kind, case_name, _module, _extra = header.groups()
        body = block.split("\n", 1)[1] if "\n" in block else ""
        # "During handling of the above exception..." encadena varios
        # tracebacks: nos interesa el ÚLTIMO (la excepción real).
        exc_type = "?"
        message = "(sin línea de excepción)"
        for raw in reversed(body.splitlines()):
            line = raw.strip()
            if not line or line.startswith(("Traceback", "File \"", "During handling")):
                continue
            match = EXC_RE.match(line)
            if match:
                tail = match.group(1) or ""
                if exc_type == "?":
                    exc_type = line.split(":", 1)[0] if ":" in line else line
                message = tail or line
        # Si no encontramos una línea de excepción, miramos la última
        # `AssertionError` del bloque (unittest las formatea así).
        if message == "(sin línea de excepción)":
            for raw in reversed(body.splitlines()):
                line = raw.strip()
                if line.startswith("AssertionError"):
                    exc_type, message = "AssertionError", line
                    break
        yield (kind, case_name, exc_type, normalize_signature(message))


def load_blocker(log: str) -> str | None:
    """Motivo por el que el archivo ni siquiera pudo correr sus tests."""
    head = log[:4000]
    for pattern, label in (
        (r"^ModuleNotFoundError: (.+)$", "ModuleNotFoundError"),
        (r"^ImportError: (.+)$", "ImportError"),
        (r"^Parse error: (.+)$", "ParseError"),
        (r"^SyntaxError: (.+)$", "SyntaxError"),
    ):
        match = re.search(pattern, head, re.MULTILINE)
        if match:
            return f"{label}: {normalize_signature(match.group(1))}"
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log-dir", default=DEFAULT_LOG_DIR)
    parser.add_argument("--top", type=int, default=25, help="cuántos clusters listar")
    parser.add_argument("--json", help="escribe también el informe en JSON")
    parser.add_argument(
        "--compare",
        help=(
            "JSON de una pasada anterior; muestra los deltas por archivo "
            "(PASS↔FAIL, yDelta de subtests) y el total"
        ),
    )
    args = parser.parse_args()

    log_dir = args.log_dir
    summary_dir = os.path.join(log_dir, "summary")
    statuses = parse_status(summary_dir)

    files: dict[str, dict] = {}
    clusters: dict[tuple[str, str], dict] = {}
    blockers: dict[str, list[str]] = defaultdict(list)

    for name in sorted(statuses):
        path = os.path.join(log_dir, f"{name}.log")
        if not os.path.exists(path):
            continue
        log = read_text(path)
        failures, errors = parse_remaining(log)
        blocker = load_blocker(log)
        crashed = None
        if not UNITTEST_SUMMARY_RE.findall(log) and statuses[name]["status"] != "PASS":
            crashed = crash_reason(log)
        files[name] = {
            "status": statuses[name]["status"],
            "ms": statuses[name]["ms"],
            "mb": statuses[name]["mb"],
            "failures": failures,
            "errors": errors,
            "remaining": failures + errors,
            "blocker": blocker,
            "crash": crashed,
        }
        if blocker:
            blockers[blocker].append(name)
        for kind, case, exc_type, message in iter_cases(log):
            # Un `AssertionError: False is not true` no dice nada por sí solo:
            # se agrupa por test, que es donde está la información real.
            if exc_type == "AssertionError" and message in GENERIC_MESSAGES:
                key = (exc_type, f"{case} :: {message}")
            else:
                key = (exc_type, message)
            entry = clusters.setdefault(
                key, {"files": set(), "cases": 0, "kinds": Counter(), "examples": []}
            )
            entry["files"].add(name)
            entry["cases"] += 1
            entry["kinds"][kind] += 1
            if len(entry["examples"]) < 3:
                entry["examples"].append(f"{name}: {case}")

    by_status = Counter(entry["status"] for entry in files.values())
    total_remaining = sum(entry["remaining"] for entry in files.values())
    total_files = len(files)

    print("=" * 78)
    print("RESUMEN")
    print("=" * 78)
    for status in ("PASS", "SKIP", "FAIL", "TIMEOUT", "PARSE_ERROR"):
        if by_status.get(status):
            print(f"  {status:<12} {by_status[status]:>4}")
    print(f"  {'TOTAL':<12} {total_files:>4}")
    print(f"  subtests fallidos restantes: {total_remaining}")

    passing = sum(1 for e in files.values() if e["status"] == "PASS")
    print(f"  progreso PASS: {passing}/{total_files}")

    print()
    print("=" * 78)
    print("NEAR-MISS (archivos no-PASS con menos fallos restantes)")
    print("=" * 78)
    # Solo archivos que LLEGARON al resumen de unittest: los que no (crash /
    # timeout / import error) se listan en su propia sección, y mezclarlos
    # aquí los haría parecer near-miss cuando en realidad no corrieron.
    near = sorted(
        (
            (name, entry)
            for name, entry in files.items()
            if entry["status"] in ("FAIL", "TIMEOUT", "PARSE_ERROR")
            and not entry.get("crash")
        ),
        key=lambda pair: (pair[1]["remaining"], pair[1]["ms"]),
    )
    for name, entry in near[: args.top]:
        extra = f"  [{entry['blocker']}]" if entry["blocker"] else ""
        print(
            f"  {entry['remaining']:>5}  {entry['status']:<12} "
            f"{entry['ms'] / 1000:>7.1f}s  {name}{extra}"
        )

    crashed = sorted(
        (name, entry)
        for name, entry in files.items()
        if entry.get("crash") and entry["status"] != "SKIP"
    )
    if crashed:
        print()
        print("=" * 78)
        print(f"CRASH (no llegaron al resumen de unittest): {len(crashed)}")
        print("=" * 78)
        for name, entry in crashed:
            print(f"  {entry['status']:<12} {name:<34} {entry['crash']}")

    print()
    print("=" * 78)
    print(f"CLUSTERS (top {args.top} por nº de archivos afectados)")
    print("=" * 78)
    ranked = sorted(
        clusters.items(),
        key=lambda kv: (-len(kv[1]["files"]), -kv[1]["cases"]),
    )
    for (exc_type, message), entry in ranked[: args.top]:
        print(
            f"  {len(entry['files']):>4} archivos  {entry['cases']:>5} casos  "
            f"{exc_type}: {message}"
        )
        for example in entry["examples"][:2]:
            print(f"          p.ej. {example}")

    if blockers:
        print()
        print("=" * 78)
        print("GATE BLOCKERS (el archivo no llega a ejecutar tests)")
        print("=" * 78)
        for reason, names in sorted(blockers.items(), key=lambda kv: -len(kv[1])):
            print(f"  {len(names):>3} archivos  {reason}")
            print(f"          {', '.join(names[:8])}{' ...' if len(names) > 8 else ''}")

    if args.compare:
        try:
            with open(args.compare, encoding="utf-8") as handle:
                old_report = json.load(handle)
        except OSError as exc:
            print(f"\n(no se pudo leer {args.compare}: {exc})")
        else:
            print()
            print("=" * 78)
            print(f"COMPARACIÓN contra {args.compare}")
            print("=" * 78)
            old_files = old_report.get("files", {})
            newly_passing = []
            newly_failing = []
            deltas = []
            for name, entry in files.items():
                previous = old_files.get(name)
                if previous is None:
                    continue
                was = previous.get("status")
                now = entry["status"]
                if was != now:
                    line = f"  {name:<36} {was} -> {now}"
                    if was in ("FAIL", "TIMEOUT", "PARSE_ERROR") and now in ("PASS", "SKIP"):
                        newly_passing.append(name)
                    elif now in ("FAIL", "TIMEOUT", "PARSE_ERROR") and was in ("PASS", "SKIP"):
                        newly_failing.append(name)
                    print(line)
                delta = entry["remaining"] - previous.get("remaining", 0)
                if delta:
                    deltas.append((delta, name))
            old_total = old_report.get("remaining_subtests", 0)
            print()
            print(f"  subtests fallidos: {old_total} -> {total_remaining} "
                  f"({total_remaining - old_total:+d})")
            print(f"  archivos que pasan a PASS/SKIP: {len(newly_passing)}")
            for name in newly_passing:
                print(f"      + {name}")
            print(f"  archivos que REGRESIONAN: {len(newly_failing)}")
            for name in newly_failing:
                print(f"      - {name}")
            if deltas:
                deltas.sort()
                print("  peores deltas (más subtests nuevos):")
                for delta, name in deltas[:10]:
                    print(f"      {delta:+6d}  {name}")
                print("  mejores deltas:")
                for delta, name in deltas[-10:][::-1]:
                    print(f"      {delta:+6d}  {name}")

    if args.json:
        payload = {
            "summary": dict(by_status),
            "total_files": total_files,
            "passing": passing,
            "remaining_subtests": total_remaining,
            "files": {name: {**entry, "blocker": entry["blocker"]} for name, entry in files.items()},
            "clusters": [
                {
                    "exception": exc_type,
                    "message": message,
                    "files": sorted(entry["files"]),
                    "file_count": len(entry["files"]),
                    "cases": entry["cases"],
                    "examples": entry["examples"],
                }
                for (exc_type, message), entry in ranked
            ],
            "blockers": {reason: names for reason, names in blockers.items()},
        }
        with open(args.json, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=1, ensure_ascii=False)
        print(f"\n(informe JSON escrito en {args.json})")

    return 0


if __name__ == "__main__":
    sys.exit(main())
