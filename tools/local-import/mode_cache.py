"""Generate mode bytecode records from installed pak bytes and authored readers.

No private exports or original UI images/fonts are imported. This supplies one
consumer dependency and never declares the full rewrite ready.
"""
import argparse
import ast
import datetime
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import time


def source_script():return Path(__file__).with_name('matrix-source')/'scripts/mode_kismet.py'


def recipes(path):
    """Read package/function recipe identities without executing generator code."""
    tree=ast.parse(Path(path).read_text(encoding='utf-8'));values={};spec=None
    def evaluate(node):
        if isinstance(node,ast.Constant) and isinstance(node.value,str):return node.value
        if isinstance(node,ast.Name) and node.id in values:return values[node.id]
        if isinstance(node,ast.BinOp) and isinstance(node.op,ast.Add):return evaluate(node.left)+evaluate(node.right)
        if isinstance(node,ast.Dict):return {evaluate(k):evaluate(v) for k,v in zip(node.keys,node.values)}
        raise ValueError('Unsupported source package recipe expression')
    for statement in tree.body:
        if not isinstance(statement,ast.Assign) or len(statement.targets)!=1 or not isinstance(statement.targets[0],ast.Name):continue
        name=statement.targets[0].id
        if name in ('GM','PKGS'):values[name]=evaluate(statement.value)
        elif name=='SPEC':spec=statement.value
    packages=values.get('PKGS');bindings={}
    if not isinstance(packages,dict) or not packages or not isinstance(spec,ast.Dict):raise ValueError('Missing source package/function recipes')
    for key,value in zip(spec.keys,spec.values):
        ident=evaluate(key)
        if ident in bindings:raise ValueError('Duplicate bytecode recipe')
        if not isinstance(value,ast.Tuple) or len(value.elts)!=5:raise ValueError('Unsupported bytecode source recipe')
        package,function=map(evaluate,value.elts[:2])
        if package not in packages or not function:raise ValueError('Unknown package/function recipe identity')
        bindings[ident]=(package,function)
    for package,path in packages.items():
        if not isinstance(package,str) or not re.fullmatch(r'Mordhau/Content/[A-Za-z0-9_/-]+',path) or '..' in Path(path).parts:
            raise ValueError('Unsupported original package path')
    if not bindings:raise ValueError('Missing bytecode bindings')
    return packages,bindings


def validate(table,packages,bindings):
    if not isinstance(table,dict) or set(table)!=set(bindings):raise ValueError('Generated mode recipe keys differ from source')
    def value_ok(value):
        if isinstance(value,bool) or isinstance(value,str):return True
        if isinstance(value,(int,float)):return math.isfinite(value)
        return isinstance(value,list) and all(value_ok(v) for v in value)
    for key,(package,function) in bindings.items():
        row=table[key]
        if not isinstance(row,dict) or set(row)!= {'value','src'} or not value_ok(row['value']):raise ValueError('Malformed/nonfinite mode value')
        prefix=packages[package].rsplit('/',1)[1]+':'+function+'@'
        citation=row['src']
        if not isinstance(citation,str) or not citation.startswith(prefix) or not re.fullmatch('[0-9]+',citation[len(prefix):]):
            raise ValueError('Generated mode source identity differs from recipe')
    return table


def run_logged(command,env,stage,index,timeout):
    started=time.monotonic();record={'command':command,'timeout_seconds':timeout}
    result=None
    try:
        result=subprocess.run(command,env=env,cwd=str(stage),capture_output=True,text=True,timeout=timeout)
        record.update(exit_code=result.returncode,stdout=result.stdout,stderr=result.stderr)
    except (OSError,subprocess.SubprocessError) as error:
        record.update(error=str(error),stdout=str(getattr(error,'stdout','')),stderr=str(getattr(error,'stderr','')))
        raise
    finally:
        record['duration_seconds']=time.monotonic()-started
        with (stage/f'process-{index:03}.json').open('x',encoding='utf-8') as f:json.dump(record,f,indent=2)
    result.check_returncode()
    return result


