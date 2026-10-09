"""Generate the runtime's native cache from the user's pinned EXE/PDB, never private exports.

This is one real import stage. It does not generate the spec matrix, shaders, body capsules,
or native bridge and never declares the complete rewrite ready.
"""
from __future__ import annotations
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import struct

EXE_SHA1 = 'dfe6f4fcb8e8a4603198025e13438b10062b0c4a'
PDB_SHA1 = '0e9abdd57f9e0831db63c103237577bdea356991'
BUILD = '702625635'
# Read locations and type identifiers are decoder recipes, not cached asset/default values.
FLOAT_RVA = 0x406A414
NAME_RVAS = (0x4317D48, 0x4317BA0, 0x4317B90, 0x4317BB0, 0x4317BD0, 0x4317BC0, 0x4317BD8)
MOTIONS = ('UAttackMotion', 'UBlockedMotion', 'UCouchedAttackMotion', 'UDisarmedMotion', 'UFeintedMotion',
           'UFlinchMotion', 'UIdleMotion', 'UKickMotion', 'UMordhauMotion', 'UParryMotion', 'UStabMotion', 'UStrikeMotion', 'UStunMotion')
DLL_NAMES = ('PxFoundation_x64', 'PxPvdSDK_x64', 'PhysX3Common_x64', 'PhysX3_x64', 'PhysX3Cooking_x64')


class ImportBlocked(ValueError):
    pass


def digest(path, algorithm='sha1'):
    h = hashlib.new(algorithm)
    with Path(path).open('rb') as f:
        while b := f.read(1 << 20):
            h.update(b)
    return h.hexdigest()


class PE:
    def __init__(self, path):
        self.path = Path(path)
        with self.path.open('rb') as f:
            head = f.read(64)
            if len(head) != 64 or head[:2] != b'MZ': raise ImportBlocked('Missing/truncated MZ header')
            off = struct.unpack_from('<I', head, 60)[0]
            if off < 64 or off+26 > self.path.stat().st_size: raise ImportBlocked('PE header outside file')
            f.seek(off); nt = f.read(24)
            if nt[:4] != b'PE\0\0' or struct.unpack_from('<H',nt,4)[0] != 0x8664: raise ImportBlocked('Original must be AMD64 PE')
            count, size = struct.unpack_from('<H',nt,6)[0], struct.unpack_from('<H',nt,20)[0]
            optional = f.read(size)
            if size < 112 or len(optional) != size or struct.unpack_from('<H',optional)[0] != 0x20b:
                raise ImportBlocked('Original must have a complete PE32+ optional header')
            self.base = struct.unpack_from('<Q',optional,24)[0]
            self.sections = []
            for _ in range(count):
                section = f.read(40)
                if len(section) != 40: raise ImportBlocked('Truncated section table')
                name = section[:8].rstrip(b'\0').decode('ascii')
                virtual, rva, rawsize, raw = struct.unpack_from('<IIII',section,8)
                if raw+rawsize > self.path.stat().st_size: raise ImportBlocked('Section raw range outside file')
                self.sections.append((name,rva,virtual,raw,rawsize))

    def read_rva(self, rva, length):
        if length < 0: raise ImportBlocked('Negative read size')
        for _, start, _, raw, size in self.sections:
            if start <= rva and rva+length <= start+size:
                with self.path.open('rb') as f:
                    f.seek(raw+rva-start); b=f.read(length)
                if len(b)==length:return b
        raise ImportBlocked(f'No file-backed PE range for RVA {rva:x}+{length:x}')

    def cstring(self,rva):
        b=self.read_rva(rva,256)
        end=b.find(b'\0')
        if end < 0:raise ImportBlocked('Unterminated native name literal')
        name=b[:end].decode('ascii')
        if not re.fullmatch(r'[A-Za-z0-9_]+',name):raise ImportBlocked('Unsupported native name literal')
        return name


def local_input(root,relative):
    root=Path(root).resolve(strict=True)
    p=(root/relative).resolve(strict=True)
    if not p.is_relative_to(root) or not p.is_file() or p.stat().st_size==0:
        raise ImportBlocked(f'Invalid/missing input under selected installation: {relative}')
    return p


