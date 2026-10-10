"""Locate existing original inputs; never install, launch or modify MORDHAU."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys

SHIPPING = Path('Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe')


def root_from_exe(exe):
    exe = Path(exe).resolve(strict=True)
    if not exe.is_file() or tuple(p.casefold() for p in exe.parts[-4:]) != tuple(p.casefold() for p in SHIPPING.parts):
        raise ValueError('Select the original Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe in its existing installation')
    return exe.parents[3]


def running_executables():
    if sys.platform != 'win32':
        return []
    script = """$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false)
@(Get-CimInstance Win32_Process -Filter \"Name='Mordhau-Win64-Shipping.exe'\" |
    Select-Object ProcessId,ExecutablePath) | ConvertTo-Json -Compress
"""
    result = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', script],
                            check=True, capture_output=True, encoding='utf-8', timeout=20)
    records = json.loads(result.stdout.strip() or '[]')
    if isinstance(records, dict):
        records = [records]
    if not isinstance(records, list) or any(not isinstance(r, dict) for r in records):
        raise ValueError('Unexpected running MORDHAU process response')
    if any(not r.get('ExecutablePath') for r in records):
        raise ValueError('Windows cannot read the running MORDHAU executable path; supply --game-exe PATH')
    return [Path(r['ExecutablePath']) for r in records]


def read_vdf(path):
    """Read Steam's quoted key/value format, retaining nesting and backslashes."""
    text = Path(path).read_text(encoding='utf-8-sig')
    if len(text) > 2 << 20:
        raise ValueError('Steam metadata exceeds supported size')
    tokens = []
    for match in re.finditer(r'//[^\n]*|\s+|"((?:\\.|[^"\\])*)"|([{}])|(\S+)', text):
        if match.group(1) is not None:
            tokens.append(('string', re.sub(r'\\([\\"])', r'\1', match.group(1))))
        elif match.group(2):
            tokens.append((match.group(2), match.group(2)))
        elif match.group(3):
            raise ValueError('Unsupported unquoted Steam metadata token')
    pos = 0

    def read_object(nested=False, depth=0):
        nonlocal pos
        if depth > 32:
            raise ValueError('Steam metadata nesting exceeds supported depth')
        values = {}
        while pos < len(tokens):
            kind, key = tokens[pos]
            pos += 1
            if kind == '}' and nested:
                return values
            if kind != 'string' or key in values or pos >= len(tokens):
                raise ValueError('Malformed or duplicate Steam metadata key')
            kind, value = tokens[pos]
            pos += 1
            if kind == '{':
                values[key] = read_object(True, depth + 1)
            elif kind == 'string':
                values[key] = value
            else:
                raise ValueError('Malformed Steam metadata value')
        if nested:
            raise ValueError('Unclosed Steam metadata object')
        return values

    return read_object()


def steam_roots():
    roots = []
    if sys.platform == 'win32':
        import winreg
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r'Software\Valve\Steam') as key:
                roots.append(Path(winreg.QueryValueEx(key, 'SteamPath')[0]))
        except OSError:
            pass
        for name in ('ProgramFiles(x86)', 'ProgramFiles'):
            if os.environ.get(name):
                roots.append(Path(os.environ[name]) / 'Steam')
    return unique_paths(roots)


def unique_paths(paths):
    return list({str(Path(p).resolve()).casefold(): Path(p).resolve() for p in paths}.values())


def installed_executables():
    libraries = []
    for root in steam_roots():
        libraries.append(root)
        folders = root / 'steamapps/libraryfolders.vdf'
        if folders.is_file():
            values = read_vdf(folders).get('libraryfolders', {})
            if not isinstance(values, dict):
                raise ValueError('Malformed Steam library list')
            for key, value in values.items():
                if key.isdigit():
                    path = value.get('path') if isinstance(value, dict) else value
                    if not isinstance(path, str) or not path:
                        raise ValueError('Malformed Steam library path')
                    libraries.append(Path(path))
    result = []
    for library in unique_paths(libraries):
        manifest = library / 'steamapps/appmanifest_629760.acf'
        if not manifest.is_file():
            continue
        state = read_vdf(manifest).get('AppState', {})
        if not isinstance(state, dict) or state.get('appid') != '629760':
            raise ValueError('MORDHAU Steam manifest has an unexpected app identity')
        name = state.get('installdir', '')
        if not isinstance(name, str) or not name or name in ('.', '..') or any(c in name for c in '/\\:'):
            raise ValueError('Invalid MORDHAU installation directory in Steam manifest')
        exe = library / 'steamapps/common' / name / SHIPPING
        if exe.is_file():
            result.append(exe)
    return unique_paths(result)


def resolve_install(game_dir=None, game_exe=None):
    if game_dir is not None and game_exe is not None:
        raise ValueError('Select either --game-dir or --game-exe')
    if game_dir is not None:
        root = Path(game_dir).resolve()
        return {'method': 'selected-directory', 'game_dir': str(root), 'game_exe': str(root / SHIPPING)}
    if game_exe is not None:
        root = root_from_exe(game_exe)
        return {'method': 'selected-executable', 'game_dir': str(root), 'game_exe': str(Path(game_exe).resolve())}
    candidates = unique_paths(running_executables())
    method = 'running-mordhau'
    if not candidates:
        candidates = installed_executables()
        method = 'existing-steam-installation'
    if len(candidates) > 1:
        raise ValueError('Multiple MORDHAU installations found; select --game-exe PATH: ' + ', '.join(map(str, candidates)))
    if not candidates:
        raise ValueError('No running MORDHAU or existing Steam installation found; supply --game-exe PATH or --game-dir PATH')
    exe = candidates[0]
    return {'method': method, 'game_dir': str(root_from_exe(exe)), 'game_exe': str(exe)}
