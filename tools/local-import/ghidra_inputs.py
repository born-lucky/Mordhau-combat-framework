"""Prepare fresh, bounded Ghidra constructor inputs from a verified installation.

This writes an explicit execution plan; it never starts Java or executes original
code. Decompiled constructor defaults require separate consumer-field review.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys

EXPORTER_SHA256='07aee139e379eda5f9acb30e4041d4ad0a43d96bd55703e616760dd516849b48'
GHIDRA_VERSION='11.3.2'


def load(name):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(name+'.py'))
    module=importlib.util.module_from_spec(spec);sys.modules[name]=module;spec.loader.exec_module(module)
    return module


def class_name(name):
    parts=name.split('::')
    return parts[0] if len(parts)==2 and parts[0]==parts[1] and re.fullmatch('[AUF][A-Za-z0-9_]+',parts[0]) else None


def text_section(pe):
    rows=[s for s in pe.sections if s[0]=='.text']
    if len(rows)!=1:raise ValueError('Missing/ambiguous original .text section')
    return rows[0]


def constructor_rows(pe,inventory,classes,decoder=None):
    """Include selected constructors and directly called constructor groups.

    Folded original identities retain every alias. Unresolved calls and indirect
    targets remain recorded; they cannot silently become constructor defaults.
    """
    if not classes:raise ValueError('Select at least one bounded constructor class')
    byclass={};byrva={}
    for row in inventory['constructors']:
        cls=class_name(row['name'])
        if cls is None:continue
        byclass.setdefault(cls,[]).append(row);byrva.setdefault(row['rva'],[]).append(row)
    selected=set()
    for cls in set(classes):
        if cls not in byclass:raise ValueError('Original constructor missing: '+cls)
        rows=byclass[cls]
        if len({(r['rva'],r['size']) for r in rows})!=1:raise ValueError('Original constructor overload unresolved: '+cls)
        selected.add(rows[0]['rva'])
    if decoder is None:
        from capstone import Cs,CS_ARCH_X86,CS_MODE_64
        cs=Cs(CS_ARCH_X86,CS_MODE_64);cs.detail=True
        decoder=lambda raw,va:cs.disasm(raw,va)
    from capstone.x86 import X86_OP_IMM
    queue=sorted(selected);seen=set();calls=[]
    while queue:
        rva=queue.pop(0)
        if rva in seen:continue
        seen.add(rva);aliases=byrva[rva]
        if len({r['size'] for r in aliases})!=1:raise ValueError('Folded constructor size ambiguity')
        size=aliases[0]['size']
        if not 0<size<=1<<20:raise ValueError('Unsupported constructor size')
        raw=pe.read_rva(rva,size)
        covered=0
        for ins in decoder(raw,pe.base+rva):
            if ins.address!=pe.base+rva+covered or not 0<ins.size<=size-covered:
                raise ValueError('Original constructor instruction coverage is not contiguous')
            covered+=ins.size
            if ins.mnemonic not in ('call','jmp'):continue
            if len(ins.operands)==1 and ins.operands[0].type==X86_OP_IMM:
                target=ins.operands[0].imm-pe.base
                if rva<=target<rva+size:continue # local control flow, not a call target
                known=target in byrva
                calls.append({'source_rva':rva,'instruction_va':ins.address,'mnemonic':ins.mnemonic,
                              'target_rva':target,'known_game_constructor':known})
                if known and target not in seen:queue.append(target)
            else:calls.append({'source_rva':rva,'instruction_va':ins.address,'mnemonic':ins.mnemonic,
                               'target_rva':None,'known_game_constructor':False})
        if covered!=size:raise ValueError('Original constructor instruction coverage incomplete')
        if len(seen)>512:raise ValueError('Bounded constructor closure exceeded')
    text=text_section(pe);start=text[1];limit=start+text[4]
    rows=[]
    for rva in sorted(seen):
        for row in sorted(byrva[rva],key=lambda r:r['name']):
            if not start<=rva<rva+row['size']<=limit:raise ValueError('Constructor not file-backed in .text')
            if any(c in row['name']+row['object'] for c in '\t\r\n'):raise ValueError('Invalid TSV procedure identity')
            rows.append(row)
    label_targets=seen|{c['target_rva'] for c in calls if c['target_rva'] is not None}
    labels={}
    for row in inventory['procedures']:
        if row['rva'] in label_targets and start<=row['rva']<limit:
            if any(c in row['name'] for c in '\t\r\n'):raise ValueError('Invalid PDB label')
            labels.setdefault((row['rva']-start,row['name']),None)
    label_text=''.join(f'{off}\t{name}\n' for off,name in sorted(labels))
    function_text='name\tsection\toffset\tsize\tobj\n'+''.join(
        f"{r['name']}\t.text\t{r['rva']-start}\t{r['size']}\t{r['object']}\n" for r in rows)
    return label_text,function_text,rows,calls


def header_closure(layouts,names):
    pending=list(names);seen=set();out={}
    while pending:
        cls=pending.pop()
        if cls in seen:continue
        seen.add(cls)
        if not re.fullmatch('[AUF][A-Za-z0-9_]+',cls):raise ValueError('Unsupported header filename: '+cls)
        layout=layouts.layout(cls);out[cls]=layout
        pending.extend(b['name'] for b in layout['bases'])
        for field in layout['members']:
            typ=layouts.types.decl(field['ti'])
            if re.fullmatch('F[A-Za-z0-9_]+',typ) and typ in layouts.byname:pending.append(typ)
        if len(seen)>1024:raise ValueError('Bounded layout closure exceeded')
    return out


def headless_command(ghidra,stage,exe,exporter_dir,timeout=60):
    ghidra=Path(ghidra).resolve(strict=True);stage=Path(stage).resolve(strict=True)
    properties=(ghidra/'Ghidra/application.properties').read_text(encoding='utf-8')
    if f'application.version={GHIDRA_VERSION}\n' not in properties.replace('\r\n','\n'):
        raise ValueError('Unsupported Ghidra version; review a new version before using it')
    launcher=ghidra/'support/launch.bat'
    if not launcher.is_file():raise ValueError('Missing Windows Ghidra launcher')
    if not 1<=timeout<=300:raise ValueError('Per-function timeout outside bounded range')
    for path in (ghidra,stage,Path(exe),Path(exporter_dir)):
        if any(c in str(path) for c in '%!&|<>\r\n"'):
            raise ValueError('Unsupported batch argument characters in input/output path')
    project=stage/'ghidra-project';project.mkdir(exist_ok=False)
    output=stage/'extract/native/decomp_r1';output.mkdir(parents=True,exist_ok=False)
    # analyzeHeadless.bat overwrites MAXMEM with 2G. Call its bundled lower-level
    # launcher directly so the reviewed heap cap is real, with no tool mutation.
    return [str(launcher),'fg','jdk','Ghidra-Headless','1G',
            '-XX:ParallelGCThreads=2 -XX:CICompilerCount=2',
            'ghidra.app.util.headless.AnalyzeHeadless',str(project),'LocalConstructors',
            '-import',str(Path(exe).resolve(strict=True)),'-noanalysis','-max-cpu','1',
            '-loader-loadLibraries','false','-scriptPath',str(Path(exporter_dir).resolve(strict=True)),
            '-postScript','DecompGame.java',str(stage/'labels.tsv'),str(stage/'game_functions.tsv'),
            str(output),str(timeout),'all','journal='+str(stage/'constructor-journal.txt')]


def prepare(game,cache,classes,ghidra):
    native=load('native_cache');symbols=load('native_symbols');layoutsmod=load('native_layouts')
    game,exe,pdb,pe=native.verify_install(game)
    cache=Path(cache).resolve();source=Path(__file__).resolve().parents[2]
    if cache==game or cache.is_relative_to(game):raise ValueError('Generated output must be outside original install')
    if cache==source or cache.is_relative_to(source) and cache.relative_to(source).parts[:1] not in [('build',),('cache',),('state',)]:
        raise ValueError('Generated output cannot be published source')
    exporter=Path(__file__).with_name('native-tools')/'DecompGame.java'
    if hashlib.sha256(exporter.read_bytes()).hexdigest()!=EXPORTER_SHA256:raise ValueError('Reviewed exporter source changed')
    cache.mkdir(parents=True,exist_ok=True)
    stage=cache/'stages'/('ghidra-constructors-'+datetime.datetime.now(datetime.UTC).strftime('%Y%m%dT%H%M%S.%fZ'))
    stage.mkdir(parents=True)
    codeview=native.codeview_module();msf=codeview.Msf(pdb);tpi=None
    try:
        inv=symbols.inventory(msf,pe)
        labels,functions,rows,calls=constructor_rows(pe,inv,classes)
        tpi=codeview.Tpi(msf.export(2,stage/'tpi.bin'),True);layouts=layoutsmod.Layouts(codeview,tpi)
        headers=header_closure(layouts,{class_name(r['name']) for r in rows})
        directory=stage/'extract/native/types';directory.mkdir(parents=True)
        for cls,layout in headers.items():
            with (directory/(cls+'.h')).open('x',encoding='utf-8') as f:f.write(codeview.header(layouts.types,layout,None))
        for name,text in [('labels.tsv',labels),('game_functions.tsv',functions)]:
            with (stage/name).open('x',encoding='utf-8') as f:f.write(text)
        command=headless_command(ghidra,stage,exe,exporter.parent)
        report={'stage':'targeted-ghidra-inputs','runtime_ready':False,'complete':False,
                'original_exe_sha1':native.EXE_SHA1,'original_pdb_sha1':native.PDB_SHA1,
                'exporter_sha256':EXPORTER_SHA256,'ghidra_version':GHIDRA_VERSION,'heap_limit':'1G',
                'output_directory':str(stage),'requested_classes':sorted(set(classes)),
                'constructor_rows':rows,'call_targets':calls,'header_classes':sorted(headers),
                'inventory_procedures':len(inv['procedures']),'exported_labels':len(labels.splitlines()),
                'headless_command':command,
                'assembly_command':[sys.executable,str(Path(__file__).with_name('ghidra_assemble.py')),
                    str(stage/'constructor-journal.txt'),str(stage/'extract/native/decomp_r1'),str(stage/'game_functions.tsv')],
                'raw_constants_command':[sys.executable,str(Path(__file__).with_name('ghidra_constants.py')),
                    '--game-dir',str(game),'--stage-dir',str(stage)],
                'limits':['No Java/original code is executed by this preparation stage.',
                          'Labels are limited to selected constructors and their direct PDB-addressed call targets; unresolved external calls remain uncertain.',
                          'No PDB prototype application, forced this-pointer typing or whole-image auto-analysis.',
                          'Derived decompilation, rdata dependency closure and consumed defaults require original-field review.',
                          'Full matrix, mode bytecode, particle inputs and permitted physics bridge remain separate requirements.']}
        with (stage/'plan.json').open('x',encoding='utf-8') as f:json.dump(report,f,indent=2)
        return report
    finally:
        if tpi is not None:tpi.mm.close();tpi.fh.close()
        msf.f.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-dir',type=Path,required=True);parser.add_argument('--cache-dir',type=Path,required=True)
    parser.add_argument('--ghidra-dir',type=Path,required=True);parser.add_argument('--class-name',action='append',required=True)
    args=parser.parse_args()
    try:result=prepare(args.game_dir,args.cache_dir,args.class_name,args.ghidra_dir)
    except (OSError,ValueError,KeyError,AssertionError,ImportError) as error:result={'runtime_ready':False,'error':str(error)}
    print(json.dumps(result,indent=2));return 2


if __name__=='__main__':raise SystemExit(main())
