"""Diagnostic local constructor-cache generation from verified original EXE/PDB.

The literal reader still reports unsupported instructions and is not accepted as a full
consumer importer. Generated defaults and raw bytes stay in the external local cache.
"""
import argparse
import datetime
import importlib.util
import json
from pathlib import Path
import re
import sys
import struct

WEAPON_TYPES=('AMordhauEquipment','AMordhauWeapon','AMordhauShield','AVirtualWeapon','AFistsWeapon','AKickWeapon','FAttackInfo')
MOTION_TYPES=('UAttackMotion','UBlockedMotion','UCouchedAttackMotion','UDisarmedMotion','UFeintedMotion','UFlinchMotion',
              'UIdleMotion','UKickMotion','UMordhauMotion','UParryMotion','UStabMotion','UStrikeMotion','UStunMotion')


def weapon_consumer_fields():
    """Read authored consumer property identities, without reading its defaults."""
    root=Path(__file__).with_name('reader-source')
    def mapping(path,name):
        text=path.read_text(encoding='utf-8')
        match=re.search(r'const '+re.escape(name)+r'\s*:=\s*\{([^}]+)\}',text,re.S)
        if match is None:raise ValueError('Authored consumer property map missing: '+str(path))
        return set(re.findall(r'"([A-Za-z0-9_]+)"\s*:\s*"[A-Za-z0-9_]+"',match[1]))
    weapon=root/'game/combat/weapon_data.gd'
    equipment=(root/'components/ue/records/equipment_def.gd').read_text(encoding='utf-8')
    equipment_keys=set(re.findall(r'\br\.(?:b|f|v3|rot|obj)\("([A-Za-z0-9_]+)"\)',equipment))
    if len(equipment_keys)<10:raise ValueError('Authored equipment consumer schema incomplete')
    motions=(root/'components/ue/records/motion_defs.gd').read_text(encoding='utf-8')
    motion_keys=set(re.findall(r'\br\.[A-Za-z0-9_]+\("([A-Za-z0-9_]+)"\)',motions))
    motion_keys.update(re.findall(r'MotionDefs\.restriction\(r,\s*"([A-Za-z0-9_]+)"\)',motions))
    return {'weapon':mapping(weapon,'MAP')|mapping(weapon,'ATTACKS')|equipment_keys,
            'attack':mapping(root/'game/combat/attack_info.gd','MAP'),'motion':motion_keys}


def zero_allocation_evidence(pe):
    """Verify the supported binary's actual UObject zero-fill call and import.

    The RVAs select instructions in the hash-verified original, never asset or
    gameplay values. A changed call/argument/import is a blocked decoder.
    """
    import capstone
    cs=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_64);cs.detail=True
    start=0x1b886da
    rows=list(cs.disasm(pe.read_rva(start,14),pe.base+start))
    expected=[('movsxd','r8, dword ptr [rbp - 0x69]'),('xor','edx, edx'),('mov','rcx, rdi')]
    if [(v.mnemonic,v.op_str) for v in rows[:3]]!=expected or len(rows)!=4 or rows[3].mnemonic!='call':
        raise ValueError('Original UObject zero-allocation instructions changed')
    target=rows[3].operands[0].imm-pe.base
    thunk=next(cs.disasm(pe.read_rva(target,6),pe.base+target))
    if thunk.mnemonic!='jmp' or thunk.operands[0].type!=capstone.x86.X86_OP_MEM or thunk.operands[0].mem.base!=capstone.x86.X86_REG_RIP:
        raise ValueError('Original zero-fill target is not an import thunk')
    slot=thunk.address+thunk.size+thunk.operands[0].mem.disp-pe.base
    name_rva=struct.unpack('<Q',pe.read_rva(slot,8))[0]
    name=pe.read_rva(name_rva+2,64).split(b'\0',1)[0]
    if name!=b'memset':raise ValueError('Original zero-fill import is not memset')
    size_rows=list(cs.disasm(pe.read_rva(0x1b8846b,51),pe.base+0x1b8846b))
    if [(v.mnemonic,v.op_str) for v in size_rows[:2]]!=[('mov','r14d, dword ptr [r15 + 0x58]'),('mov','dword ptr [rbp - 0x69], r14d')]:
        raise ValueError('Original UObject PropertiesSize source changed')
    if not any(v.mnemonic=='mov' and v.op_str=='edx, r14d' for v in size_rows):raise ValueError('Original allocation size dependency missing')
    return {'allocation_size_field':'original UClass+0x58 PropertiesSize','memset_import':name.decode(),
            'memset_call_va':hex(rows[3].address),'memset_zero_argument':'EDX=0','memset_size_argument':'R8=sign-extended original PropertiesSize',
            'instructions':[{'address':hex(v.address),'asm':v.mnemonic+' '+v.op_str} for v in size_rows+rows]}


