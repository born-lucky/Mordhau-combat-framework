"""Strict weapon records from reviewed native fields plus original Blueprint deltas.

Authored GDScript supplies field bindings/types, never gameplay default values.
The package scanner and constructor decoder supply every imported value.
"""
import copy
import math
from pathlib import Path
import re

SOURCE = Path(__file__).with_name('reader-source')
WEAPON_SOURCE = SOURCE / 'game/combat/weapon_data.gd'
ATTACK_SOURCE = SOURCE / 'game/combat/attack_info.gd'
EQUIP_SOURCE = SOURCE / 'components/ue/records/equipment_def.gd'
NATIVE_CLASSES = {'MordhauWeapon': 'AMordhauWeapon', 'MordhauShield': 'AMordhauShield',
                  'FistsWeapon': 'AFistsWeapon', 'KickWeapon': 'AKickWeapon'}
OPTIONAL_CATALOG = {'EquipmentName'}
BLUEPRINT_ONLY = {'DamageMultiplier', 'AttackSpeedModifier', 'MaxComboCount'}
TYPE_IDS = {'bool': 1, 'int': 2, 'float': 3, 'string': 4, 'vec2': 5, 'vec3': 9,
            'json': 28, 'float_list': 32, 'string_list': 34}


def binding(source, name):
    text = Path(source).read_text(encoding='utf-8')
    match = re.search(r'const ' + re.escape(name) + r'\s*:=\s*\{(.*?)\n\}', text, re.S)
    if not match:
        raise ValueError('Authored binding unavailable: ' + name)
    pairs = re.findall(r'"([A-Za-z0-9_]+)"\s*:\s*"([A-Za-z0-9_]+)"', match[1])
    if not pairs or len(dict(pairs)) != len(pairs):
        raise ValueError('Missing/duplicate authored bindings: ' + name)
    return dict(pairs)


def field_types(source):
    result = {}
    for line in Path(source).read_text(encoding='utf-8').splitlines():
        match = re.match(r'(?:@export\s+)?var\s+(\w+)\s*(?::\s*([\w\[\]]+)\s*)?(?:[:=]+\s*(.*))?$', line.split('#')[0].strip())
        if not match:
            continue
        name, declared, expr = match.groups()
        expr = (expr or '').strip()
        typ = declared or expr.split('(')[0].split('.')[0]
        if typ in ('Vector2', 'Vector2i'):
            kind = 'vec2'
        elif typ in ('Vector3', 'Vector3i'):
            kind = 'vec3'
        elif typ in ('PackedFloat32Array', 'PackedFloat64Array'):
            kind = 'float_list'
        elif typ == 'PackedStringArray':
            kind = 'string_list'
        elif expr in ('true', 'false') or typ == 'bool':
            kind = 'bool'
        elif expr.startswith('"') or typ == 'String':
            kind = 'string'
        elif typ == 'int' or re.fullmatch(r'-?\d+', expr):
            kind = 'int'
        elif typ == 'float' or re.fullmatch(r'-?\d+\.\d*', expr):
            kind = 'float'
        else:
            kind = 'json'
        result[name] = kind
    return result


def merge(parent, child):
    if isinstance(parent, dict) and isinstance(child, dict):
        result = copy.deepcopy(parent)
        for key, value in child.items():
            result[key] = merge(result.get(key), value)
        return result
    return copy.deepcopy(child)


def package_path(value, what):
    if value is None or value == {}:
        return ''
    if not isinstance(value, dict) or not isinstance(value.get('ObjectPath'), str):
        raise ValueError(what + ': invalid original object reference')
    return re.sub(r'\.\d+$', '', value['ObjectPath'])


def finite_number(value, what):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(what + ': expected finite number')
    return float(value)