def verify_install(game):
    game=Path(game).resolve(strict=True)
    exe=local_input(game,'Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe')
    pdb=local_input(game,'Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.pdb')
    pe=PE(exe)
    if digest(exe)!=EXE_SHA1:raise ImportBlocked('Unsupported original EXE hash')
    if digest(pdb)!=PDB_SHA1:raise ImportBlocked('Missing/unsupported original PDB; no private symbol-cache fallback')
    for marker in ('version.txt','installedversion','installedversion.txt'):
        if (game/marker).exists() and local_input(game,marker).read_text().strip()!=BUILD:raise ImportBlocked('Unsupported original version')
    local_input(game,'Mordhau/Content/Paks/pakchunk0-WindowsClient.pak')
    for name in DLL_NAMES:PE(local_input(game,f'Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015/{name}.dll'))
    return game,exe,pdb,pe


def codeview_module():
    p=Path(__file__).with_name('codeview_types.py')
    spec=importlib.util.spec_from_file_location('local_codeview_types',p)
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
    return m


def public_initializers(msf, pe):
    """DBI SymRecordStream contains structured S_PUB32 records, not a raw string search."""
    dbi=msf.read(3)
    if len(dbi)<64 or struct.unpack_from('<i',dbi)[0]!=-1:raise ImportBlocked('Unsupported PDB DBI header')
    stream=struct.unpack_from('<H',dbi,20)[0]
    if stream>=len(msf.sizes):raise ImportBlocked('PDB symbol stream index invalid')
    data=msf.read(stream)
    out=[];pos=0
    while pos<len(data):
        if pos+4>len(data):raise ImportBlocked('Truncated symbol header')
        length,kind=struct.unpack_from('<HH',data,pos)
        end=pos+2+length
        if length<2 or end>len(data):raise ImportBlocked('PDB symbol record outside stream')
        if kind==0x110E: # S_PUB32: flags, offset, section, zero-terminated decorated symbol
            if length<13:raise ImportBlocked('Truncated public symbol')
            flags,offset,section=struct.unpack_from('<IIH',data,pos+4)
            name=data[pos+14:end].split(b'\0',1)[0].decode('utf-8','replace')
            match=re.match(r'^\?\?__E(NAME_[A-Za-z0-9_]+)@@',name)
            if match and flags&2:
                if not 1<=section<=len(pe.sections):raise ImportBlocked('Initializer section invalid')
                out.append((match[1],pe.sections[section-1][1]+offset))
        pos=end
    if not out:
        # Local initializers are private procedure records in this original PDB, not linker publics.
        symbols_spec=importlib.util.spec_from_file_location('local_native_symbols',Path(__file__).with_name('native_symbols.py'))
        symbols=importlib.util.module_from_spec(symbols_spec);symbols_spec.loader.exec_module(symbols)
        for proc in symbols.inventory(msf,pe)['procedures']:
            match=re.fullmatch(r"`dynamic initializer for 'NAME_([A-Za-z0-9_]+)''",proc['name'])
            if match:out.append(('NAME_'+match[1],proc['rva']))
    if not out:raise ImportBlocked('No structured original NAME initializer procedures found')
    return out


def bind_names(pe,initializers):
    import capstone
    if capstone.__version__!='5.0.7':raise ImportBlocked('Native decoder requires capstone 5.0.7')
    md=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_64);md.detail=True
    wanted={rva:pe.cstring(rva) for rva in NAME_RVAS}
    found={rva:set() for rva in wanted}
    for name,rva in initializers:
        field=name.removeprefix('NAME_')
        candidates=[a for a,text in wanted.items() if text.casefold()==field.casefold()]
        if not candidates:continue
        terminal=False
        for ins in md.disasm(pe.read_rva(rva,96),pe.base+rva):
            for operand in ins.operands:
                if operand.type==capstone.x86.X86_OP_MEM and operand.mem.base==capstone.x86.X86_REG_RIP:
                    target=ins.address+ins.size+operand.mem.disp-pe.base
                    if target in candidates:found[target].add((field,rva))
            if ins.mnemonic in ('ret','jmp'):
                terminal=True;break
        if not terminal:raise ImportBlocked(f'NAME initializer has no bounded terminal: {name}')
    for rva,bindings in found.items():
        if not bindings or len({field for field,_ in bindings})!=1:
            raise ImportBlocked(f'Native name initializer identity is ambiguous/missing at {rva:x}')
    return {rva:(next(iter(values))[0],tuple(sorted({at for _,at in values}))) for rva,values in found.items()}


