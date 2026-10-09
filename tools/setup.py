"""Consumer local import entry point. Current implementation generates native cache only.
Never downloads/copies private exports or claims full runtime readiness.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True)
    parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--verify-tool',type=Path,help='Optional existing verify-install executable (install check only)')
    parser.add_argument('--pak-tool',type=Path,help='Optional source-built mh-pak executable for local shadow capsule import')
    parser.add_argument('--json',action='store_true',help='Emit final structured status (also emitted by default)')
    parser.add_argument('--check',action='store_true',help='Verify original inputs only; creates no cache')
    args=parser.parse_args()
    module=Path(__file__).with_name('local-import')/'native_cache.py'
    spec=importlib.util.spec_from_file_location('native_cache',module)
    native=importlib.util.module_from_spec(spec);spec.loader.exec_module(native)
    inputs_verified=False
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
            receipt=native.generate(args.game_dir,args.cache_dir)
            if args.pak_tool is None:
                bundled_reader=Path(__file__).with_name('bin')/'mh-pak.exe'
                if bundled_reader.is_file():args.pak_tool=bundled_reader
            shadow_receipt=None
            if args.pak_tool:
                shadow_spec=importlib.util.spec_from_file_location('shadow_cache',module.with_name('shadow_cache.py'))
                shadow=importlib.util.module_from_spec(shadow_spec);shadow_spec.loader.exec_module(shadow)
                shadow_receipt=shadow.generate(args.game_dir,args.cache_dir,args.pak_tool,native)
            result={'original_inputs_verified':True,'native_cache_generated':True,'runtime_ready':False,
                    'native_stage':receipt,'shadow_stage':shadow_receipt,'status':'Requested local import stages complete. Full matrix and remaining runtime inputs are unfinished; launch remains blocked.'}
        print(json.dumps(result,indent=2))
        return 0 if args.check else 2
    except (OSError,ValueError,ImportError,subprocess.CalledProcessError,AssertionError) as error:
        print(json.dumps({'original_inputs_verified':inputs_verified,'native_cache_generated':False,'runtime_ready':False,
                          'status':'Local import blocked','error':str(error)},indent=2))
        return 2


if __name__=='__main__':raise SystemExit(main())
