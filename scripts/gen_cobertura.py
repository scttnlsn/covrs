#!/usr/bin/env python3

"""
Generate a Cobertura XML coverage file for performance benchmarking.

./scripts/gen_cobertura.py [--output FILE] [--target-mb SIZE]

"""

import argparse
import random
import sys

DIRS = [
    "src",
    "lib",
    "core",
    "api",
    "handlers",
    "middleware",
    "services",
    "models",
    "utils",
    "config",
    "db",
    "auth",
    "routes",
    "controllers",
    "views",
    "templates",
    "helpers",
    "validators",
    "serializers",
    "tasks",
    "workers",
    "consumers",
    "producers",
    "adapters",
    "clients",
    "providers",
    "repositories",
    "factories",
    "strategies",
    "observers",
    "decorators",
]

SUBCOMPONENTS = [
    "user",
    "account",
    "session",
    "payment",
    "order",
    "product",
    "cart",
    "inventory",
    "shipping",
    "notification",
    "email",
    "search",
    "cache",
    "logging",
    "metrics",
    "health",
    "admin",
    "report",
    "export",
    "import",
    "sync",
    "migration",
    "backup",
    "queue",
    "scheduler",
    "webhook",
    "oauth",
    "token",
    "permission",
    "role",
    "audit",
    "analytics",
]

EXTENSIONS = [".py", ".js", ".ts", ".rs", ".go", ".java", ".rb"]

METHOD_PREFIXES = [
    "get",
    "set",
    "create",
    "update",
    "delete",
    "find",
    "list",
    "validate",
    "process",
    "handle",
    "parse",
    "format",
    "convert",
    "serialize",
    "deserialize",
    "encode",
    "decode",
    "encrypt",
    "decrypt",
    "hash",
    "verify",
    "check",
    "is",
    "has",
    "can",
    "should",
    "init",
    "setup",
    "teardown",
    "cleanup",
    "reset",
    "refresh",
    "load",
    "save",
    "fetch",
    "send",
    "receive",
    "publish",
    "subscribe",
    "connect",
    "disconnect",
    "open",
    "close",
    "read",
    "write",
    "flush",
    "sync",
    "merge",
    "split",
    "filter",
    "map",
    "reduce",
    "sort",
    "search",
    "index",
    "compute",
    "calculate",
    "transform",
    "normalize",
    "sanitize",
    "escape",
    "render",
]

METHOD_SUFFIXES = [
    "data",
    "result",
    "response",
    "request",
    "config",
    "options",
    "params",
    "args",
    "context",
    "state",
    "status",
    "info",
    "details",
    "metadata",
    "record",
    "entry",
    "item",
    "element",
    "node",
    "value",
    "key",
    "id",
    "name",
    "type",
    "format",
    "schema",
    "model",
    "entity",
    "resource",
    "connection",
    "session",
    "transaction",
    "batch",
    "chunk",
    "page",
    "token",
    "header",
    "body",
    "payload",
    "message",
    "event",
    "signal",
    "callback",
    "handler",
    "listener",
    "observer",
    "subscriber",
    "worker",
]


def gen_filename(pkg_idx: int, cls_idx: int) -> str:
    d = DIRS[pkg_idx % len(DIRS)]
    sub = SUBCOMPONENTS[(pkg_idx * 7 + cls_idx) % len(SUBCOMPONENTS)]
    ext = EXTENSIONS[(pkg_idx + cls_idx) % len(EXTENSIONS)]
    suffix = f"_{cls_idx}" if cls_idx > 0 else ""
    return f"{d}/{sub}{suffix}{ext}"


def gen_method_name(idx: int) -> str:
    prefix = METHOD_PREFIXES[idx % len(METHOD_PREFIXES)]
    suffix = METHOD_SUFFIXES[(idx * 3 + 7) % len(METHOD_SUFFIXES)]
    return f"{prefix}_{suffix}"


