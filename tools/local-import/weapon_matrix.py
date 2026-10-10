"""Generate the weapon/attack matrix through the existing spreadsheet validator."""
import ast
import contextlib
import importlib.util
import json
from pathlib import Path
import re
import sqlite3
import sys

ROOT = Path(__file__).parent
KIT = ROOT / 'matrix-source/tools/spec-kit'
sys.path.insert(0, str(KIT))
from speckit import book, matrix


def populate_module():
    path = ROOT / 'matrix-source/scripts/sheets_populate.py'
    spec = importlib.util.spec_from_file_location('local_weapon_sheet_bindings', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def rule_params():
    """Reuse the authored port's lookup/swap semantics, not a generated developer sheet."""
    source = ROOT / 'reader-source'
    text = (source / 'components/ue/records/combat_data.gd').read_text()
    body = text.split('static func attack_info_for(', 1)[1].split('\nstatic func ', 1)[0]
    slots = {}
    for moves, slot in re.findall(r'^\s*(CombatEnums\.Move\.[\w., ]+): return w\.(\w+)\s*$', body, re.M):
        for move in re.findall(r'CombatEnums\.Move\.(\w+)', moves):
            slots[move] = slot
    default = re.search(r'^\treturn w\.(\w+)\s*$', body, re.M)
    move_source = (source / 'game/combat/combat_enums.gd').read_text()
    move_text = re.search(r'enum Move \{([^}]+)\}', move_source).group(1)
    names = re.findall(r'(\w+)\s*=', move_text)
    if not default or not slots or not names:
        raise ValueError('Authored attack lookup unavailable')
    slots = {name: slots.get(name, default.group(1)) for name in names}
    motion_source = (source / 'game/combat/motion_system.gd').read_text()
    swaps = re.search(r'const _ATTACK_SWAPS := (\[\[.*?\]\])', motion_source, re.S)
    if not swaps:
        raise ValueError('Authored alternate-mode swaps unavailable')
    return {'attack_slot_by_move': slots, 'alt_mode_swaps': ast.literal_eval(swaps.group(1))}


def make_book(records, native_receipt, package_receipt):
    adapter = populate_module()
    b = book.Book({'weapon': 'WPN', 'attack': 'ATK'}, adapter.UNITS,
                  'Original supported MORDHAU build, locally imported')
    native = b.source('native', 'original-exe-pdb', json.dumps({
        'exe_sha1': native_receipt['exe_sha1'], 'pdb_sha1': native_receipt['pdb_sha1'],
        'constructor_outputs': [item['sha256'] for item in native_receipt['outputs']]}))
    b.source('generator', 'weapon_records.py', 'Reviewed native constructors + original Blueprint CDO ancestry')
    slots = adapter.ATTACK_SLOTS
    for wid, entry in sorted(records.items()):
        w = entry['weapon']
        source = b.source('pkg', w['class_path'], 'Original pak/CDO chain; package-stage output SHA256 ' + package_receipt['outputs'][0]['sha256'])
        eid = b.entity('weapon', wid, w.get('display_name', wid), source, record='WeaponData')
        adapter.put_record(b, 'weapon', eid, w, skip=set(slots) | {'unset'})
        b.set('weapon', eid, 'native_default_ue_fields', w['unset'], 'string_list')
        adapter.put_record(b, 'weapon', eid, entry['equip'],
                           skip={'path', 'native', 'b_has_alternate_mode', 'b_is_two_handed', 'b_second_is_two_handed'}, prefix='equip_')
        b.set('weapon', eid, 'profile', entry['profile'], 'string')
        # This standalone matrix carries package paths, rather than unresolved references
        # into an unrelated character/motion matrix. The source records retain both maps.
        b.set('weapon', eid, 'parry_motion_path', entry['parry_motion'], 'string')
        b.set('weapon', eid, 'motion_paths', entry['motions'], 'json')
        if 'alt_profile' in entry:
            b.set('weapon', eid, 'alt_profile', entry['alt_profile'], 'string')
            b.set('weapon', eid, 'alt_parry_motion_path', entry['alt_parry_motion'], 'string')
            b.set('weapon', eid, 'alt_motion_paths', entry['alt_motions'], 'json')
        for slot in slots:
            if slot not in w:
                continue
            attack = w[slot]
            aid = b.entity('attack', wid + '_' + slot.upper(), wid + ' ' + slot, source, parent=eid, record='AttackInfo')
            adapter.put_record(b, 'attack', aid, attack, skip={'unset'})
            b.set('attack', aid, 'native_default_ue_fields', attack['unset'], 'string_list')
            b.set('weapon', eid, 'attack_' + slot, aid, 'ref', 'attack')
    params = rule_params()
    for rid, rtype, description, functions, key, refs in adapter.CURATED:
        if rid not in ('RULE_ATTACK_SLOT_BY_MOVE', 'RULE_ALT_MODE_ATTACK_SWAP'):
            continue
        b.rules[rid] = {'rule_id': rid, 'rule_type': rtype, 'name': rid, 'description': description,
                        'params': json.dumps({key: params[key]}), 'field_refs': ';'.join(refs),
                        'source': '; '.join(functions), 'implemented_by': 'mh-spec::Spec::attack_for_move',
                        'area': 'combat', 'test_id': '', 'status': 'ported', 'tiers': 'b',
                        'tier_note': 'Authored port semantics; values are not copied from a developer matrix'}
    for field in b.fields.values():
        field['source'] = native + '; original package CDO ancestry'
        field['description'] = 'Original local weapon data; no authored gameplay default fallback'
    return b


def generate(records, stage, native_receipt, package_receipt):
    stage = Path(stage)
    b = make_book(records, native_receipt, package_receipt)
    headers, rows = book.sheets(b)
    workbook = stage / 'weapon_spec.xlsx'
    book.write_book(workbook, headers, rows)
    matrix.configure(root=ROOT.parents[1], fmt='mordhau-spec/1')
    errors, files = matrix.outputs(matrix.read(workbook))
    if errors:
        raise ValueError('Weapon workbook rejected: ' + '\n'.join(errors[:20]))
    out = stage / 'weapon_spec'
    for name, contents in files.items():
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('x', encoding='utf-8', newline='\n') as f:
            f.write(contents)
    # The database is a generated view of the same validated workbook values.
    database = stage / 'weapon_spec.sqlite'
    with contextlib.closing(sqlite3.connect(database)) as db:
        db.execute('CREATE TABLE entities (id TEXT PRIMARY KEY, kind TEXT NOT NULL, source_id TEXT NOT NULL, record TEXT NOT NULL)')
        db.execute('CREATE TABLE fields (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, type TEXT NOT NULL)')
        db.execute('CREATE TABLE entity_values (entity_id TEXT NOT NULL REFERENCES entities(id), field_id TEXT NOT NULL REFERENCES fields(id), value_json TEXT NOT NULL, PRIMARY KEY(entity_id,field_id))')
        db.execute('PRAGMA foreign_keys=ON')
        for eid, e in b.entities.items():
            db.execute('INSERT INTO entities VALUES (?,?,?,?)', (eid, e['entity_type'], e['source_id'], e['record']))
        for fid, f in b.fields.items():
            db.execute('INSERT INTO fields VALUES (?,?,?,?)', (fid, f['entity_type'], f['name'], f['type']))
        for kind in b.values:
            for eid, fields in b.values[kind].items():
                for fid, value in fields.items():
                    db.execute('INSERT INTO entity_values VALUES (?,?,?)', (eid, fid, json.dumps(value, allow_nan=False)))
        if db.execute('PRAGMA foreign_key_check').fetchall():
            raise ValueError('Generated weapon database has unresolved references')
        db.commit()
    return {'workbook': str(workbook), 'matrix': str(out), 'database': str(database),
            'counts': json.loads(files['index.json'])['counts']}