def generate(game,cache,pak_tool,native):
    game,exe,pdb,pe=native.verify_install(game)
    cache=Path(cache).resolve();source_root=Path(__file__).resolve().parents[2]
    if cache==game or cache.is_relative_to(game):raise ValueError('Mode outputs must be outside original install')
    if cache==source_root or cache.is_relative_to(source_root) and cache.relative_to(source_root).parts[:1] not in [('build',),('cache',),('state',)]:
        raise ValueError('Mode outputs cannot be published source')
    tool=Path(pak_tool).resolve(strict=True)
    if not tool.is_file() or tool.is_relative_to(game) or tool.name.lower()!='mh-pak.exe':
        raise ValueError('Expected separately source-built mh-pak.exe outside the game installation')
    script=source_script();packages,bindings=recipes(script)
    cache.mkdir(parents=True,exist_ok=True)
    stage=cache/'stages'/('mode-bytecode-'+datetime.datetime.now(datetime.UTC).strftime('%Y%m%dT%H%M%S.%fZ'))
    raw=stage/'raw';raw.mkdir(parents=True);output=stage/'mode_kismet.json'
    env=dict(os.environ,MORDHAU_DIR=str(game),MORDHAU_MODE_RAW=str(raw),MORDHAU_MODE_OUT=str(output),
             MORDHAU_MODE_REQUIRE_RAW='1',MORDHAU_MODE_CONSTANTS_ONLY='1')
    inputs=[];process_index=0
    for package,path in sorted(packages.items()):
        for extension in ('.uasset','.uexp'):
            relative=path+extension;destination=raw/relative;destination.parent.mkdir(parents=True,exist_ok=True)
            process_index+=1
            result=run_logged([str(tool),'raw',relative,str(destination)],env,stage,process_index,60)
            if not destination.is_file() or destination.stat().st_size==0 or destination.stat().st_size>16<<20:
                raise ValueError('Missing/oversized original mode package bytes')
            if extension=='.uasset' and destination.read_bytes()[:4]!=b'\xc1\x83\x2a\x9e':raise ValueError('Wrong original UAsset magic')
            inputs.append({'package':package,'path':relative,'sha256':native.digest(destination,'sha256'),
                           'bytes':destination.stat().st_size,'reader_stdout':result.stdout,'reader_stderr':result.stderr})
    generated=run_logged([sys.executable,str(script)],env,stage,process_index+1,180)
    if not output.is_file() or output.stat().st_size>4<<20:raise ValueError('Missing/oversized generated mode cache')
    table=validate(json.loads(output.read_text(encoding='utf-8')),packages,bindings)
    data=(json.dumps(table,indent=1,sort_keys=True,allow_nan=False)+'\n').encode()
    outputs=native.commit_generated(cache,{'data_gen/mode/mode_kismet.json':data})
    dependencies=[script,*sorted((script.parent/'kismet').glob('*.py'))]
    receipt={'stage':'original-mode-bytecode','complete':True,'runtime_ready':False,'exe_sha1':native.EXE_SHA1,
             'pdb_sha1':native.PDB_SHA1,'reader_sha256':native.digest(tool,'sha256'),
             'source_files':[{'path':str(p.relative_to(Path(__file__).parent)),'sha256':native.digest(p,'sha256')} for p in dependencies],
             'packages':len(packages),'constants':len(table),'original_inputs':inputs,'outputs':outputs,
             'generator_stdout':generated.stdout,'generator_stderr':generated.stderr,
             'scope':'Stored original Blueprint bytecode constants/citations only; no original UI texture/font export',
             'limits':['This stage does not generate the full spec matrix, particle inputs or native physics bridge.',
                       'Source reader semantics and actual original package/result identity require independent acceptance.']}
    with (stage/'receipt.json').open('x',encoding='utf-8') as f:json.dump(receipt,f,indent=2)
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True);parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--pak-tool',type=Path,required=True);args=parser.parse_args()
    spec=importlib.util.spec_from_file_location('native_cache',Path(__file__).with_name('native_cache.py'))
    native=importlib.util.module_from_spec(spec);spec.loader.exec_module(native)
    try:result=generate(args.game_dir,args.cache_dir,args.pak_tool,native)
    except (OSError,ValueError,KeyError,AssertionError,ImportError,subprocess.SubprocessError) as error:
        result={'runtime_ready':False,'error':str(error)}
    print(json.dumps(result,indent=2));return 2


if __name__=='__main__':raise SystemExit(main())
