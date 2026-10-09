"""Bounded literal-store constructor reconstruction; never executes original instructions.

This mirrors the existing source reader's zero-object/literal replay contract. Unsupported
instructions/calls are recorded, and output is diagnostic until separately judged for each
consumer. A lexical array append represents the original reader contract, not heap emulation.
"""
from dataclasses import dataclass
import struct

@dataclass(frozen=True)
class Pointer:
    kind: str
    offset: int

UNKNOWN = None


def register(md, reg):
    n=md.reg_name(reg) or ""
    if n.startswith('xmm'):return n
    aliases={'al':'rax','ah':'rax','ax':'rax','eax':'rax','bl':'rbx','bh':'rbx','bx':'rbx','ebx':'rbx',
             'cl':'rcx','ch':'rcx','cx':'rcx','ecx':'rcx','dl':'rdx','dh':'rdx','dx':'rdx','edx':'rdx',
             'sil':'rsi','si':'rsi','esi':'rsi','dil':'rdi','di':'rdi','edi':'rdi',
             'bpl':'rbp','bp':'rbp','ebp':'rbp','spl':'rsp','sp':'rsp','esp':'rsp'}
    if n in aliases:return aliases[n]
    if n.startswith('r') and n[-1:] in ('d','w','b') and n[1:-1].isdigit():return n[:-1]
    return n