def write_xml(out, target_bytes: int):
    written = 0

    def emit(s: str):
        nonlocal written
        data = s.encode("utf-8")
        out.buffer.write(data)
        written += len(data)

    emit('<?xml version="1.0" ?>\n')
    emit(
        '<coverage version="6.5.0" timestamp="1700000000000" '
        'lines-valid="999999" lines-covered="750000" line-rate="0.75" '
        'branches-covered="5000" branches-valid="10000" branch-rate="0.5" '
        'complexity="0">\n'
    )
    emit("    <sources>\n")
    emit("        <source>/home/user/project</source>\n")
    emit("    </sources>\n")
    emit("    <packages>\n")

    pkg_idx = 0
    while written < target_bytes:
        pkg_name = (
            f"{DIRS[pkg_idx % len(DIRS)]}.{SUBCOMPONENTS[pkg_idx % len(SUBCOMPONENTS)]}"
        )
        emit(
            f'        <package name="{pkg_name}" line-rate="0.75" '
            f'branch-rate="0.5" complexity="0">\n'
        )
        emit("            <classes>\n")

        # 5-15 classes per package
        num_classes = 5 + (pkg_idx * 7 + 3) % 11
        for cls_idx in range(num_classes):
            filename = gen_filename(pkg_idx, cls_idx)
            classname = filename.rsplit(".", 1)[0].replace("/", ".")
            line_rate = round(random.uniform(0.4, 1.0), 2)
            branch_rate = round(random.uniform(0.3, 1.0), 2)

            emit(
                f'                <class name="{classname}" filename="{filename}" '
                f'complexity="0" line-rate="{line_rate}" branch-rate="{branch_rate}">\n'
            )

            # Methods
            num_methods = 3 + (pkg_idx + cls_idx) % 12
            emit("                    <methods>\n")
            line_num = 1
            for m_idx in range(num_methods):
                method_name = gen_method_name(pkg_idx * 100 + cls_idx * 10 + m_idx)
                method_line_rate = round(random.uniform(0.5, 1.0), 2)
                emit(
                    f'                        <method name="{method_name}" '
                    f'signature="()" line-rate="{method_line_rate}" branch-rate="0">\n'
                )
                emit("                            <lines>\n")
                # 3-8 lines per method
                num_method_lines = 3 + (m_idx * 3 + cls_idx) % 6
                for _ in range(num_method_lines):
                    hits = random.choice([0, 0, 1, 1, 1, 2, 3, 5, 10, 42])
                    emit(
                        f'                                <line number="{line_num}" hits="{hits}"/>\n'
                    )
                    line_num += 1
                emit("                            </lines>\n")
                emit("                        </method>\n")
            emit("                    </methods>\n")

            # Class-level lines (all method lines + some extras)
            total_lines = line_num + random.randint(5, 20)
            emit("                    <lines>\n")
            for ln in range(1, total_lines + 1):
                hits = random.choice([0, 0, 1, 1, 1, 2, 3, 5, 10, 42])
                # ~15% of lines are branch points
                if random.random() < 0.15:
                    covered = random.randint(0, 4)
                    total_branches = random.randint(covered, covered + 4)
                    if total_branches == 0:
                        total_branches = 2
                        covered = random.randint(0, 2)
                    pct = round(100 * covered / total_branches)
                    emit(
                        f'                        <line number="{ln}" hits="{hits}" '
                        f'branch="true" condition-coverage="{pct}% ({covered}/{total_branches})"/>\n'
                    )
                else:
                    emit(
                        f'                        <line number="{ln}" hits="{hits}"/>\n'
                    )
            emit("                    </lines>\n")
            emit("                </class>\n")

            # Early exit check inside the class loop
            if written >= target_bytes:
                break

        emit("            </classes>\n")
        emit("        </package>\n")
        pkg_idx += 1

        # Progress indicator on stderr
        if pkg_idx % 50 == 0:
            mb = written / (1024 * 1024)
            print(f"  ... {pkg_idx} packages, {mb:.1f} MB", file=sys.stderr)

    emit("    </packages>\n")
    emit("</coverage>\n")

    return written


def main():
    parser = argparse.ArgumentParser(
        description="Generate a large Cobertura XML file for benchmarking"
    )
    parser.add_argument(
        "--output",
        "-o",
        help="Output file path",
    )
    parser.add_argument(
        "--target-mb",
        type=float,
        default=10.0,
        help="Target file size in MB (default: 10)",
    )
    args = parser.parse_args()

    target_bytes = int(args.target_mb * 1024 * 1024)

    print(
        f"Generating ~{args.target_mb} MB Cobertura XML -> {args.output}",
        file=sys.stderr,
    )

    with open(args.output, "w", encoding="utf-8") as f:
        total = write_xml(f, target_bytes)

    mb = total / (1024 * 1024)
    print(f"Done: {mb:.1f} MB written to {args.output}", file=sys.stderr)


if __name__ == "__main__":
    main()
