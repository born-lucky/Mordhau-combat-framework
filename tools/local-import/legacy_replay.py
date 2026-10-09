"""Read-only legacy source-reader oracle; never supplies runtime/import defaults.

A Python translation of the existing NativeCtor.gd literal/decomp contract for comparison
when the independent headless tool cannot start. Original decompiler data remains local.
"""
import csv
from pathlib import Path
import re
import struct

SIZE={'undefined1':1,'byte':1,'char':1,'bool':1,'undefined2':2,'short':2,'ushort':2,
      'undefined4':4,'int':4,'uint':4,'float':4,'undefined8':8,'longlong':8,'ulonglong':8}

class Oracle:
    def __init__(self,extract):
        self.root=Path(extract)/'native';self.layouts={};self.rows={};self.calls=[]
        with (self.root/'rdata.tsv').open(encoding='utf-8') as f:
            for row in csv.DictReader(f,delimiter='\t'):self.rows[int(row['va'],16)]=row
    def layout(self,cls):
        if cls not in self.layouts:
            text=(self.root/'types'/f'{cls}.h').read_text();size=re.search(r'sizeof = 0x([0-9a-f]+)',text)
            base=re.search(r'(?m)^(?:class|struct) \w+ : public (\w+)',text)
            fields=[dict(off=int(m[1],16),type=m[2],name=m[3]) for m in re.finditer(r'(?m)^\t/\* 0x([0-9a-f]+) \*/ (.+) (\w+);',text)]
            self.layouts[cls]=dict(size=int(size[1],16) if size else 0,base=base[1] if base else '',fields=fields)
        return self.layouts[cls]
    def has(self,cls):return (self.root/'types'/f'{cls}.h').is_file()
    def body(self,cls):
        folder='decomp_r1' if (self.root/'decomp_r1').is_dir() else 'decomp';p=self.root/folder/f'{cls}.cpp'
        if not p.exists():return ''
        text=p.read_text();key=f'// {cls}::{cls}  rva=';start=text.find(key)
        if start<0:return ''
        end=text.find('\n// ',start+len(key));return text[start:end if end>=0 else len(text)]
    def lit(self,s):
        s=s.strip()
        try:return int(s,16) if s.lstrip('-').startswith('0x') else int(s)
        except ValueError:return None
    def rhs(self,s,locals):
        s=s.strip();value=self.lit(s)
        if value is not None:return value
        if s in locals:return locals[s]
        if re.fullmatch('_DAT_[0-9a-fA-F]+',s):
            row=self.rows.get(int(s[5:],16));return int.from_bytes(bytes.fromhex(row['raw'])[:8],'little') if row else None
        m=re.fullmatch(r'\(\w+\)(\w+) << 0x20',s)
        if m:
            value=self.rhs(m[1],locals)
            if value is not None:return (value&0xffffffff)<<32
        m=re.fullmatch(r'CONCAT44\((.+),(.+)\)',s)
        if m:
            hi,lo=self.rhs(m[1],locals),self.rhs(m[2],locals)
            if hi is not None and lo is not None:return (hi<<32)|(lo&0xffffffff)
        return None
    def apply(self,cls,buf,at,arrays,active=()):
        if cls in active:raise ValueError('Legacy recursive constructor')
        base=self.layout(cls)['base']
        if base and self.has(base):self.apply(base,buf,at,arrays,(*active,cls))
        body=self.body(cls)
        if not body:return
        self.calls.append(cls)
        sig=re.search(r'\((\w+) \*param_1',body);element=sig[1] if sig else 'undefined1';esz=SIZE.get(element,1)
        decl={m[2]:m[1] for m in re.finditer(r'(?m)^\s+(\w+) \*(\w+);',body)};alias={};whole=set();locals={}
        def put(o,t,v):
            if o is None or v is None or t not in SIZE:return
            size=SIZE[t];off=at+o
            if 0<=off<=off+size<=len(buf):buf[off:off+size]=(v&((1<<(size*8))-1)).to_bytes(size,'little')
        for line in body.splitlines():
            line=line.strip()
            m=re.fullmatch(r'(\w+) = \(\w+ \*\)\(param_1 \+ (\w+)\);',line)
            if m:alias[m[1]]=self.lit(m[2])*esz;whole.discard(m[1]);continue
            m=re.fullmatch(r'(\w+) = (?:\(\w+ \*\))?(.+);',line)
            if m and m[1] in decl:
                if m[2]=='param_1' or m[2] in whole:whole.add(m[1]);continue
                whole.discard(m[1])
            elif m:whole.discard(m[1])
            m=re.fullmatch(r'([a-z]Var\d+|[a-z]Stack_\w+) = (.+);',line)
            if m:locals[m[1]]=self.rhs(m[2],locals);continue
            if whole:
                m=re.fullmatch(r'\*\((\w+) \*\)\(\(longlong\)(\w+) \+ (\w+)\) = (.+);',line);bytes_=m is not None
                if m is None:m=re.fullmatch(r'\*\((\w+) \*\)\((\w+) \+ (\w+)\) = (.+);',line)
                if m and m[2] in whole and m[1] in SIZE:
                    o=self.lit(m[3]);v=self.rhs(m[4],locals)
                    if o is not None:put(o*(1 if bytes_ else SIZE.get(decl.get(m[2],''),1)),m[1],v)
                    continue
            m=re.fullmatch(r'(\w+)__(\w+)\((?:\(longlong\))?param_1 \+ (\w+)\);',line)
            if m and m[1]==m[2] and self.has(m[1]):
                self.apply(m[1],buf,at+self.lit(m[3])*(1 if '(longlong)param_1' in line else esz),arrays,(*active,cls));continue
            m=re.fullmatch(r'\*\((\w+) \*\)\(\*(?:\(longlong \*\)\(param_1 \+ (\w+)\)|(\w+)) \+ \(longlong\)\w+ \* \d+\) = (-?\w+);',line)
            if m:
                value=self.lit(m[4]);o=self.lit(m[2])*esz if m[2] else alias.get(m[3])
                if value is not None and o is not None:arrays.setdefault(at+o,[]).append(value)
                continue
            m=re.fullmatch(r'\*\((\w+) \*\)\(\(longlong\)param_1 \+ (\w+)\) = (.+);',line)
            if m:put(self.lit(m[2]),m[1],self.rhs(m[3],locals));continue
            m=re.fullmatch(r'\*\((\w+) \*\)\(param_1 \+ (\w+)\) = (.+);',line)
            if m:
                o=self.lit(m[2]);put(o*esz if o is not None else None,m[1],self.rhs(m[3],locals));continue
            m=re.fullmatch(r'param_1\[(\w+)\] = (.+);',line)
            if m:
                o=self.lit(m[1]);put(o*esz if o is not None else None,element,self.rhs(m[2],locals));continue
            if line.startswith('*param_1 = '):put(0,element,self.lit(line[len('*param_1 = '):-1]))
    def decode(self,cls,buf,at,arrays):
        layout=self.layout(cls);out={}
        if layout['base'] and self.has(layout['base']):out.update(self.decode(layout['base'],buf,at,arrays))
        for field in layout['fields']:
            o=at+field['off'];typ=field['type'];key=field['name']
            if o>=len(buf):continue
            if typ=='float':out[key]=struct.unpack_from('<f',buf,o)[0]
            elif typ=='int32':out[key]=struct.unpack_from('<i',buf,o)[0]
            elif typ=='bool':out[key]=bool(buf[o])
            elif typ=='uint8' or (typ.startswith('E') and '<' not in typ and '*' not in typ):out[key]=buf[o]
            elif typ in ('FVector2D','FVector','FRotator'):
                axes={'FVector2D':['X','Y'],'FVector':['X','Y','Z'],'FRotator':['Pitch','Yaw','Roll']}[typ]
                out[key]={k:struct.unpack_from('<f',buf,o+i*4)[0] for i,k in enumerate(axes)}
            elif typ.startswith('TArray<float'):out[key]=[struct.unpack('<f',(v&0xffffffff).to_bytes(4,'little'))[0] for v in arrays.get(o,[])]
            elif typ.startswith('F') and '*' not in typ and '<' not in typ and self.has(typ):out[key]=self.decode(typ,buf,o,arrays)
        return out
    def defaults(self,cls):
        buf=bytearray(max(self.layout(cls)['size'],8));arrays={};self.apply(cls,buf,0,arrays)
        return self.decode(cls,buf,0,arrays)
