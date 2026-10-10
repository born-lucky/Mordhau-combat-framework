"""Consumer local import entry point for native, shadow and mode-bytecode stages.
Never downloads/copies private exports or claims full runtime readiness.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys


def select_install(game_dir,game_exe):
    spec=importlib.util.spec_from_file_location('local_install_source',Path(__file__).with_name('install_source.py'))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module.resolve_install(game_dir,game_exe)


def load_stage(name):
    module=Path(__file__).with_name('local-import')/(name+'.py')
    spec=importlib.util.spec_from_file_location(name,module)
    loaded=importlib.util.module_from_spec(spec);spec.loader.exec_module(loaded)
    return loaded


def discover_weapon_tools(args):
    locations=[Path(__file__).with_name('bin')]
    if args.pak_tool is not None:locations.append(args.pak_tool.parent)
    for field,filename in [('weapon_tool','mh-weapon-packages.exe'),('weapon_verify_tool','mh-verify-weapon-import.exe')]:
        if getattr(args,field) is None:
            for directory in locations:
                candidate=directory/filename
                if candidate.is_file():
                    setattr(args,field,candidate)
                    break


def populate_weapon_stage(args,native,stages,required=False):
    discover_weapon_tools(args)
    if args.weapon_tool is None or args.weapon_verify_tool is None:
        if required:
            raise ValueError('Weapon import requires source-built mh-weapon-packages.exe and mh-verify-weapon-import.exe; supply --weapon-tool and --weapon-verify-tool or place them beside the supplied pak reader')
        return
    receipt=load_stage('weapon_cache').generate(args.game_dir,args.cache_dir,args.weapon_tool,args.weapon_verify_tool,native)
    if receipt.get('complete') is not True or receipt.get('runtime_ready') is not False:
        raise ValueError('Weapon import did not establish a completed, verified weapon stage')
    stages['weapon_stage']=receipt


def populate_stages(args,native,stages):
    stages['native_stage']=native.generate(args.game_dir,args.cache_dir)
    stages['enum_stage']=load_stage('enum_cache').generate(args.game_dir,args.cache_dir,[
        'Z_Construct_UEnum_Mordhau_EAttackMove',
        'Z_Construct_UEnum_Mordhau_EMovementRestriction',
        'Z_Construct_UEnum_Mordhau_EPerk'])
    if args.pak_tool is None:
        bundled_reader=Path(__file__).with_name('bin')/'mh-pak.exe'
        if bundled_reader.is_file():args.pak_tool=bundled_reader
    if args.pak_tool:
        stages['shadow_stage']=load_stage('shadow_cache').generate(args.game_dir,args.cache_dir,args.pak_tool,native)
        stages['mode_stage']=load_stage('mode_cache').generate(args.game_dir,args.cache_dir,args.pak_tool,native)
    populate_weapon_stage(args,native,stages)


def remaining_requirements(stages):
    missing=[]
    if stages['shadow_stage'] is None:missing.append('local body shadow capsules: source-built pak reader required')
    if stages['mode_stage'] is None:missing.append('local mode bytecode cache: source-built pak reader required')
    if stages['weapon_stage'] is None:missing.append('verified weapon data: source-built mh-weapon-packages.exe and mh-verify-weapon-import.exe required')
    return missing+['full generated spec matrix and reviewed constructor fields','local particle inputs','permitted native physics bridge']


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    original=parser.add_mutually_exclusive_group()
    original.add_argument('--game-dir',type=Path,help='Existing original installation (otherwise detect running MORDHAU, then Steam)')
    original.add_argument('--game-exe',type=Path,help='Original Shipping executable inside its existing installation')
    parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--verify-tool',type=Path,help='Optional existing verify-install executable (install check only)')
    parser.add_argument('--pak-tool',type=Path,help='Optional source-built mh-pak executable for local shadow and mode bytecode import')
    parser.add_argument('--weapon-tool',type=Path,help='Source-built mh-weapon-packages.exe for original weapon package/CDO import')
    parser.add_argument('--weapon-verify-tool',type=Path,help='Source-built mh-verify-weapon-import.exe for Rust weapon record verification')
    parser.add_argument('--json',action='store_true',help='Emit final structured status (also emitted by default)')
    operation=parser.add_mutually_exclusive_group()
    operation.add_argument('--check',action='store_true',help='Verify original inputs only; creates no cache')
    operation.add_argument('--weapons-only',action='store_true',help='Generate and verify weapon records only; does not establish full runtime readiness')
    args=parser.parse_args(argv)
    native=load_stage('native_cache')
    inputs_verified=False
    installation=None
    stages={'native_stage':None,'enum_stage':None,'shadow_stage':None,'mode_stage':None,'weapon_stage':None}
    try:
        installation=select_install(args.game_dir,args.game_exe)
        args.game_dir=Path(installation['game_dir'])
        if args.verify_tool:
            if not args.verify_tool.is_file():raise native.ImportBlocked('Supplied install verifier does not exist')
            subprocess.run([str(args.verify_tool.resolve()),str(args.game_dir.resolve())],check=True,stdout=sys.stderr,stderr=sys.stderr)
        _,exe,pdb,_=native.verify_install(args.game_dir)
        inputs_verified=True
        if args.check:
            result={'original_inputs_verified':True,'native_cache_generated':False,'runtime_ready':False,
                    'exe_sha1':native.EXE_SHA1,'pdb_sha1':native.PDB_SHA1,
                    'status':'Original EXE/PDB/pak/DLL inputs verified; no generated data written and runtime readiness not established.'}
        elif args.weapons_only:
            populate_weapon_stage(args,native,stages,required=True)
            result={'original_inputs_verified':True,'native_cache_generated':False,'weapon_cache_generated':True,'runtime_ready':False,
                    **stages,'remaining':remaining_requirements(stages),
                    'status':'Weapon data imported and verified. Remaining full runtime inputs are unfinished; launch readiness is not established.'}
        else:
            populate_stages(args,native,stages)
            result={'original_inputs_verified':True,'native_cache_generated':True,'runtime_ready':False,
                    **stages,'remaining':remaining_requirements(stages),
                    'status':'Requested local import stages complete. Full matrix and remaining runtime inputs are unfinished; launch remains blocked.'}
        result['installation']=installation
        print(json.dumps(result,indent=2))
        return 0 if args.check or args.weapons_only else 2
    except (OSError,ValueError,ImportError,subprocess.SubprocessError,AssertionError) as error:
        print(json.dumps({'original_inputs_verified':inputs_verified,'native_cache_generated':stages['native_stage'] is not None,'runtime_ready':False,
                          **stages,'installation':installation,
                          'status':'Local import blocked','error':str(error)},indent=2))
        return 2


if __name__=='__main__':raise SystemExit(main())
