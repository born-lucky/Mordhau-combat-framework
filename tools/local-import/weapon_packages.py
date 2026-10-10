"""Import serialized weapon Blueprint/CDO chains from the user's original paks.

Requires the source-built mh-weapon-packages reader; generated records have no
native constructor defaults and never establish complete runtime readiness.
"""
import argparse
import collections
import datetime
import importlib.util
import json
import math
import os
from pathlib import Path
import subprocess
import time

ROOT_COUNTS = {'MordhauWeapon': 241, 'MordhauShield': 27, 'FistsWeapon': 7, 'KickWeapon': 4}


def package_path(value):
    return (isinstance(value, str) and value.startswith('Mordhau/Content/')
            and '\\' not in value and ':' not in value
            and all(part not in ('', '.', '..') for part in value.split('/')))


def validate(payload):
    if not isinstance(payload, dict) or payload.get('schema') != 1 or payload.get('source') != 'original-paks':
        raise ValueError('Unsupported original weapon package schema')
    if payload.get('runtime_ready') is not False:
        raise ValueError('Serialized package reader cannot establish runtime readiness')
    weapons = payload.get('weapons')
    records = payload.get('records')
    if not isinstance(weapons, list) or not weapons or any(not package_path(w) for w in weapons) or weapons != sorted(set(weapons)):
        raise ValueError('Missing, unsorted or duplicate original weapon identities')
    if not isinstance(records, dict) or any(not package_path(p) for p in records):
        raise ValueError('Invalid original package record identities')
    if payload.get('errors') != []:
        raise ValueError('Original combat package reader reported errors: ' + str(payload.get('errors'))[:2000])
    def check_values(value):
        if isinstance(value, dict):
            if '__unsupported__' in value:
                raise ValueError('Undecoded original property marker')
            for child in value.values():
                check_values(child)
        elif isinstance(value, list):
            for child in value:
                check_values(child)
        elif isinstance(value, float) and not math.isfinite(value):
            raise ValueError('Nonfinite original property value')
    check_values(payload)
    counts = collections.Counter()
    for path, record in records.items():
        if not isinstance(record, dict) or record.get('package') != path:
            raise ValueError('Original package identity differs from record')
        if record.get('native_defaults_included') is not False or record.get('decode_errors') != []:
            raise ValueError('Unreviewed native defaults or undecoded original properties: ' + path)
        chain = record.get('chain_child_first')
        if not isinstance(chain, list) or not chain or chain[0] != path or any(not package_path(p) for p in chain) or len(chain) != len(set(chain)):
            raise ValueError('Missing, cyclic or invalid original Blueprint chain: ' + path)
        if not isinstance(record.get('serialized_defaults'), dict) or not isinstance(record.get('maps_merged'), dict):
            raise ValueError('Missing original serialized defaults/maps: ' + path)
        packages = record.get('original_packages')
        if not isinstance(packages, list) or [p.get('package') for p in packages if isinstance(p, dict)] != list(reversed(chain)):
            raise ValueError('Original package provenance does not match inheritance chain: ' + path)
        for package in packages:
            exports = package.get('exports')
            if not isinstance(exports, list) or not exports or any(not isinstance(e, dict) or not isinstance(e.get('export_index'), int) or e['export_index'] < 0 or not isinstance(e.get('record'), dict) for e in exports):
                raise ValueError('Invalid original export provenance: ' + path)
            if record.get('native_root') and not any(e['record'].get('Name', '').startswith('Default__') for e in exports):
                raise ValueError('Missing class default object in original ancestor: ' + path)
        if path in weapons:
            counts[record.get('native_root')] += 1
    if any(not package_path(w) or w not in records for w in weapons) or dict(counts) != ROOT_COUNTS:
        raise ValueError('Supported original weapon class census differs: ' + str(dict(counts)))
    return payload


def pak_metadata(game):
    files = sorted((game / 'Mordhau/Content/Paks').glob('*.pak'))
    if not files:
        raise ValueError('Original installation has no pak files')
    return [{'path': str(p.relative_to(game)), 'bytes': p.stat().st_size,
             'mtime_ns': p.stat().st_mtime_ns} for p in files]


