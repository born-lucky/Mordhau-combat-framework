"""Prepare or compile a BSD-provenanced adapter to the user's installed PhysX.

Default: source preparation only. --compile never installs, loads or executes a
DLL. Every invocation uses a fresh ignored build/local-cache directory.
"""
import argparse
import datetime
import difflib
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess

from compatibility import PIN, REPOSITORY, BASE_SHA256, adapt
from verify_callsites import check as check_callsites

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PREFIXES = (
    "README.md", "PhysX_3.4/Include", "PxShared/include",
    "PhysX_3.4/Source/Common/src", "PhysX_3.4/Source/PhysXExtensions/src",
    "PhysX_3.4/Source/PhysXMetaData/core/include",
    "PhysX_3.4/Source/PhysXMetaData/extensions/include",
    "PxShared/src/foundation/include",
)
README_SHA = "023428eee0b9bcbc7002131b6891f883426a61c0765164493241daa15c3864f4"
EXE_SHA1 = "dfe6f4fcb8e8a4603198025e13438b10062b0c4a"
DLL_SHA256 = {
    "PxFoundation_x64.dll": "7690c1572c74cf8029258fa1feeb00ba938d6a6c4b5fea34b9fb421224672dbe",
    "PhysX3Common_x64.dll": "1ed491817611f251cdea84d438ccd9d9ab8cb7cf42a1ecd7476834b7bbe53be4",
    "PhysX3_x64.dll": "09c50ddccf9b31dcf97e57505f0b4616c8a963b6d4502ce10d836f0c460a1b00",
    "PhysX3Cooking_x64.dll": "bff31ef9eaded496fecdb1e2f89f3108b9529d6e221e56d53c7321cf2575bcd4",
}


def digest(path, algorithm="sha256"):
    h = hashlib.new(algorithm)
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def metadata(path):
    return dict(path=str(path.resolve()), size=path.stat().st_size, sha256=digest(path))


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def git(repository, *args):
    return subprocess.check_output(["git", "--no-replace-objects", "-C", str(repository), *args])


def output_directory(args):
    build_root = (ROOT / "build/physics").resolve()
    if args.cache_root:
        build_root = args.cache_root.resolve()
        if build_root.is_relative_to(ROOT):
            raise ValueError("Consumer cache must be outside the source tree; otherwise use build/physics")
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    out = (args.output or build_root / stamp).resolve()
    if not out.is_relative_to(build_root) or out == build_root:
        raise ValueError("Output must be a new child of build/physics or the explicit cache root")
    if args.game_dir:
        game = args.game_dir.resolve()
        for target in (build_root, out):
            if target.is_relative_to(game) or game.is_relative_to(target):
                raise ValueError("Cache/output must not be inside or an ancestor of the installed game")
    out.mkdir(parents=True, exist_ok=False)
    return out


