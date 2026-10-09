"""Consumer local import entry point for native, shadow and mode-bytecode stages.
Never downloads/copies private exports or claims full runtime readiness.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys


def load_stage(name):
    module=Path(__file__).with_name('local-import')/(name+'.py')
    spec=importlib.util.spec_from_file_location(name,module)
    loaded=importlib.util.module_from_spec(spec);spec.loader.exec_module(loaded)
    return loaded


def populate_stages(args,native,stages):
    stages['native_stage']=native.generate(args.game_dir,args.cache_dir)
    if args.pak_tool is None:
        bundled_reader=Path(__file__).with_name('bin')/'mh-pak.exe'
        if bundled_reader.is_file():args.pak_tool=bundled_reader
    if args.pak_tool:
        stages['shadow_stage']=load_stage('shadow_cache').generate(args.game_dir,args.cache_dir,args.pak_tool,native)
        stages['mode_stage']=load_stage('mode_cache').generate(args.game_dir,args.cache_dir,args.pak_tool,native)


def remaining_requirements(stages):
    missing=[]
    if stages['shadow_stage'] is None:missing.append('local body shadow capsules: source-built pak reader required')
    if stages['mode_stage'] is None:missing.append('local mode bytecode cache: source-built pak reader required')
    return missing+['full generated spec matrix and reviewed constructor fields','local particle inputs','permitted native physics bridge']


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True)
    parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--verify-tool',type=Path,help='Optional existing verify-install executable (install check only)')
    parser.add_argument('--pak-tool',type=Path,help='Optional source-built mh-pak executable for local shadow and mode bytecode import')
    parser.add_argument('--json',action='store_true',help='Emit final structured status (also emitted by default)')
    parser.add_argument('--check',action='store_true',help='Verify original inputs only; creates no cache')
    args=parser.parse_args(argv)
    native=load_stage('native_cache')
    inputs_verified=False
    stages={'native_stage':None,'shadow_stage':None,'mode_stage':None}
    try:
        if args.verify_tool:
            if not args.verify_tool.is_file():raise native.ImportBlocked('Supplied install verifier does not exist')
            subprocess.run([str(args.verify_tool.resolve()),str(args.game_dir.resolve())],check=True,stdout=sys.stderr,stderr=sys.stderr)
        _,exe,pdb,_=native.verify_install(args.game_dir)
        inputs_verified=True
        if args.check:
            result={'original_inputs_verified':True,'native_cache_generated':False,'runtime_ready':False,
                    'exe_sha1':native.EXE_SHA1,'pdb_sha1':native.PDB_SHA1,
                    'status':'Original EXE/PDB/pak/DLL inputs verified; no generated data written and runtime readiness not established.'}
        else:
            populate_stages(args,native,stages)
            result={'original_inputs_verified':True,'native_cache_generated':True,'runtime_ready':False,
                    **stages,'remaining':remaining_requirements(stages),
                    'status':'Requested local import stages complete. Full matrix and remaining runtime inputs are unfinished; launch remains blocked.'}
        print(json.dumps(result,indent=2))
        return 0 if args.check else 2
    except (OSError,ValueError,ImportError,subprocess.SubprocessError,AssertionError) as error:
        print(json.dumps({'original_inputs_verified':inputs_verified,'native_cache_generated':stages['native_stage'] is not None,'runtime_ready':False,
                          **stages,
                          'status':'Local import blocked','error':str(error)},indent=2))
        return 2


if __name__=='__main__':raise SystemExit(main())