def convert(value, kind, what):
    if kind == 'bool':
        if isinstance(value, bool):
            return value
        if isinstance(value, int) and value in (0, 1):
            return bool(value)
    elif kind == 'int':
        if isinstance(value, int) and not isinstance(value, bool):
            return value
    elif kind == 'float':
        return finite_number(value, what)
    elif kind in ('vec2', 'vec3'):
        keys = ('X', 'Y') if kind == 'vec2' else ('X', 'Y', 'Z')
        if kind == 'vec3' and isinstance(value, dict) and 'Pitch' in value:
            keys = ('Pitch', 'Yaw', 'Roll')
        if isinstance(value, dict) and all(k in value for k in keys):
            return [finite_number(value[k], what + '.' + k) for k in keys]
        if isinstance(value, list) and len(value) == len(keys):
            return [finite_number(v, what) for v in value]
    elif kind == 'float_list' and isinstance(value, list):
        return [finite_number(v, what) for v in value]
    elif kind == 'string_list' and isinstance(value, list):
        out = []
        for v in value:
            if isinstance(v, dict) and isinstance(v.get('Name'), str):
                v = v['Name']
            if not isinstance(v, str):
                raise ValueError(what + ': invalid original name')
            out.append(v)
        return out
    elif kind == 'string':
        if value is None or value == {}:
            return ''  # accepted native null pointer or serialized null, not a missing field
        if isinstance(value, str):
            return value
        if isinstance(value, dict):
            for key in ('ObjectPath', 'LocalizedString', 'SourceString', 'CultureInvariantString', 'Name'):
                if key in value and isinstance(value[key], str):
                    return value[key]
        # An unsupported native integer or opaque FText is never converted to an invented label.
    elif kind == 'json' and isinstance(value, (dict, list)):
        return copy.deepcopy(value)
    raise ValueError(what + ': original value does not fit ' + kind)


def typed_record(class_name, values, kinds):
    return {'__class': class_name, '__types': {k: TYPE_IDS[kinds[k]] for k in values}, **values}


def attack_record(native, serialized, maps, kinds, what):
    if not isinstance(native, dict) or not isinstance(serialized, dict):
        raise ValueError(what + ': missing original native/Blueprint attack struct')
    merged = merge(native, serialized)
    values = {}
    for ue, field in maps.items():
        if ue not in merged:
            raise ValueError(what + ': original attack field unavailable: ' + ue)
        values[field] = convert(merged[ue], kinds[field], what + '.' + ue)
    for field in ('damage', 'head_bonus', 'leg_bonus'):
        if len(values[field]) != 4:
            raise ValueError(what + ': expected four original armor-tier values: ' + field)
    values['unset'] = sorted(k for k in maps if k not in serialized)
    values['unmapped'] = sorted(k for k in serialized if k not in maps)
    return typed_record('AttackInfo', values, {**kinds, 'unmapped': 'string_list', 'unset': 'string_list'})


def equipment_record(merged, path, native, shield):
    methods = {'b': 'bool', 'f': 'float', 'v3': 'vec3', 'rot': 'vec3', 'obj': 'string'}
    bindings = re.findall(r'^\s*(\w+) = r\.(b|f|v3|rot|obj)\("(\w+)"\)', EQUIP_SOURCE.read_text(), re.M)
    values = {'path': path, 'native': native, 'is_weapon': True, 'is_shield': shield}
    kinds = {'path': 'string', 'native': 'string', 'is_weapon': 'bool', 'is_shield': 'bool'}
    for field, method, ue in bindings:
        kinds[field] = methods[method]
        if ue == 'bAllowShieldWall' and not shield:
            values[field] = False  # property does not exist on a non-shield; IsA gate in EquipmentDef.read
            continue
        if ue not in merged:
            raise ValueError(path + ': original equipment field unavailable: ' + ue)
        values[field] = package_path(merged[ue], path + '.' + ue) if method == 'obj' else convert(merged[ue], kinds[field], path + '.' + ue)
    return typed_record('EquipmentDef', values, kinds)


def profile_data(path, packages):
    if not path:
        return {'profile': '', 'parry_motion': '', 'motions': {}}
    if path not in packages:
        raise ValueError('Original animation profile unavailable: ' + path)
    row = packages[path]
    moves = {'RightStrike': 'RIGHT_STRIKE', 'LeftStrike': 'LEFT_STRIKE', 'Stab': 'STAB',
             'AltStab': 'ALT_STAB', 'Kick': 'KICK', 'Bash': 'BASH', 'Couch': 'COUCH', 'Ranged': 'RANGED'}
    motions = {}
    for key, value in row.get('maps_merged', {}).get('Attacks', {}).items():
        move = key.split('::')[-1]
        if move not in moves:
            raise ValueError('Original profile contains an unknown attack move: ' + key)
        motion = package_path(value, path + '.Attacks.' + key)
        if motion:
            if motion not in packages:
                raise ValueError('Original motion package unavailable: ' + motion)
            motions[moves[move]] = motion
    parry = package_path(row['serialized_defaults'].get('ParryMotion'), path + '.ParryMotion')
    if parry and parry not in packages:
        raise ValueError('Original parry motion package unavailable: ' + parry)
    return {'profile': path, 'parry_motion': parry, 'motions': motions}


def component(record, name):
    result = {}
    for package in record['original_packages']:
        for export in package['exports']:
            row = export['record']
            if row.get('Name') == name:
                result = merge(result, row.get('Properties', {}))
    return result