class Replay:
    def __init__(self, pe, procedures, size, array_offsets=()):
        import capstone
        if capstone.__version__!='5.0.7':raise ValueError('Requires Capstone5.0.7')
        self.cs=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_64);self.cs.detail=True
        self.op=capstone.x86;self.pe=pe
        self.procedures={}
        for p in procedures:self.procedures.setdefault(p['rva'],[]).append(p)
        self.object=bytearray(size);self.array_offsets=set(array_offsets);self.arrays={}
        self.stores=[];self.unsupported=[];self.calls=[];self.active=[]

    def issue(self, ins, reason):
        self.unsupported.append({'address':hex(ins.address),'asm':ins.mnemonic+' '+ins.op_str,'reason':reason})

    def ctor(self,rva):
        rows=self.procedures.get(rva,[])
        ctors=[]
        for p in rows:
            parts=p['name'].split('::')
            if len(parts)>=2 and parts[-1]==parts[-2] and parts[-1]:ctors.append(p)
        names={p['name'] for p in ctors}
        if len({p['size'] for p in ctors})>1:raise ValueError('Conflicting folded constructor code ranges')
        if ctors:
            return dict(ctors[0],name=ctors[0]['name'] if len(names)==1 else '<folded original constructor body>',aliases=sorted(names))
        return None

    def apply(self,rva,at=0):
        proc=self.ctor(rva)
        if proc is None:raise ValueError('Selected original function is not a PDB constructor')
        if len(self.active)>=32 or rva in self.active:raise ValueError('Constructor recursion unsupported')
        self.active.append(rva)
        regs={'rcx':Pointer('object',at),'rsp':Pointer('stack',0)};stack={}
        self.calls.append({'name':proc['name'],'aliases':proc['aliases'],'rva':hex(rva),'object_offset':at})
        def address(ins,operand):
            m=operand.mem
            if m.base==self.op.X86_REG_RIP:return Pointer('data',ins.address+ins.size+m.disp-self.pe.base)
            base=regs.get(register(self.cs,m.base),0 if not m.base else None)
            index=regs.get(register(self.cs,m.index),0 if not m.index else None)
            if not isinstance(index,int):return None
            shift=m.disp+index*m.scale
            if isinstance(base,Pointer):
                if base.kind=='array':return base # NativeCtor's lexical append is tied to the array header, not heap index.
                return Pointer(base.kind,base.offset+shift)
            if isinstance(base,int):return base+shift
            return None
        def read(ins,operand):
            if operand.type==self.op.X86_OP_IMM:return operand.imm
            if operand.type==self.op.X86_OP_REG:
                v=regs.get(register(self.cs,operand.reg))
                if isinstance(v,Pointer) and operand.size<8:return None
                if isinstance(v,int):
                    shift=8 if self.cs.reg_name(operand.reg) in ('ah','bh','ch','dh') else 0
                    return (v>>shift)&((1<<(operand.size*8))-1)
                if isinstance(v,bytes):return v[:operand.size]
                return v
            if operand.type==self.op.X86_OP_MEM:
                a=address(ins,operand);size=operand.size
                if not isinstance(a,Pointer):return None
                if a.kind=='stack':return stack.get((a.offset,size))
                if a.kind=='object':
                    if size==8 and a.offset in self.array_offsets:return Pointer('array',a.offset)
                    if 0<=a.offset<=a.offset+size<=len(self.object):return int.from_bytes(self.object[a.offset:a.offset+size],'little')
                if a.kind=='data':
                    try:return self.pe.read_rva(a.offset,size)
                    except ValueError:return None
            return None
        def write(ins,operand,value):
            size=operand.size
            if operand.type==self.op.X86_OP_REG:
                key=register(self.cs,operand.reg)
                if isinstance(value,Pointer) and size<8:value=None
                if isinstance(value,bytes) and not key.startswith('xmm'):value=int.from_bytes(value,'little')
                if isinstance(value,int):
                    value&=(1<<(size*8))-1
                    # Legacy 8/16-bit register writes preserve the rest; 32-bit writes zero extend.
                    if size<4 and not key.startswith('xmm'):
                        old=regs.get(key)
                        if not isinstance(old,int):value=None
                        else:
                            shift=8 if self.cs.reg_name(operand.reg) in ('ah','bh','ch','dh') else 0
                            mask=((1<<(size*8))-1)<<shift;value=(old&~mask)|(value<<shift)
                regs[key]=value;return
            if operand.type!=self.op.X86_OP_MEM:return
            dest=address(ins,operand)
            if not isinstance(dest,Pointer):self.issue(ins,'Unknown store destination');return
            if dest.kind=='stack':stack[(dest.offset,size)]=value;return
            if dest.kind=='array':
                # Array index is appended lexically like NativeCtor.gd; only scalar literal writes qualify.
                if isinstance(value,int) and size==4:
                    self.arrays.setdefault(dest.offset,[]).append(value&0xffffffff)
                    self.stores.append({'address':hex(ins.address),'array_offset':dest.offset,'raw':(value&0xffffffff).to_bytes(4,'little').hex()})
                else:self.issue(ins,'Unsupported array store')
                return
            if dest.kind!='object':return
            if isinstance(value,Pointer):
                # Object/asset/data pointers are not scalar defaults and are not fabricated.
                return
            if isinstance(value,int):raw=(value&((1<<(size*8))-1)).to_bytes(size,'little')
            elif isinstance(value,bytes) and len(value)>=size:raw=value[:size]
            else:self.issue(ins,'Unknown object store value');return
            if not 0<=dest.offset<=dest.offset+size<=len(self.object):raise ValueError('Constructor write outside typed object')
            self.object[dest.offset:dest.offset+size]=raw
            self.stores.append({'address':hex(ins.address),'object_offset':dest.offset,'raw':raw.hex()})
        try:
            for ins in self.cs.disasm(self.pe.read_rva(rva,proc['size']),self.pe.base+rva):
                op=ins.operands;name=ins.mnemonic
                if name in ('mov','movss','movsd','movaps','movups','movdqa','movdqu','movd','movq') and len(op)==2:
                    value=read(ins,op[1])
                    if op[0].type==self.op.X86_OP_REG and register(self.cs,op[0].reg).startswith('xmm') and name in ('movss','movsd','movd','movq'):
                        count=4 if name in ('movss','movd') else 8
                        if isinstance(value,int):value=(value&((1<<(count*8))-1)).to_bytes(count,'little')
                        if isinstance(value,bytes):
                            value=value[:count]
                            if op[1].type==self.op.X86_OP_MEM or name in ('movd','movq'):value+=bytes(16-count)
                            else:
                                previous=regs.get(register(self.cs,op[0].reg))
                                if isinstance(previous,bytes) and len(previous)>=16:value+=previous[count:16]
                    write(ins,op[0],value)
                elif name in ('movsx','movsxd','movzx') and len(op)==2:
                    value=read(ins,op[1])
                    if isinstance(value,bytes):value=int.from_bytes(value,'little')
                    if isinstance(value,int) and name!='movzx':
                        sign=1<<(op[1].size*8-1);value=(value^sign)-sign
                    write(ins,op[0],value)
                elif name=='lea' and len(op)==2:write(ins,op[0],address(ins,op[1]))
                elif name in ('xor','xorps','xorpd','pxor') and len(op)==2 and op[0].type==self.op.X86_OP_REG and op[1].type==self.op.X86_OP_REG and op[0].reg==op[1].reg:
                    write(ins,op[0],bytes(op[0].size) if name!='xor' else 0)
                elif name in ('xorps','xorpd','pxor','andps','andpd','pand','orps','orpd','por') and len(op)==2:
                    a,b=read(ins,op[0]),read(ins,op[1]);value=None
                    if isinstance(a,bytes) and isinstance(b,bytes):
                        operation=(lambda x,y:x^y) if name in ('xorps','xorpd','pxor') else (lambda x,y:x&y) if name in ('andps','andpd','pand') else (lambda x,y:x|y)
                        value=bytes(operation(x,y) for x,y in zip(a,b))
                    write(ins,op[0],value)
                elif name in ('unpcklps','unpckhps','movhlps','movlhps','shufps') and len(op)>=2:
                    a,b=read(ins,op[0]),read(ins,op[1]);value=None
                    if isinstance(a,bytes) or isinstance(b,bytes):
                        def lane(raw,i):return raw[i*4:(i+1)*4] if isinstance(raw,bytes) and len(raw)>=(i+1)*4 else None
                        if name=='unpcklps':parts=[lane(a,0),lane(b,0),lane(a,1),lane(b,1)]
                        elif name=='unpckhps':parts=[lane(a,2),lane(b,2),lane(a,3),lane(b,3)]
                        elif name=='movhlps':parts=[lane(b,2),lane(b,3),lane(a,2),lane(a,3)]
                        elif name=='movlhps':parts=[lane(a,0),lane(a,1),lane(b,0),lane(b,1)]
                        else:
                            control=read(ins,op[2]);parts=[lane(a,control&3),lane(a,(control>>2)&3),lane(b,(control>>4)&3),lane(b,(control>>6)&3)] if isinstance(control,int) else [None]
                        prefix=[]
                        for part in parts:
                            if part is None:break
                            prefix.append(part)
                        value=b''.join(prefix) or None
                    write(ins,op[0],value)
                elif name in ('add','sub','and','or','xor','shl','shr','sar') and len(op)==2:
                    a,b=read(ins,op[0]),read(ins,op[1]);value=None
                    if isinstance(a,Pointer) and isinstance(b,int) and name in ('add','sub'):value=Pointer(a.kind,a.offset+(b if name=='add' else -b))
                    elif isinstance(a,int) and isinstance(b,int):
                        value={'add':lambda:a+b,'sub':lambda:a-b,'and':lambda:a&b,'or':lambda:a|b,'xor':lambda:a^b,'shl':lambda:a<<b,'shr':lambda:a>>b,'sar':lambda:((a^(1<<(op[0].size*8-1)))-(1<<(op[0].size*8-1)))>>b}[name]()
                    write(ins,op[0],value)
                elif name=='push' and len(op)==1:
                    sp=regs.get('rsp');regs['rsp']=Pointer('stack',sp.offset-8);stack[(sp.offset-8,8)]=read(ins,op[0])
                elif name=='pop' and len(op)==1:
                    sp=regs.get('rsp');write(ins,op[0],stack.get((sp.offset,8)));regs['rsp']=Pointer('stack',sp.offset+8)
                elif name=='call':
                    target=read(ins,op[0]);target=target-self.pe.base if isinstance(target,int) else None
                    nested=self.ctor(target);this=regs.get('rcx')
                    if nested and isinstance(this,Pointer) and this.kind=='object':self.apply(target,this.offset)
                    else:self.issue(ins,'External/indirect call not replayed')
                    for key in ['rax','rcx','rdx','r8','r9','r10','r11',*[f'xmm{i}' for i in range(6)]]:regs[key]=None
                elif name=='ret':break
                elif name in ('nop','int3','cmp','test'):pass
                elif name.startswith('j'):
                    self.issue(ins,'Control flow not emulated; lexical literal-reader contract only')
                else:
                    self.issue(ins,'Instruction not supported by literal reader')
                    # Do not allow stale destination-register values to become fabricated stores.
                    if op and op[0].type==self.op.X86_OP_REG:regs[register(self.cs,op[0].reg)]=None
        finally:self.active.pop()
        return {'bytes':self.object,'arrays':self.arrays,'stores':self.stores,'calls':self.calls,'unsupported':self.unsupported,
                'runtime_ready':False,'scope':'literal constructor replay diagnostic, not arbitrary x64 execution/full matrix'}