def enum_strings(layouts,values,review):
    """Name native enum bytes using the installed complete PDB enum type."""
    for row in review:
        typ=row['type']
        if not row['accepted'] or not typ.startswith('E') or '*' in typ or '<' in typ:continue
        current=values
        path=row['field'].split('.')
        for part in path[:-1]:current=current[part]
        value=current[path[-1]]
        entries=layouts.layout(typ)['enumerators'];matches=[n for n,v in entries if v==value]
        if len(matches)!=1:
            del current[path[-1]];row['accepted']=False;row['reason']='Native enum numeric value has missing/ambiguous original PDB name'
            continue
        label=matches[0];current[path[-1]]=label if '::' in label else typ+'::'+label
        row['enum_type_source']='original-complete-PDB';row['enum_raw_value']=value


def load(name):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(name+'.py'))
    module=importlib.util.module_from_spec(spec);sys.modules[name]=module;spec.loader.exec_module(module)
    return module


def generate(game,cache,selected=None,reviewed=False):
    native=load('native_cache');symbols=load('native_symbols');literal=load('literal_ctor');layoutmod=load('native_layouts')
    game,exe,pdb,pe=native.verify_install(game)
    cache=Path(cache).resolve()
    if cache==game or cache.is_relative_to(game):raise ValueError('Cache must be outside game install')
    source=Path(__file__).resolve().parents[2]
    if cache.is_relative_to(source) and cache.relative_to(source).parts[:1] not in [('build',),('cache',),('state',)]:
        raise ValueError('Derived constructor data must stay outside published source')
    cache.mkdir(parents=True,exist_ok=True)
    stage=cache/'stages'/('constructors-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));stage.mkdir(parents=True)
    codeview=native.codeview_module();msf=codeview.Msf(pdb);tpi=None
    try:
        inv=symbols.inventory(msf,pe)
        tpi=codeview.Tpi(msf.export(2,stage/'tpi.bin'),True);layouts=layoutmod.Layouts(codeview,tpi)
        groups={}
        for p in inv['constructors']:
            cls=p['name'].split('::')[0]
            if re.fullmatch('[AUF][A-Za-z0-9_]+',cls) and p['name']==cls+'::'+cls and cls in layouts.byname:
                groups.setdefault(cls,[]).append(p)
        # An inlined native constructor can exist only as UE's placement-new
        # wrapper. Its original object-initializer load is decoded explicitly.
        for p in inv['procedures']:
            match=re.fullmatch(r'InternalConstructor<([AU][A-Za-z0-9_]+)>',p['name'])
            if match and match[1] not in groups and match[1] in layouts.byname:groups.setdefault(match[1],[]).append(p)
        names=sorted(groups) if selected is None else sorted(set(selected))
        records={};reports={};headers={};reviewed_records={};field_reviews={}
        allocation=zero_allocation_evidence(pe) if reviewed else None
        consumer_fields=weapon_consumer_fields() if reviewed else None
        if reviewed and any(n not in WEAPON_TYPES+MOTION_TYPES for n in names):raise ValueError('Reviewed field exporter is scoped to weapon/motion prerequisites')
        for cls in names:
            try:
                rows=groups[cls]
                if len({(r['rva'],r['size']) for r in rows})!=1:raise ValueError('Overloaded constructor identity is unresolved')
                layout=layouts.layout(cls);replay=literal.Replay(pe,inv['procedures'],layout['size'],layouts.array_offsets(cls))
                result=replay.apply(rows[0]['rva']);layouts.skipped_fields=[];records[cls]=layouts.decode(cls,result['bytes'],result['arrays'])
                reports[cls]={k:v for k,v in result.items() if k!='bytes'}
                reports[cls]['raw_object_hex']=result['bytes'].hex()
                reports[cls]['field_decode_skips']=layouts.skipped_fields
                if reviewed:
                    values,fields=layouts.decode_reviewed(cls,result['bytes'],result['arrays'],bytes.fromhex(result['known_bytes']),result['stores'])
                    required=consumer_fields['attack' if cls=='FAttackInfo' else 'motion' if cls in MOTION_TYPES else 'weapon']
                    values={k:v for k,v in values.items() if k in required}
                    fields=[r for r in fields if r['field'].split('.')[0] in required]
                    enum_strings(layouts,values,fields)
                    reviewed_records[cls]=values;field_reviews[cls]=fields
                headers['extract/native/types/'+cls+'.h']=codeview.header(layouts.types,layout,None).encode()
            except (ValueError,KeyError,AssertionError,NotImplementedError) as error:
                reports[cls]={'error':str(error),'runtime_ready':False}
        for relative,data in headers.items():
            path=stage/'headers'/Path(relative).name;path.parent.mkdir(exist_ok=True)
            with path.open('xb') as f:f.write(data)
        with (stage/'defaults.json').open('x',encoding='utf-8') as f:json.dump(records,f,indent=2,allow_nan=False)
        with (stage/'replay-review.json').open('x',encoding='utf-8') as f:json.dump(reports,f,indent=2,allow_nan=False)
        with (stage/'procedure-index.json').open('x',encoding='utf-8') as f:json.dump(inv,f,indent=2)
        if reviewed:
            if len(reviewed_records)!=len(names):raise ValueError('Selected constructor review failed; see diagnostic stage')
            enum_types={}
            for typ in sorted({r['type'] for rows in field_reviews.values() for r in rows if r['accepted'] and r.get('enum_type_source')}):
                enum_types[typ]=[{'name':n if '::' in n else typ+'::'+n,'value':v} for n,v in layouts.layout(typ)['enumerators']]
            document={'schema_version':1,'exe_sha1':native.EXE_SHA1,'pdb_sha1':native.PDB_SHA1,'classes':reviewed_records,'enums':enum_types}
            with (stage/'constructor-defaults.json').open('x',encoding='utf-8') as f:json.dump(document,f,indent=2,allow_nan=False)
            review={'schema_version':1,'exe_sha1':native.EXE_SHA1,'pdb_sha1':native.PDB_SHA1,'zero_allocation':allocation,
                    'classes':field_reviews,'scope':'Per-field scalar/null/empty-container reconstruction; unsupported fields omitted, never zero-substituted',
                    'runtime_ready':False}
            with (stage/'constructor-review.json').open('x',encoding='utf-8') as f:json.dump(review,f,indent=2,allow_nan=False)
        # No consumer defaults publication before review: generated data stays in this diagnostic stage.
        receipt={'stage':'constructor-literal-diagnostic','complete':False,'runtime_ready':False,'exe_sha1':native.EXE_SHA1,
                 'pdb_sha1':native.PDB_SHA1,'output_directory':str(stage),'selected':len(names),'decoded':len(records),
                 'errors':len(names)-len(records),'unsupported_instructions':sum(len(v.get('unsupported',[])) for v in reports.values()),
                 'scope':'fresh original PDB-addressed literal replay, requiring per-consumer review before matrix use'}
        if reviewed:
            receipt.update(stage='constructor-field-review',defaults=str(stage/'constructor-defaults.json'),review=str(stage/'constructor-review.json'),
                           accepted_fields=sum(sum(r['accepted'] for r in v) for v in field_reviews.values()),
                           omitted_fields=sum(sum(not r['accepted'] for r in v) for v in field_reviews.values()))
            receipt['outputs']=[{'path':str(stage/name),'sha256':native.digest(stage/name,'sha256')} for name in ('constructor-defaults.json','constructor-review.json')]
        with (stage/'receipt.json').open('x',encoding='utf-8') as f:json.dump(receipt,f,indent=2)
        return receipt
    finally:
        if tpi is not None:tpi.mm.close();tpi.fh.close()
        msf.f.close()


def generate_reviewed(game,cache,selected=WEAPON_TYPES):
    return generate(game,cache,selected,reviewed=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True);parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--class-name',action='append',dest='classes')
    parser.add_argument('--reviewed-fields',action='store_true',help='Export accepted field projection and proof; unresolved fields remain omitted')
    args=parser.parse_args()
    try:result=generate(args.game_dir,args.cache_dir,args.classes or (WEAPON_TYPES if args.reviewed_fields else None),args.reviewed_fields)
    except (OSError,ValueError,KeyError,AssertionError,ImportError) as e:result={'runtime_ready':False,'error':str(e)}
    print(json.dumps(result,indent=2));return 2

if __name__=='__main__':raise SystemExit(main())