def validate_native(native_data, native_review):
    if native_data.get('schema_version') != 1 or native_review.get('schema_version') != 1:
        raise ValueError('Unsupported reviewed native constructor format')
    for key in ('exe_sha1', 'pdb_sha1'):
        if not native_data.get(key) or native_data[key] != native_review.get(key):
            raise ValueError('Native constructor/review original identity mismatch: ' + key)
    if native_review.get('zero_allocation', {}).get('memset_import') != 'memset':
        raise ValueError('Original UObject allocation proof unavailable')
    for cls, values in native_data['classes'].items():
        rows = native_review['classes'].get(cls)
        if not isinstance(rows, list):
            raise ValueError('Reviewed native field coverage unavailable: ' + cls)
        accepted = {row['field'] for row in rows if row.get('accepted') is True}
        rejected = {row['field'] for row in rows if row.get('accepted') is not True}
        def check(value, path):
            if isinstance(value, dict) and value:
                for key, child in value.items():
                    check(child, path + '.' + key if path else key)
            elif path not in accepted or path in rejected:
                raise ValueError('Unreviewed native field cannot supply a default: ' + cls + '.' + path)
        for key, value in values.items():
            check(value, key)


def build_records(package_data, native_data, native_review):
    validate_native(native_data, native_review)
    if package_data.get('errors'):
        raise ValueError('Original package scan reported errors')
    classes = native_data['classes']
    packages = package_data['records']
    weapon_map = binding(WEAPON_SOURCE, 'MAP')
    attack_map = binding(ATTACK_SOURCE, 'MAP')
    attacks = binding(WEAPON_SOURCE, 'ATTACKS')
    weapon_kinds = field_types(WEAPON_SOURCE)
    attack_kinds = field_types(ATTACK_SOURCE)
    result = {}; catalog_unknown = []
    for path in package_data['weapons']:
        row = packages[path]
        if row.get('decode_errors') or row.get('native_defaults_included'):
            raise ValueError('Invalid original-only package record: ' + path)
        root = row['native_root']; native = NATIVE_CLASSES.get(root)
        if native not in classes:
            raise ValueError('Reviewed native constructor unavailable: ' + str(native))
        serialized = row['serialized_defaults']
        merged = merge(classes[native], serialized)
        values = {}; kinds = dict(weapon_kinds)
        for ue, field in weapon_map.items():
            if ue not in merged:
                if ue in OPTIONAL_CATALOG:
                    catalog_unknown.append({'package': path, 'field': ue, 'reason': 'Opaque native catalog text; no original label invented'})
                    continue
                if ue in BLUEPRINT_ONLY:
                    continue  # field is absent on this Blueprint, not a zero gameplay default
                raise ValueError(path + ': original weapon field unavailable: ' + ue)
            values[field] = convert(merged[ue], kinds[field], path + '.' + ue)
        values.update(id=path.rsplit('/', 1)[-1], class_path=path, native_class=root, chain=row['chain_child_first'])
        kinds.update(id='string', class_path='string', native_class='string', chain='string_list')
        for ue, field in attacks.items():
            if ue.startswith('Base') and ue not in serialized:
                continue
            base = classes[native].get(ue, classes.get('FAttackInfo') if ue.startswith('Base') else None)
            values[field] = attack_record(base, serialized.get(ue, {}), attack_map, attack_kinds, path + '.' + ue)
            kinds[field] = 'json'
        values['unset'] = sorted(k for k in weapon_map if k not in serialized)
        kinds['unset'] = 'string_list'
        # Retain only original mesh references for collision/animation consumers; no mesh or material export.
        mesh = component(row, 'SkeletalMeshComponent').get('SkeletalMesh')
        values['mesh'] = convert(mesh, 'string', path + '.SkeletalMesh') if mesh is not None else ''
        values['mesh_glb'] = ''; kinds.update(mesh='string', mesh_glb='string')
        weapon = typed_record('WeaponData', values, kinds)
        equip = equipment_record(merged, path, native, root == 'MordhauShield')
        entry = {'weapon': weapon, 'equip': equip, **profile_data(equip['weapon_animation_profile'], packages)}
        if weapon['b_has_alternate_mode']:
            alt = profile_data(equip['second_weapon_animation_profile'], packages)
            entry.update(alt_profile=alt['profile'], alt_parry_motion=alt['parry_motion'], alt_motions=alt['motions'])
        wid = weapon['id']
        if wid in result:
            raise ValueError('Duplicate original weapon identity: ' + wid)
        result[wid] = entry
    return result, catalog_unknown