def constant_tsv(pe,names):
    rows=['va\trdata_off\twidth\traw\tf32\tf64\ti32\ti64\tfuncs']
    rdata=next((s for s in pe.sections if s[0]=='.rdata'),None)
    if rdata is None:raise ImportBlocked('Original has no .rdata section')
    for rva in sorted((FLOAT_RVA,*NAME_RVAS)):
        width=4 if rva==FLOAT_RVA else 8
        b=pe.read_rva(rva,8)
        if rva in names:funcs=f"`dynamic initializer for 'NAME_{names[rva][0]}''"
        else:funcs='local importer: direct original PE read; no cached numeric value'
        values=(pe.base+rva,rva-rdata[1],width,b[:width].hex(),*struct.unpack('<f',b[:4]),*struct.unpack('<d',b),*struct.unpack('<i',b[:4]),*struct.unpack('<q',b),funcs)
        rows.append(f'{values[0]:x}\t{values[1]:x}\t'+'\t'.join(map(str,values[2:])))
    return ('\n'.join(rows)+'\n').encode()


def headers(msf,stage):
    m=codeview_module();tpi_path=msf.export(2,stage/'tpi.bin');tpi=m.Tpi(tpi_path,True)
    types=m.Types(tpi);byname={}
    for ti in tpi.complete.values():
        byname.setdefault(tpi.udt(ti)[1],set()).add(ti)
    out={}
    try:
        for name in MOTIONS:
            candidates=byname.get(name,set())
            if len(candidates)!=1:raise ImportBlocked(f'Motion PDB complete-type identity ambiguous/missing: {name}')
            layout=types.layout(next(iter(candidates)))
            if len(layout['bases'])!=1 or layout['bases'][0]['off']!=0:raise ImportBlocked(f'Unsupported native primary ancestry: {name}')
            out[f'extract/native/types/{name}.h']=m.header(types,layout,None).encode()
    finally:
        tpi.mm.close();tpi.fh.close()
    return out


def commit_generated(cache,products):
    """Never overwrite divergent existing imports; compare every destination before writing any."""
    cache=Path(cache).resolve(strict=True)
    destinations=[]
    for relative,data in products.items():
        parts=Path(relative).parts
        if Path(relative).is_absolute() or '..' in parts:raise ImportBlocked('Invalid generated relative path')
        dst=cache/relative
        # Resolve parents including existing junctions before creating/writing outputs.
        if not dst.resolve().is_relative_to(cache):raise ImportBlocked('Generated output escapes cache')
        if dst.exists() and dst.read_bytes()!=data:raise ImportBlocked(f'Existing local import differs; preserved: {relative}')
        destinations.append((dst,data))
    for dst,data in destinations:
        if not dst.exists():
            dst.parent.mkdir(parents=True,exist_ok=True)
            with dst.open('xb') as out:out.write(data)
    return [{'path':str(dst),'sha256':digest(dst,'sha256'),'size':len(data)} for dst,data in destinations]


def generate(game,cache):
    # Verify BOTH original inputs before creating even the work directory.
    game,exe,pdb,pe=verify_install(game)
    cache=Path(cache).expanduser().resolve()
    if cache==game or cache.is_relative_to(game):raise ImportBlocked('Import cache must be outside original installation')
    source=Path(__file__).resolve().parents[2]
    if cache.is_relative_to(source) and cache.relative_to(source).parts[:1] not in [('cache',),('build',),('state',)]:
        raise ImportBlocked('Generated game records must stay outside published source directories')
    cache.mkdir(parents=True,exist_ok=True)
    stage=cache/'stages'/('native-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    stage.mkdir(parents=True,exist_ok=False)
    m=codeview_module();msf=m.Msf(pdb)
    try:
        names=bind_names(pe,public_initializers(msf,pe))
        products={'extract/native/rdata.tsv':constant_tsv(pe,names),**headers(msf,stage)}
    finally:msf.f.close()
    outputs=commit_generated(cache,products)
    receipt={'stage':'native-cache','complete':True,'runtime_ready':False,'exe_sha1':EXE_SHA1,'pdb_sha1':PDB_SHA1,
             'scope':'8 original constant/name rows +13 actual PDB motion headers, locally generated only',
             'outputs':outputs,'name_bindings':{hex(pe.base+rva):{'field':name,'initializer_rvas':[hex(v) for v in at]} for rva,(name,at) in names.items()},
             'remaining':['full generated spec matrix','local body shadow capsule records','licensed original-ABI bridge build']}
    with (stage/'receipt.json').open('x',encoding='utf-8') as out:json.dump(receipt,out,indent=2)
    return receipt
