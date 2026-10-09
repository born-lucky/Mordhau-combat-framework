"""Generate local shadow geometry from source-built mh-pak, never an exported private table."""
import json
import math
import os
from pathlib import Path
import re
import subprocess

PACKAGE = 'Mordhau/Content/UMA/UMA/Master/UMA_Master_ShadowPhysicsAsset'


def number(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError('Missing/nonfinite stored capsule number')
    return value


def capsules(exports):
    if not isinstance(exports, list): raise ValueError('Package exports must be an array')
    assets = [e for e in exports if isinstance(e, dict) and e.get('Type') == 'PhysicsAsset']
    if len(assets) != 1: raise ValueError('PhysicsAsset identity is ambiguous/missing')
    props = assets[0].get('Properties', {})
    refs = props.get('SkeletalBodySetups')
    if not isinstance(refs, list) or not refs: raise ValueError('No stored body references')
    rows = []; visited = set()
    for ref in refs:
        path = ref.get('ObjectPath') if isinstance(ref, dict) else None
        if not isinstance(path, str): raise ValueError('Invalid body reference')
        match = re.fullmatch(re.escape(PACKAGE) + r'\.([0-9]+)', path)
        if not match: raise ValueError('Body reference does not belong to selected original asset')
        index = int(match[1])
        if index in visited or index >= len(exports): raise ValueError('Duplicate/out-of-range body reference')
        visited.add(index)
        body = exports[index]
        if not isinstance(body, dict) or body.get('Type') != 'SkeletalBodySetup': raise ValueError('Wrong body export class')
        p = body.get('Properties', {})
        bone = p.get('BoneName')
        if not isinstance(bone, str) or not bone or bone == 'None': raise ValueError('Missing stored body bone')
        geom = p.get('AggGeom')
        if not isinstance(geom, dict): raise ValueError('Missing stored aggregate geometry')
        elems = geom.get('SphylElems', []) # absent serialized array means no capsule elements, never invented geometry
        if not isinstance(elems, list): raise ValueError('Malformed capsule array')
        for e in elems:
            if not isinstance(e, dict): raise ValueError('Malformed capsule element')
            center = e.get('Center'); rotation = e.get('Rotation')
            if not isinstance(center, dict) or not isinstance(rotation, dict): raise ValueError('Missing stored capsule transform')
            radius, length = number(e.get('Radius')), number(e.get('Length'))
            if radius <= 0 or length < 0: raise ValueError('Unsupported nonpositive capsule geometry')
            rows.append({'bone': bone, 'center': [number(center.get(k)) for k in ('X','Y','Z')],
                         'rotation_pyr': [number(rotation.get(k)) for k in ('Pitch','Yaw','Roll')],
                         'radius': radius, 'length': length})
    if not rows: raise ValueError('Asset contains no stored shadow capsules')
    return {'source': PACKAGE + ' (local original pak decode: SkeletalBodySetups/AggGeom.SphylElems; UE cm, bone space)',
            'capsules': rows}


def generate(game, cache, pak_tool, native):
    # Validate original installation before any cache writes. The tool is our small source-built reader.
    native.verify_install(game)
    tool = Path(pak_tool).resolve(strict=True)
    if not tool.is_file(): raise ValueError('Source-built mh-pak executable missing')
    env = dict(os.environ, MORDHAU_DIR=str(Path(game).resolve(strict=True)))
    result = subprocess.run([str(tool), 'json', PACKAGE], env=env, check=True, capture_output=True, text=True)
    table = capsules(json.loads(result.stdout))
    # Native stage established and validated cache containment before this follow-up.
    cache = Path(cache).resolve(strict=True)
    data = (json.dumps(table, indent=2, allow_nan=False) + '\n').encode()
    outputs = native.commit_generated(cache, {'shadow_capsules.json': data})
    return {'stage': 'shadow-capsules', 'complete': True, 'runtime_ready': False,
            'capsule_count': len(table['capsules']), 'bone_count': len({r['bone'] for r in table['capsules']}),
            'reader_sha256': native.digest(tool, 'sha256'), 'outputs': outputs}