def generate(game, cache, reader_tool, native, include_packages=()):
    game, exe, pdb, _ = native.verify_install(game)
    game = Path(game).resolve()
    cache = Path(cache).resolve()
    source = Path(__file__).resolve().parents[2]
    if cache == game or cache.is_relative_to(game):
        raise ValueError('Weapon records must be generated outside the original installation')
    if cache == source or cache.is_relative_to(source) and cache.relative_to(source).parts[:1] not in [('build',), ('cache',), ('state',)]:
        raise ValueError('Original weapon records cannot be generated into published source')
    reader = Path(reader_tool).resolve(strict=True)
    if not reader.is_file() or reader.is_relative_to(game) or reader.name.casefold() != 'mh-weapon-packages.exe':
        raise ValueError('Expected separately source-built mh-weapon-packages.exe outside the game installation')
    include_packages = sorted(set(include_packages))
    if any(not package_path(p) for p in include_packages):
        raise ValueError('Invalid included package identity')
    original_paks = pak_metadata(game)
    stage = cache / 'stages' / ('weapon-packages-' + datetime.datetime.now(datetime.UTC).strftime('%Y%m%dT%H%M%S.%fZ'))
    stage.mkdir(parents=True)
    output = stage / 'weapon-packages.json'
    command = [str(reader), str(output)]
    for package in include_packages:
        command += ['--include', package]
    started = time.monotonic()
    result = subprocess.run(command, env=dict(os.environ, MORDHAU_DIR=str(game)), cwd=stage,
                            capture_output=True, text=True, timeout=300)
    log = {'command': command, 'exit_code': result.returncode, 'stdout': result.stdout,
           'stderr': result.stderr, 'duration_seconds': time.monotonic() - started}
    (stage / 'process.json').write_text(json.dumps(log, indent=2), encoding='utf-8')
    result.check_returncode()
    if not output.is_file() or output.stat().st_size > 128 << 20:
        raise ValueError('Missing or oversized original weapon package output')
    payload = validate(json.loads(output.read_text(encoding='utf-8')))
    if pak_metadata(game) != original_paks:
        raise ValueError('Original pak files changed while reading weapon records; retry after the game update finishes')
    outputs = native.commit_generated(cache, {'data_gen/import/weapon_packages.json': output.read_bytes()})
    counts = dict(collections.Counter(payload['records'][w]['native_root'] for w in payload['weapons']))
    helper = Path(__file__).with_name('native-tools') / 'WeaponPackages.rs'
    receipt = {'stage': 'original-weapon-packages', 'complete': True, 'runtime_ready': False,
               'exe_sha1': native.EXE_SHA1, 'pdb_sha1': native.PDB_SHA1,
               'reader_sha256': native.digest(reader, 'sha256'), 'source_sha256': native.digest(helper, 'sha256'),
               'output_sha256': native.digest(output, 'sha256'), 'original_paks': original_paks,
               'original_entry_hashes_verified': True, 'weapons': len(payload['weapons']),
               'root_counts': counts, 'package_records': len(payload['records']),
               'native_roots': sorted({r['native_root'] for r in payload['records'].values() if r['native_root']}),
               'excluded_references': payload.get('excluded_references', []), 'outputs': outputs,
               'limits': ['Serialized Blueprint/CDO deltas and combat curves only; native constructor defaults must be generated separately.',
                          'Animation/audio/mesh references are retained without copying those asset payloads.']}
    (stage / 'receipt.json').write_text(json.dumps(receipt, indent=2), encoding='utf-8')
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir', type=Path, required=True)
    parser.add_argument('--cache-dir', type=Path, required=True)
    parser.add_argument('--weapon-tool', type=Path, required=True)
    parser.add_argument('--include-package', action='append', default=[])
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('native_cache', Path(__file__).with_name('native_cache.py'))
    native = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(native)
    try:
        result = generate(args.game_dir, args.cache_dir, args.weapon_tool, native, args.include_package)
    except (OSError, ValueError, ImportError, AssertionError, subprocess.SubprocessError) as error:
        result = {'runtime_ready': False, 'error': str(error)}
    print(json.dumps(result, indent=2))
    return 2


if __name__ == '__main__':
    raise SystemExit(main())
