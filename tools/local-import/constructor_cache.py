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


def load(name):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(name+'.py'))
    module=importlib.util.module_from_spec(spec);sys.modules[name]=module;spec.loader.exec_module(module)
    return module


def generate(game,cache,selected=None):
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
        names=sorted(groups) if selected is None else sorted(set(selected))
        records={};reports={};headers={}
        for cls in names:
            try:
                rows=groups[cls]
                if len({(r['rva'],r['size']) for r in rows})!=1:raise ValueError('Overloaded constructor identity is unresolved')
                layout=layouts.layout(cls);replay=literal.Replay(pe,inv['procedures'],layout['size'],layouts.array_offsets(cls))
                result=replay.apply(rows[0]['rva']);layouts.skipped_fields=[];records[cls]=layouts.decode(cls,result['bytes'],result['arrays'])
                reports[cls]={k:v for k,v in result.items() if k!='bytes'}
                reports[cls]['raw_object_hex']=result['bytes'].hex()
                reports[cls]['field_decode_skips']=layouts.skipped_fields
                headers['extract/native/types/'+cls+'.h']=codeview.header(layouts.types,layout,None).encode()
            except (ValueError,KeyError,AssertionError,NotImplementedError) as error:
                reports[cls]={'error':str(error),'runtime_ready':False}
        for relative,data in headers.items():
            path=stage/'headers'/Path(relative).name;path.parent.mkdir(exist_ok=True)
            with path.open('xb') as f:f.write(data)
        with (stage/'defaults.json').open('x',encoding='utf-8') as f:json.dump(records,f,indent=2,allow_nan=False)
        with (stage/'replay-review.json').open('x',encoding='utf-8') as f:json.dump(reports,f,indent=2,allow_nan=False)
        with (stage/'procedure-index.json').open('x',encoding='utf-8') as f:json.dump(inv,f,indent=2)
        # No consumer defaults publication before review: generated data stays in this diagnostic stage.
        receipt={'stage':'constructor-literal-diagnostic','complete':False,'runtime_ready':False,'exe_sha1':native.EXE_SHA1,
                 'pdb_sha1':native.PDB_SHA1,'output_directory':str(stage),'selected':len(names),'decoded':len(records),
                 'errors':len(names)-len(records),'unsupported_instructions':sum(len(v.get('unsupported',[])) for v in reports.values()),
                 'scope':'fresh original PDB-addressed literal replay, requiring per-consumer review before matrix use'}
        with (stage/'receipt.json').open('x',encoding='utf-8') as f:json.dump(receipt,f,indent=2)
        return receipt
    finally:
        if tpi is not None:tpi.mm.close();tpi.fh.close()
        msf.f.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True);parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--class-name',action='append',dest='classes')
    args=parser.parse_args()
    try:result=generate(args.game_dir,args.cache_dir,args.classes)
    except (OSError,ValueError,KeyError,AssertionError,ImportError) as e:result={'runtime_ready':False,'error':str(e)}
    print(json.dumps(result,indent=2));return 2

if __name__=='__main__':raise SystemExit(main())
