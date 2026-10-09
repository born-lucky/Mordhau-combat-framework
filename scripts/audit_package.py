"""Audit the source-only v1 staging tree. This does not certify binary releases."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from reviewed_reader_sources import OWN_READER_SOURCE

ALLOWED_SUFFIXES = {".rs", ".toml", ".lock", ".cpp", ".h", ".md", ".py", ".yml", ".sh", ".ps1"}
ALLOWED_JSON = {"release-base.json", "release-state.json"}
OWN_SHADER_SOURCE = {"core/crates/mh-assets/shaders/ue_tint.wgsl", "core/crates/mh-runtime/src/uepost.wgsl"}
OWN_EMBEDDED_SOURCE = {
    "core/crates/mh-setup/src/main.rs": {b"setup.ps1"},
    "core/crates/mh-assets/src/shader.rs": {b"../shaders/ue_tint.wgsl"},
    "core/crates/mh-runtime/src/uepost_render.rs": {b"uepost.wgsl"},
}
EXCLUDED_DIRS = {"extract", "ghidra", "state", "data_gen", "sheets", "cache", "sdk", "vendor", "node_modules"}
GAME_SUFFIXES = {".exe", ".dll", ".pak", ".pdb", ".uasset", ".uexp", ".ubulk", ".glb", ".gltf", ".wgsl", ".tsv", ".xlsx", ".png", ".jpg", ".ogg", ".wav", ".mp3", ".mp4", ".zip", ".7z"}
SENSITIVE = re.compile(rb"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)")


def errors_for(path: Path, relative: str) -> list[str]:
    errors = []
    st = path.lstat()
    if path.is_symlink() or getattr(st, "st_file_attributes", 0) & 0x400:
        return [f"Reparse/symlink input: {relative}"]
    parts = Path(relative).parts
    if any(p.lower() in EXCLUDED_DIRS for p in parts):
        errors.append(f"Private or generated directory: {relative}")
    suffix = path.suffix.lower()
    allowed = suffix in ALLOWED_SUFFIXES or relative in ALLOWED_JSON or relative in OWN_SHADER_SOURCE or relative in OWN_READER_SOURCE or relative == ".gitignore" or (relative.startswith("demo/") and suffix == ".txt")
    if (suffix in GAME_SUFFIXES and relative not in OWN_SHADER_SOURCE) or not allowed:
        errors.append(f"Unreviewed file type/path: {relative}")
    if path.stat().st_size > 2_000_000:
        errors.append(f"Oversized source input: {relative}")
    blob = path.read_bytes()
    if blob[:2] == b"MZ" or blob[:4] == b"\x7fELF" or b"\0" in blob:
        errors.append(f"Binary payload: {relative}")
    if SENSITIVE.search(blob):
        errors.append(f"Credential/private-key pattern: {relative}")
    if suffix == ".rs":
        if re.search(rb"include_bytes!\s*\(", blob):
            errors.append(f"Embedded binary requires separate review: {relative}")
        for payload in re.findall(rb'include_str!\s*\(\s*"([^"]+)"', blob):
            if payload != b"../news/news.md" and payload not in OWN_EMBEDDED_SOURCE.get(relative, set()):
                errors.append(f"Unreviewed embedded text: {relative}: {payload.decode()}")
    return errors


def audit(root: Path, tracked_only: bool = False):
    errors, files = [], []
    if tracked_only:
        names = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"]).decode().split("\0")
        paths = [root / n for n in names if n]
    else:
        paths = []
        for directory, dirs, names in os.walk(root, followlinks=False):
            rel = Path(directory).relative_to(root)
            for name in list(dirs):
                p = Path(directory) / name
                if name in {".git", "build", "__pycache__"}:
                    dirs.remove(name)
                    continue
                if p.is_symlink() or getattr(p.lstat(), "st_file_attributes", 0) & 0x400:
                    errors.append(f"Reparse/symlink directory: {p.relative_to(root).as_posix()}")
                    dirs.remove(name)
                elif name.lower() in EXCLUDED_DIRS:
                    errors.append(f"Private or generated directory: {(rel/name).as_posix()}")
                    dirs.remove(name)
            paths.extend(Path(directory) / name for name in names if not name.endswith(".pyc"))
    for path in sorted(paths):
        relative = path.relative_to(root).as_posix()
        errors.extend(errors_for(path, relative))
        files.append({"path": relative, "size": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    return errors, files


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    ap.add_argument("--tracked-only", action="store_true")
    ap.add_argument("--public-release", action="store_true", help="Also require explicit reviewed playable-release approval")
    args = ap.parse_args()
    root = args.root.resolve()
    errors, files = audit(root, args.tracked_only)
    state = json.loads((root/"release-state.json").read_text(encoding="utf-8"))
    if args.public_release and (not state.get("public_release_ready") or state.get("blockers")):
        errors.append("Public playable release blocked: complete importer, licensing and binary/clean-install review first")
    result = {"scope": "source-only private staging", "public_release_ready": state.get("public_release_ready", False), "file_count": len(files), "errors": errors}
    print(json.dumps(result, indent=2))
    return 1 if errors else 0

if __name__ == "__main__":
    sys.exit(main())
