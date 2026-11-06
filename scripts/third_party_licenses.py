#!/usr/bin/env python3
"""Write THIRD_PARTY_LICENSES.txt for the Rust crates streamprobe is built from.

    python3 scripts/third_party_licenses.py --target x86_64-unknown-linux-musl
    python3 scripts/third_party_licenses.py --target aarch64-apple-darwin \\
        --target x86_64-apple-darwin -o dist/THIRD_PARTY_LICENSES.txt

For each --target it runs `cargo metadata --locked --filter-platform TARGET`,
walks the resolved dependency graph from the streamprobe package along normal
dependency edges (dev- and build-script-only dependencies are skipped), and
takes the union over all targets. For every crate it copies the licence files
that ship next to its Cargo.toml (LICENSE*, LICENCE*, COPYING*, NOTICE*,
UNLICENSE*, and anything under LICENSES/).

A crate that ships no licence file must have an entry in FALLBACKS below, with
its texts vendored in scripts/license-fallbacks/<crate>/. Anything else is an
error: the script exits non-zero and writes nothing, so a new dependency can't
slip into a release without its licence.

Standard library only; needs cargo on PATH (or $CARGO).
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT_PACKAGE = "streamprobe"
HERE = Path(__file__).resolve().parent
MANIFEST = HERE.parent / "Cargo.toml"
FALLBACK_DIR = HERE / "license-fallbacks"
LICENSE_NAME = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)", re.IGNORECASE)

# Crates published without any licence file. Their texts are vendored in
# scripts/license-fallbacks/<crate>/ from the upstream repository. The licence
# expression is recorded so that a later version that changes its licence
# fails here instead of silently reusing a stale text. When a crate starts
# shipping its own files, those are used and the fallback is ignored.
FALLBACKS = {
    "asn1-rs-impl": (
        "MIT OR Apache-2.0",
        "github.com/rusticata/asn1-rs @ a20e5f7 (the commit 0.2.0 was published from)",
    ),
    "dasp_frame": (
        "MIT OR Apache-2.0",
        "github.com/rustaudio/dasp @ 221b810 (the commit 0.11.0 was published from)",
    ),
    "dasp_sample": (
        "MIT OR Apache-2.0",
        "github.com/rustaudio/dasp @ 97c3bb9 (the commit 0.11.0 was published from)",
    ),
    "objc2": ("MIT", "github.com/madsmtm/objc2 @ ee9a7ad (licence texts added upstream)"),
    "objc2-encode": ("MIT", "github.com/madsmtm/objc2 @ ee9a7ad (licence texts added upstream)"),
    "block2": ("MIT", "github.com/madsmtm/objc2 @ ee9a7ad (licence texts added upstream)"),
    "dispatch2": (
        "Zlib OR Apache-2.0 OR MIT",
        "github.com/madsmtm/objc2 @ ee9a7ad (licence texts added upstream)",
    ),
}

RULE = "=" * 78


def spdx(expr):
    """Cargo still accepts the legacy 'MIT/Apache-2.0' form; print it as SPDX."""
    return re.sub(r"\s*/\s*", " OR ", expr.strip()) if expr else expr


def cargo_metadata(target):
    cmd = [
        os.environ.get("CARGO", "cargo"),
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--manifest-path",
        str(MANIFEST),
        "--filter-platform",
        target,
    ]
    try:
        out = subprocess.run(cmd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except FileNotFoundError:
        sys.exit("third_party_licenses: cargo not found on PATH")
    except subprocess.CalledProcessError as e:
        sys.stderr.write(e.stderr.decode(errors="replace"))
        sys.exit(f"third_party_licenses: cargo metadata failed for {target}")
    return json.loads(out.stdout)


def runtime_packages(meta):
    """Packages reachable from the root along normal (non-dev, non-build) edges."""
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [i for i, p in packages.items() if p["name"] == ROOT_PACKAGE and p["source"] is None]
    if len(roots) != 1:
        sys.exit(f"third_party_licenses: expected one local '{ROOT_PACKAGE}' package, found {len(roots)}")
    seen = set()
    stack = [roots[0]]
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            if not any(k["kind"] is None for k in dep["dep_kinds"]):
                continue  # only reachable as a dev- or build-dependency
            if dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return roots[0], packages, seen


def licence_files(pkg):
    """(files, fallback source or None, error or None) for one package."""
    pkg_dir = Path(pkg["manifest_path"]).parent
    files = [f for f in pkg_dir.iterdir() if f.is_file() and LICENSE_NAME.match(f.name)]
    for sub in ("LICENSES", "licenses"):  # REUSE-style layout
        if (pkg_dir / sub).is_dir():
            files += [f for f in (pkg_dir / sub).iterdir() if f.is_file()]
    if pkg.get("license_file"):
        declared = (pkg_dir / pkg["license_file"]).resolve()
        if declared.is_file() and declared not in [f.resolve() for f in files]:
            files.append(declared)
    if files:
        return sorted(set(files), key=lambda f: str(f.relative_to(pkg_dir))), None, None

    name = pkg["name"]
    if name not in FALLBACKS:
        return [], None, "ships no licence file and has no entry in FALLBACKS"
    expected, source = FALLBACKS[name]
    if spdx(pkg.get("license")) != expected:
        return [], None, (
            f"declares '{pkg.get('license')}' but its fallback was recorded for '{expected}': "
            "check the new licence and update FALLBACKS and the vendored texts"
        )
    fb_dir = FALLBACK_DIR / name
    files = sorted(f for f in fb_dir.iterdir() if f.is_file()) if fb_dir.is_dir() else []
    if not files:
        return [], None, f"has a FALLBACKS entry but {fb_dir.relative_to(HERE.parent)} is empty or missing"
    return files, source, None


def read_text(path):
    text = path.read_bytes().decode("utf-8", errors="replace")
    text = text.replace("\r\n", "\n").replace("\r", "\n").lstrip("﻿")
    return "\n".join(line.rstrip() for line in text.split("\n")).strip("\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument(
        "--target",
        action="append",
        required=True,
        metavar="TRIPLE",
        help="target triple to collect dependencies for (repeat for a universal build)",
    )
    ap.add_argument(
        "-o",
        "--output",
        default="THIRD_PARTY_LICENSES.txt",
        help="file to write, or - for stdout (default: %(default)s)",
    )
    args = ap.parse_args()
    targets = sorted(set(args.target))

    root_version = None
    packages = {}
    selected = set()
    for target in targets:
        meta = cargo_metadata(target)
        root, pkgs, ids = runtime_packages(meta)
        root_version = pkgs[root]["version"]
        packages.update(pkgs)
        selected |= ids

    crates = sorted((packages[i] for i in selected), key=lambda p: (p["name"], p["version"]))
    errors = []
    texts = {}  # normalised text -> index into blocks
    blocks = []  # [text, [(crate label, file name), ...]]
    rows = []
    for pkg in crates:
        label = f"{pkg['name']} {pkg['version']}"
        expr = spdx(pkg.get("license"))
        if not expr and pkg.get("license_file"):
            expr = f"see {pkg['license_file']}"
        if not expr:
            errors.append(f"{label}: declares no licence in Cargo.toml")
            continue
        files, source, err = licence_files(pkg)
        if err:
            errors.append(f"{label} ({expr}): {err}")
            continue
        refs = []
        for f in files:
            text = read_text(f)
            if not text:
                errors.append(f"{label}: licence file {f.name} is empty")
                continue
            if text not in texts:
                texts[text] = len(blocks)
                blocks.append([text, []])
            idx = texts[text]
            blocks[idx][1].append((label, f.name))
            if idx + 1 not in refs:
                refs.append(idx + 1)
        rows.append((label, expr, sorted(refs), source))

    if errors:
        sys.stderr.write("third_party_licenses: cannot account for every dependency's licence:\n")
        for e in errors:
            sys.stderr.write(f"  - {e}\n")
        sys.stderr.write(
            "Add the crate's upstream licence texts under scripts/license-fallbacks/<crate>/ "
            "and an entry in FALLBACKS, or drop the dependency.\n"
        )
        return 1

    width = max(len(r[0]) for r in rows) + 2
    expr_width = max(len(r[1]) for r in rows) + 2
    out = [
        "THIRD-PARTY LICENCES",
        RULE,
        "",
        f"streamprobe {root_version} is licensed under MIT OR Apache-2.0 (see LICENSE-MIT",
        "and LICENSE-APACHE). It is built from the Rust crates listed below, for:",
        "",
        *(f"    {t}" for t in targets),
        "",
        "Dependencies used only by build scripts or tests are not listed. Each",
        "licence text is printed once, followed by the crates and files it came",
        "from; the numbers in the list below point to those texts.",
        "",
        "ffmpeg and ffprobe are not part of streamprobe. They are separate programs",
        "that you install yourself, and streamprobe runs them as external processes.",
        "",
        "",
        f"CRATES ({len(rows)})",
        RULE,
        "",
    ]
    for label, expr, refs, source in rows:
        out.append(f"{label:<{width}}{expr:<{expr_width}}[{', '.join(map(str, refs))}]")
        if source:
            out.append(f"    (the crate ships no licence file; texts from {source})")
    out += ["", "", f"LICENCE TEXTS ({len(blocks)})", RULE]
    for n, (text, users) in enumerate(blocks, 1):
        out += ["", RULE, f"[{n}] used by:"]
        out += [f"    {label} ({name})" for label, name in users]
        out += [RULE, "", text, ""]
    body = "\n".join(out).rstrip("\n") + "\n"

    if args.output == "-":
        sys.stdout.write(body)
    else:
        Path(args.output).write_text(body, encoding="utf-8")
        print(
            f"third_party_licenses: {len(rows)} crates, {len(blocks)} distinct texts -> {args.output}",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
