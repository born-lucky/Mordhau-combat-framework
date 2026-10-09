"""Install reviewed free combat recordings into the framework's own local audio bank.

Python 3.11+ and 7-Zip (for the small qubodup archive); no original game files are read.
"""
from __future__ import annotations
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def safe_name(name: str) -> str:
    name = name.replace("\\", "/")
    p = PurePosixPath(name)
    if p.is_absolute() or any(part in {"..", "."} or ":" in part for part in p.parts):
        raise ValueError(f"Unsafe archive member: {name}")
    return p.as_posix()

def archive(cache: Path, key: str, spec: dict) -> Path:
    suffix = ".7z" if spec["url"].endswith(".7z") else ".zip"
    path = cache / (key + suffix)
    if not path.exists():
        request = urllib.request.Request(spec["url"], headers={"User-Agent": "MordhauCombatFramework-free-audio/1"})
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read(64 * 1024 * 1024 + 1)
        if len(data) > 64 * 1024 * 1024:
            raise ValueError(f"Oversized archive: {key}")
        if hashlib.sha256(data).hexdigest() != spec["sha256"]:
            raise ValueError(f"Archive digest changed: {key}; review source before installing")
        path.write_bytes(data)
    if hashlib.sha256(path.read_bytes()).hexdigest() != spec["sha256"]:
        raise ValueError(f"Archive digest mismatch: {key}")
    return path

def members(path: Path, sevenzip: str | None) -> list[str]:
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as z:
            return [safe_name(i.filename) for i in z.infolist() if not i.is_dir()]
    if not sevenzip:
        raise RuntimeError("Install 7-Zip or pass --sevenzip /path/to/7z for the reviewed impact archive")
    output = subprocess.check_output([sevenzip, "l", "-slt", "--", str(path)], text=True)
    entries = output.split("----------", 1)[1]
    return [safe_name(line.removeprefix("Path = ")) for line in entries.splitlines() if line.startswith("Path = ")]

def read_member(path: Path, name: str, sevenzip: str | None) -> bytes:
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as z:
            info = next(i for i in z.infolist() if safe_name(i.filename) == name)
            if info.file_size > 16 * 1024 * 1024:
                raise ValueError(f"Oversized recording: {name}")
            return z.read(info)
    # Read exactly one reviewed member to stdout: no archive path can write outside our bank.
    data = subprocess.check_output([sevenzip, "e", "-so", "--", str(path), name.replace("/", "\\")])
    if len(data) > 16 * 1024 * 1024:
        raise ValueError(f"Oversized recording: {name}")
    return data

def install(destination: Path, cache: Path, sevenzip: str | None) -> dict:
    recipe = tomllib.loads((ROOT / "audio/sources.toml").read_text())
    destination.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    packages = {key: archive(cache, key, spec) for key, spec in recipe["archives"].items()}
    names = {key: members(path, sevenzip) for key, path in packages.items()}
    result = {"version": 1, "roles": {}, "samples": {}, "sources": recipe["archives"]}
    for role, patterns in recipe["roles"].items():
        files = []
        for pattern in patterns:
            key, pattern = pattern.split(":", 1)
            selected = sorted(n for n in names[key] if fnmatch.fnmatchcase(n, pattern))
            if not selected:
                raise ValueError(f"No recordings matched {key}:{pattern}")
            for name in selected:
                filename = safe_name(f"{key}/{name}")
                target = destination.joinpath(*PurePosixPath(filename).parts)
                if not target.resolve().is_relative_to(destination.resolve()):
                    raise ValueError(f"Recording path escapes bank: {filename}")
                target.parent.mkdir(parents=True, exist_ok=True)
                blob = read_member(packages[key], name, sevenzip)
                if not blob.startswith((b"OggS", b"RIFF")):
                    raise ValueError(f"Unsupported audio format: {filename}")
                target.write_bytes(blob)
                result["samples"][filename] = {"sha256": hashlib.sha256(blob).hexdigest(), "source": key, "archive_member": name}
                files.append(filename)
        result["roles"][role] = files
    # Publish manifest last; never leave a partially installed bank marked ready.
    manifest = destination / "bank.json"
    temp = destination / "bank.json.installing"
    temp.write_text(json.dumps(result, indent=2) + "\n")
    temp.replace(manifest)
    return {"bank": str(manifest.resolve()), "roles": len(result["roles"]), "samples": len(result["samples"])}

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    config = Path(os.getenv("MH_CONFIG_DIR", str(Path(os.getenv("LOCALAPPDATA", "")) / "MordhauCombatFramework/Saved/Config/WindowsClient")))
    parser.add_argument("--destination", type=Path, default=config / "audio")
    parser.add_argument("--cache", type=Path, default=ROOT / "build/audio-sources")
    parser.add_argument("--sevenzip", default=shutil.which("7z") or shutil.which("7zz"))
    args = parser.parse_args()
    print(json.dumps(install(args.destination.resolve(), args.cache.resolve(), args.sevenzip), indent=2))

if __name__ == "__main__":
    main()
