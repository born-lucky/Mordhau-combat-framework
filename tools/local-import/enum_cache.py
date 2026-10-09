"""Generate original reflection enum records locally, without exported label tables."""
import argparse,datetime,importlib.util,json,sys
from pathlib import Path

def load(name):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(name+'.py'))
    module=importlib.util.module_from_spec(spec);sys.modules[name]=module;spec.loader.exec_module(module);return module

def generate(game,cache,labels):
    native=load('native_cache');symbols=load('native_symbols');reflection=load('native_reflection');layoutmod=load('native_layouts')
    game,exe,pdb,pe=native.verify_install(game)
    cache=Path(cache).resolve();source=Path(__file__).resolve().parents[2]
    if cache==game or cache.is_relative_to(game):raise ValueError('Enum cache must be outside installation')
    if cache==source or cache.is_relative_to(source) and cache.relative_to(source).parts[:1] not in [('build',),('cache',),('state',)]:
        raise ValueError('Original enum records cannot be placed in published source')
    cache.mkdir(parents=True,exist_ok=True)
    stage=cache/'stages'/('enums-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));stage.mkdir(parents=True)
    codeview=native.codeview_module();msf=codeview.Msf(pdb);tpi=None
    try:
        records=reflection.collect(msf,pe,symbols)
        tpi=codeview.Tpi(msf.export(2,stage/'tpi.bin'),True);layouts=layoutmod.Layouts(codeview,tpi)
        params=layouts.layout('UE4CodeGen_Private::FEnumParams');entry=layouts.layout('UE4CodeGen_Private::FEnumeratorParam')
        values={label:reflection.read_enum(pe,records,label,params,entry) for label in labels}
        with (stage/'enums.json').open('x',encoding='utf-8') as f:json.dump(values,f,indent=2,allow_nan=False)
        receipt={'stage':'native-reflection-enums','exe_sha1':native.EXE_SHA1,'pdb_sha1':native.PDB_SHA1,
                 'output':str(stage/'enums.json'),'enum_count':len(values),'row_count':sum(len(v['rows']) for v in values.values()),
                 'complete':False,'runtime_ready':False,'scope':'Original structured PDB data and typed original PE table reads; consumer matrix integration pending'}
        with (stage/'receipt.json').open('x',encoding='utf-8') as f:json.dump(receipt,f,indent=2)
        return receipt
    finally:
        if tpi is not None:tpi.mm.close();tpi.fh.close()
        msf.f.close()

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--game-dir',type=Path,required=True)
    p.add_argument('--cache-dir',type=Path,required=True);p.add_argument('--enum-label',action='append',required=True)
    args=p.parse_args()
    try:result=generate(args.game_dir,args.cache_dir,args.enum_label)
    except (OSError,ValueError,KeyError,AssertionError,ImportError,NotImplementedError) as e:result={'runtime_ready':False,'error':str(e)}
    print(json.dumps(result,indent=2));return 2
if __name__=='__main__':raise SystemExit(main())
