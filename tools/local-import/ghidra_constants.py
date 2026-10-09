"""Reconstruct raw .rdata references in local constructor decompilation.

This implements the existing reader's low-64-bit table contract, not vector,
control-flow or constructor-default verification. No original values ship here.
"""
import argparse
import datetime
import importlib.util
import json
from pathlib import Path
import re
import struct


HEADER='va\trdata_off\twidth\traw\tf32\tf64\ti32\ti64\tfuncs\n'
REFERENCE=re.compile(r'\b_DAT_([0-9a-fA-F]{8,16})\b')


def reconstruct(pe,files):
    sections=[s for s in pe.sections if s[0]=='.rdata']
    if len(sections)!=1:raise ValueError('Missing/ambiguous original .rdata')
    section=sections[0];references={}
    for name,source in files:
        for match in REFERENCE.finditer(source):
            references.setdefault(int(match[1],16),set()).add(name)
    if not references:raise ValueError('No raw constructor .rdata references found')
    rows=[HEADER.rstrip('\n')];accepted=[];skipped=[]
    for va,users in sorted(references.items()):
        rva=va-pe.base
        if not section[1]<=rva<rva+8<=section[1]+section[4]:
            skipped.append({'va':hex(va),'files':sorted(users),'reason':'not an eight-byte file-backed .rdata range'})
            continue
        raw=pe.read_rva(rva,8)
        if len(raw)!=8:raise ValueError('Incomplete original PE read')
        fields=[f'{va:x}',f'{rva-section[1]:x}','8',raw.hex(),str(struct.unpack('<f',raw[:4])[0]),
                str(struct.unpack('<d',raw)[0]),str(struct.unpack('<i',raw[:4])[0]),str(struct.unpack('<q',raw)[0]),
                'local constructor reference: '+','.join(sorted(users))]
        rows.append('\t'.join(fields));accepted.append({'va':hex(va),'raw_hex':raw.hex(),'files':sorted(users)})
    return ('\n'.join(rows)+'\n').encode(),accepted,skipped


def generate(game,stage,native):
    game,exe,pdb,pe=native.verify_install(game)
    stage=Path(stage).resolve(strict=True);source_root=Path(__file__).resolve().parents[2]
    if stage==game or stage.is_relative_to(game):raise ValueError('Derived output cannot be original installation')
    if stage==source_root or stage.is_relative_to(source_root) and stage.relative_to(source_root).parts[:1] not in [('build',),('cache',),('state',)]:
        raise ValueError('Derived output cannot be published source')
    plan=json.loads((stage/'plan.json').read_text(encoding='utf-8'))
    if plan.get('original_exe_sha1')!=native.EXE_SHA1 or plan.get('original_pdb_sha1')!=native.PDB_SHA1:
        raise ValueError('Constructor plan original input pins differ')
    directory=stage/'extract/native/decomp_r1'
    if not directory.resolve(strict=True).is_relative_to(stage):raise ValueError('Decompilation input escaped local stage')
    files=[]
    for path in sorted(directory.glob('*.cpp')):
        if not path.resolve(strict=True).is_relative_to(stage):raise ValueError('Decompilation source escaped local stage')
        if path.stat().st_size>16<<20:raise ValueError('Constructor output exceeds bounded file limit')
        text=path.read_text(encoding='utf-8')
        if not text.startswith('// '):raise ValueError('Missing constructor export header')
        files.append((path.name,text))
    data,accepted,skipped=reconstruct(pe,files)
    outputs=native.commit_generated(stage,{'extract/native/rdata.tsv':data})
    receipt={'stage':'constructor-raw-rdata','complete':False,'runtime_ready':False,'original_exe_sha1':native.EXE_SHA1,
             'original_pdb_sha1':native.PDB_SHA1,'accepted':accepted,'skipped':skipped,'outputs':outputs,
             'limits':['Low64 .rdata bytes only, matching the existing reader contract.',
                       'Decompilation authenticity/prototypes, 128-bit/vector semantics, branches and consumed defaults remain separate review gates.',
                       'Writable/global/runtime data are not substituted with zero or original-value guesses.']}
    path=stage/('rdata-receipt-'+datetime.datetime.now(datetime.UTC).strftime('%Y%m%dT%H%M%S.%fZ')+'.json')
    with path.open('x',encoding='utf-8') as f:json.dump(receipt,f,indent=2)
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True);parser.add_argument('--stage-dir',type=Path,required=True)
    args=parser.parse_args()
    spec=importlib.util.spec_from_file_location('native_cache',Path(__file__).with_name('native_cache.py'))
    native=importlib.util.module_from_spec(spec);spec.loader.exec_module(native)
    try:result=generate(args.game_dir,args.stage_dir,native)
    except (OSError,ValueError,KeyError,AssertionError,ImportError) as error:result={'runtime_ready':False,'error':str(error)}
    print(json.dumps(result,indent=2));return 2


if __name__=='__main__':raise SystemExit(main())