def acquire(out, repository):
    if repository is None:
        repository = out / "upstream.git"
        subprocess.run(["git", "init", "--bare", str(repository)], check=True)
        subprocess.run(["git", "-C", str(repository), "remote", "add", "origin", REPOSITORY], check=True)
        subprocess.run(["git", "-C", str(repository), "fetch", "--depth=1", "origin", PIN], check=True)
    repository = repository.resolve()
    if git(repository, "rev-parse", PIN + "^{commit}").decode().strip() != PIN:
        raise ValueError("Pinned official BSD commit missing")
    readme = git(repository, "show", PIN + ":README.md")
    if hashlib.sha256(readme).hexdigest() != README_SHA:
        raise ValueError("Pinned owner's BSD notice changed")
    # Never check out or trust HEAD/worktree files, including a local 2016 HEAD.
    entries = {}
    for row in git(repository, "ls-tree", "-r", "-z", PIN, "--", *PREFIXES).split(b"\0"):
        if not row:
            continue
        info, name = row.split(b"\t", 1)
        mode, kind, blob = info.decode().split()
        rel = PurePosixPath(name.decode("utf-8"))
        if mode != "100644" or kind != "blob" or rel.is_absolute() or ".." in rel.parts:
            raise ValueError("Nonregular/unsafe upstream entry")
        if str(rel) == "README.md" or rel.suffix in (".h", ".cpp", ".inl", ".inc"):
            entries[str(rel)] = blob
    if not set(BASE_SHA256).issubset(entries):
        raise ValueError("Patch target missing from pinned tree")
    staged = out / "bsd-source"
    staged.mkdir()
    # cat-file returns raw Git objects. `git archive` applies a local repository's
    # core.autocrlf policy; trusting that would confuse checkout bytes with the
    # pinned owner bytes. No checkout/attributes/filter program is invoked here.
    process = subprocess.Popen(["git", "--no-replace-objects", "-C", str(repository), "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    inventory = []
    notices = {readme.decode("utf-8").split("## Introduction", 1)[0].strip()}
    try:
        for relative, expected_blob in entries.items():
            process.stdin.write((expected_blob + "\n").encode("ascii"))
            process.stdin.flush()
            header = process.stdout.readline().decode("ascii").strip().split()
            if len(header) != 3 or header[:2] != [expected_blob, "blob"]:
                raise ValueError("Pinned object missing: " + relative)
            size = int(header[2])
            if size < 0 or size > 16 << 20:
                raise ValueError("Unexpected source file size")
            raw = process.stdout.read(size)
            if len(raw) != size or process.stdout.read(1) != b"\n":
                raise ValueError("Truncated pinned source object")
            blob = hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()
            if blob != expected_blob:
                raise ValueError("Pinned Git blob mismatch: " + relative)
            if relative != "README.md":
                text = raw.decode("utf-8")
                if "Redistribution and use in source and binary forms" not in text[:6000]:
                    raise ValueError("Unreviewed license preamble: " + relative)
                cut = re.search(r"^\s*#(?:ifndef|include|pragma)", text, re.MULTILINE)
                if cut:
                    notices.add(text[:cut.start()].strip())
            path = staged / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
            inventory.append(dict(path=relative, git_blob=blob, base_sha256=hashlib.sha256(raw).hexdigest()))
        process.stdin.close()
        if process.wait() != 0:
            raise RuntimeError("Pinned source acquisition failed")
    finally:
        process.stdout.close()
        if process.poll() is None:
            process.terminate()
            process.wait()
        if not process.stdin.closed:
            process.stdin.close()
    if len(inventory) != len(entries):
        raise ValueError("Incomplete pinned source acquisition")
    changes, diffs = [], []
    for row in inventory:
        path = staged / row["path"]
        if row["path"] in BASE_SHA256:
            original = path.read_bytes()
            adapted, edits = adapt(row["path"], original)
            path.write_bytes(adapted)
            changes.append(dict(path=row["path"], edits=edits))
            diffs.extend(difflib.unified_diff(original.decode().splitlines(True), adapted.decode().splitlines(True), fromfile="bsd/"+row["path"], tofile="installed-compat/"+row["path"]))
        row["staged_sha256"] = digest(path)
    (out / "compatibility.patch").write_text("".join(diffs), encoding="utf-8")
    (out / "NVIDIA-PHYSX-NOTICES.txt").write_text("\n\n---\n\n".join(sorted(notices)) + "\n", encoding="utf-8")
    save(out / "source-manifest.json", dict(repository=REPOSITORY, pin=PIN, readme_sha256=README_SHA, files=sorted(inventory, key=lambda r:r["path"]), modifications=changes))
    return staged, inventory


def pe64(path):
    with path.open("rb") as stream:
        dos = stream.read(64)
        if len(dos) != 64 or dos[:2] != b"MZ":
            raise ValueError("Missing original PE header")
        offset = struct.unpack_from("<I", dos, 60)[0]
        if offset < 64 or offset + 26 > path.stat().st_size:
            raise ValueError("Original PE header outside file")
        stream.seek(offset)
        nt = stream.read(26)
    if nt[:4] != b"PE\0\0" or struct.unpack_from("<H", nt, 4)[0] != 0x8664 or struct.unpack_from("<H", nt, 24)[0] != 0x20b:
        raise ValueError("Original DLL must be AMD64 PE32+")


def installed_files(game):
    if game is None:
        raise ValueError("--game-dir is required to compile; original DLLs are never downloaded")
    game = game.resolve(strict=True)
    exe = (game / "Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe").resolve(strict=True)
    if not exe.is_relative_to(game) or digest(exe, "sha1") != EXE_SHA1:
        raise ValueError("Unsupported original installation")
    pe64(exe)
    dlls = []
    for name, expected in DLL_SHA256.items():
        dll = (game / "Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015" / name).resolve(strict=True)
        if not dll.is_relative_to(game) or digest(dll) != expected:
            raise ValueError("Installed original DLL differs: " + name)
        pe64(dll)
        dlls.append(dll)
    return exe, dlls


def compiler_environment():
    if os.name != "nt":
        raise ValueError("Windows x64 MSVC build required")
    vswhere = Path(os.environ["ProgramFiles(x86)"]) / "Microsoft Visual Studio/Installer/vswhere.exe"
    vs = Path(subprocess.check_output([str(vswhere), "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], text=True).strip()).resolve()
    vcvars = vs / "VC/Auxiliary/Build/vcvars64.bat"
    if not vcvars.is_file() or any(c in str(vcvars) for c in '"%\r\n'):
        raise ValueError("MSVC environment unavailable")
    # Only the fixed, resolved Microsoft batch-file path goes through cmd.
    result = subprocess.check_output(f'cmd.exe /d /s /c ""{vcvars}" >nul && set"', text=True)
    env = {k.upper():v for k,v in os.environ.items()}
    for line in result.splitlines():
        if "=" in line and not line.startswith("="):
            key, value = line.split("=", 1)
            env[key.upper()] = value
    env.pop("CL", None)
    env.pop("_CL_", None)
    env["VSLANG"] = "1033"  # Make /showIncludes parsing deterministic.
    system_roots = [vs]
    for key in ("WINDOWSSDKDIR", "UNIVERSALCRTSDKDIR"):
        if env.get(key):
            system_roots.append(Path(env[key]).resolve())
    tools = {name:shutil.which(name + ".exe", path=env["PATH"]) for name in ("cl", "dumpbin", "lib")}
    if not all(tools.values()):
        raise ValueError("Missing MSVC tool")
    return env, tools, system_roots


def compile_bridge(out, staged, inventory, args):
    for row in inventory:
        if digest(staged / row["path"]) != row["staged_sha256"]:
            raise ValueError("Prepared source drift: " + row["path"])
    exe, dlls = installed_files(args.game_dir)
    env, tools, system_roots = compiler_environment()
    own = out / "authored-source"
    own.mkdir()
    authored = [HERE / n for n in ("abi_checks.cpp", "d6_checks.cpp", "mh_installed_physx.h", "build_bridge.py", "compatibility.py", "verify_callsites.py")]
    authored += [ROOT / "core/crates/mh-physics/native" / n for n in ("bridge.cpp", "validation_allocator.h")]
    source_before = [metadata(p) for p in authored]
    for path in authored:
        shutil.copyfile(path, own / path.name)
    for row in source_before:
        if digest(own / Path(row["path"]).name) != row["sha256"]:
            raise ValueError("Authored source drift during snapshot")
    (own / "typeinfo.h").write_text("#include <typeinfo>\n", encoding="utf-8")
    allowed = {p.resolve() for p in own.iterdir() if p.is_file()}
    allowed.update((staged / r["path"]).resolve() for r in inventory)
    before = source_before + [metadata(p) for p in own.iterdir()] + [metadata(staged / r["path"]) for r in inventory] + [metadata(exe)] + [metadata(p) for p in dlls]
    before += [metadata(out / name) for name in ("source-manifest.json", "compatibility.patch", "NVIDIA-PHYSX-NOTICES.txt")]
    px, shared = staged / "PhysX_3.4", staged / "PxShared"
    includes = [own, px / "Include", px / "Include/extensions", px / "Include/geometry", px / "Include/common", px / "Source/Common/src", px / "Source/PhysXExtensions/src", px / "Source/PhysXMetaData/core/include", px / "Source/PhysXMetaData/extensions/include", shared / "include", shared / "src/foundation/include"]
    commands, observed = [], set()

    def logged(command, phase):
        commands.append(dict(phase=phase, command=list(map(str, command))))
        save(out / "commands.json", commands)
        result = subprocess.run(list(map(str, command)), cwd=out, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (out / (phase + ".log")).write_text(result.stdout, encoding="utf-8")
        for line in result.stdout.splitlines():
            match = re.search(r"including file:\s*(.+)", line)
            if match:
                path = Path(match[1].strip()).resolve()
                if path in allowed:
                    observed.add(path)
                elif not any(path.is_relative_to(root) for root in system_roots):
                    raise ValueError("Unpinned include: " + str(path))
        print("\n".join(result.stdout.splitlines()[-8:]))
        if result.returncode != 0:
            raise subprocess.CalledProcessError(result.returncode, command)
        return result.stdout

    common = [tools["cl"], "/nologo", "/MD", "/EHsc", "/std:c++17", "/O2", "/DNDEBUG", "/DPX_SUPPORT_PVD=0", "/DWIN32", "/DWIN64", "/showIncludes", "/FAs"] + ["/I"+str(p) for p in includes]
    if args.guarded_validation:
        common.append("/DMH_PHYSX_VALIDATION=1")
    # Compiler evidence includes direct virtual-call displacements and descriptor
    # constructors. Fixtures are compiled only, never linked/executed.
    for name in ("abi_checks", "d6_checks"):
        logged(common + ["/c", "/Fa"+str(out / (name+".asm")), "/Fo"+str(out / (name+".obj")), own / (name+".cpp")], name)
    save(out / "abi-callsite-checks.json", check_callsites((out / "abi_checks.asm").read_text(encoding="utf-8")))
    libs = []
    for dll in dlls:
        exports = logged([tools["dumpbin"], "/exports", dll], dll.stem + "-exports")
        names = [m.group(1) for line in exports.splitlines() if (m:=re.match(r"\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)", line))]
        if not names:
            raise ValueError("Original DLL exports absent")
        definition = out / (dll.stem + ".def")
        definition.write_text("LIBRARY " + dll.name + "\nEXPORTS\n" + "\n".join(names) + "\n", encoding="utf-8")
        library = out / (dll.stem + ".lib")
        logged([tools["lib"], "/nologo", "/machine:x64", "/def:"+str(definition), "/out:"+str(library)], dll.stem + "-importlib")
        libs.append(library)
    sources = [own / "bridge.cpp"] + [px / "Source/PhysXExtensions/src" / name for name in ("ExtD6Joint.cpp", "ExtD6JointSolverPrep.cpp", "ExtJoint.cpp")]
    logged(common + ["/LD", "/Fa"+str(out)+os.sep, "/Fo"+str(out)+os.sep] + sources + ["/link", "/OUT:"+str(out / "mh_physx.dll")] + libs, "bridge")
    if not any(p.name == "PxPhysicsAPI.h" for p in observed):
        raise ValueError("Compiler include provenance missing")
    logged([tools["dumpbin"], "/imports", out / "mh_physx.dll"], "bridge-imports")
    changed = [r["path"] for r in before if digest(Path(r["path"])) != r["sha256"]]
    save(out / "provenance.json", dict(scope="COMPILED_ONLY_NOT_ACCEPTED_OR_INSTALLED", licensed_source_pin=PIN, installed_factory_version="0x03040000", guarded_validation=args.guarded_validation, inputs=before, observed_local_includes=[metadata(p) for p in sorted(observed)], tools=[metadata(Path(p)) for p in tools.values()], commands=commands, changed_inputs=changed, output=metadata(out / "mh_physx.dll")))
    if changed:
        raise ValueError("Source/installed inputs drifted during compilation")
    print("Compile only. Original ABI/solver/guarded body+joint+cooked/render validation remains required.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk-repository", type=Path, help="Optional local Git object store; HEAD is never used")
    parser.add_argument("--output", type=Path, help="Fresh child of build/physics or explicit cache-root")
    parser.add_argument("--cache-root", type=Path, help="Consumer-owned cache outside source tree")
    parser.add_argument("--game-dir", type=Path, help="Supported original installed game; required only for compile")
    parser.add_argument("--compile", action="store_true", help="Explicit MSVC compile gate; does not execute/install")
    parser.add_argument("--guarded-validation", action="store_true", help="Diagnostic allocator DLL; never install")
    args = parser.parse_args()
    out = output_directory(args)
    try:
        staged, inventory = acquire(out, args.sdk_repository)
        save(out / "preparation.json", dict(scope="PINNED_BSD_SOURCES_PREPARED_NO_COMPILER_OR_NATIVE_ENTRY", pin=PIN, file_count=len(inventory), manifest=metadata(out / "source-manifest.json"), patch=metadata(out / "compatibility.patch"), notices=metadata(out / "NVIDIA-PHYSX-NOTICES.txt")))
        if args.compile:
            compile_bridge(out, staged, inventory, args)
        print(out)
    except Exception as error:
        save(out / "failure.json", dict(error=str(error), output_preserved=True, installed=False, executed=False))
        raise


if __name__ == "__main__":
    main()
