"""Complete weapon-data import from an existing original installation.

The accepted weapon matrix is separate from the character/motion/runtime matrix.
No developer export, authored numeric default, or asset payload is an input.
"""
import datetime
import importlib.util
import json
from pathlib import Path
import subprocess
import time
import sys

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import weapon_records
import weapon_matrix
import weapon_packages
import constructor_cache


def read_verified(receipt, key, native):
    path = Path(receipt[key]).resolve(strict=True)
    matching = [entry for entry in receipt['outputs'] if Path(entry['path']).resolve() == path]
    if len(matching) != 1 or native.digest(path, 'sha256') != matching[0]['sha256']:
        raise ValueError('Generated constructor output changed before consumption: ' + key)
    return json.loads(path.read_text(encoding='utf-8'))


def generate(game, cache, reader_tool, verify_tool, native):
    game, _, _, _ = native.verify_install(game)
    cache = Path(cache).resolve()
    source = ROOT.parents[1]
    if cache == game or cache.is_relative_to(game):
        raise ValueError('Weapon import cache must be outside the original installation')
    if cache.is_relative_to(source) and cache.relative_to(source).parts[:1] not in [('build',), ('cache',), ('state',)]:
        raise ValueError('Original weapon data must stay outside published source')
    verifier = Path(verify_tool).resolve(strict=True)
    if not verifier.is_file() or verifier.is_relative_to(game) or verifier.name.casefold() != 'mh-verify-weapon-import.exe':
        raise ValueError('Expected source-built mh-verify-weapon-import.exe outside original installation')
    cache.mkdir(parents=True, exist_ok=True)
    stage = cache / 'stages' / ('weapons-' + datetime.datetime.now(datetime.UTC).strftime('%Y%m%dT%H%M%S.%fZ'))
    stage.mkdir(parents=True)
    native_receipt = constructor_cache.generate_reviewed(game, stage / 'native')
    defaults = read_verified(native_receipt, 'defaults', native)
    review = read_verified(native_receipt, 'review', native)
    package_receipt = weapon_packages.generate(game, stage / 'packages', reader_tool, native)
    package_file = Path(package_receipt['outputs'][0]['path'])
    if native.digest(package_file, 'sha256') != package_receipt['outputs'][0]['sha256']:
        raise ValueError('Generated package records changed before consumption')
    packages = weapon_packages.validate(json.loads(package_file.read_text(encoding='utf-8')))
    for receipt in (native_receipt, package_receipt):
        if receipt['exe_sha1'] != native.EXE_SHA1 or receipt['pdb_sha1'] != native.PDB_SHA1:
            raise ValueError('Original native and package input identities differ')
    records, catalog_unknown = weapon_records.build_records(packages, defaults, review)
    record_file = stage / 'weapon_records.json'
    record_file.write_text(json.dumps(records, indent=2, sort_keys=True, allow_nan=False) + '\n', encoding='utf-8')
    matrix_receipt = weapon_matrix.generate(records, stage, native_receipt, package_receipt)
    data_source = source / 'core/crates/mordhau-core/src/data.rs'
    checked_paths = [record_file, data_source, *sorted(Path(matrix_receipt['matrix']).rglob('*.json'))]
    checked_hashes = {path: native.digest(path, 'sha256') for path in checked_paths}
    started = time.monotonic()
    process = subprocess.run([str(verifier), matrix_receipt['matrix'], str(record_file), str(data_source)],
                             cwd=stage, capture_output=True, text=True, timeout=120)
    log = {'exit_code': process.returncode, 'stdout': process.stdout, 'stderr': process.stderr,
           'duration_seconds': time.monotonic() - started}
    (stage / 'rust-verification-process.json').write_text(json.dumps(log, indent=2), encoding='utf-8')
    process.check_returncode()
    acceptance = json.loads(process.stdout)
    if acceptance.get('rust_consumers_accepted') is not True or acceptance.get('runtime_ready') is not False:
        raise ValueError('Rust weapon consumers did not establish acceptance')
    if acceptance.get('weapon_records') != len(records) or acceptance.get('weapon_records') != len(packages['weapons']):
        raise ValueError('Rust/source weapon record counts differ')
    # Recheck the original build and paks after both stages, before publishing.
    native.verify_install(game)
    if weapon_packages.pak_metadata(game) != package_receipt['original_paks']:
        raise ValueError('Original pak inputs changed during weapon import')
    read_verified(native_receipt, 'defaults', native)
    read_verified(native_receipt, 'review', native)
    for path, expected in checked_hashes.items():
        if native.digest(path, 'sha256') != expected:
            raise ValueError('Accepted consumer input changed during verification: ' + str(path))
    products = {'data_gen/weapons/records.json': record_file.read_bytes()}
    matrix_dir = Path(matrix_receipt['matrix'])
    for path in sorted(matrix_dir.rglob('*.json')):
        products['data_gen/weapons/spec/' + path.relative_to(matrix_dir).as_posix()] = path.read_bytes()
    products['data_gen/weapons/constructor-defaults.json'] = Path(native_receipt['defaults']).read_bytes()
    products['data_gen/weapons/constructor-review.json'] = Path(native_receipt['review']).read_bytes()
    outputs = native.commit_generated(cache, products)
    receipt = {'stage': 'weapon-data', 'complete': True, 'weapon_data_ready': True, 'runtime_ready': False,
               'exe_sha1': native.EXE_SHA1, 'pdb_sha1': native.PDB_SHA1, 'acceptance': acceptance,
               'native_stage': native_receipt, 'package_stage': package_receipt,
               'spreadsheet_database_stage': matrix_receipt, 'outputs': outputs,
               'verifier_sha256': native.digest(verifier, 'sha256'),
               'consumer_schema_sha256': native.digest(data_source, 'sha256'),
               'catalog_names_unavailable': catalog_unknown,
               'limits': ['Weapon values, attack slots, grip swaps and profile references are complete for the supported original build.',
                          'This weapon matrix does not replace the full character, motion, particle and physics setup.',
                          'Custom models, weapon meshes, UI and sounds are not replaced by this import.']}
    (stage / 'receipt.json').write_text(json.dumps(receipt, indent=2), encoding='utf-8')
    return receipt
