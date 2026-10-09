"""Source-only DBI procedure indexing for constructor decoding from the installed PDB."""
import struct

# CodeView procedure records share parent/end/next/length/debug/type/offset/segment fields.
PROC_KINDS = {0x110f, 0x1110, 0x1146, 0x1147}


def modules(msf):
    dbi=msf.read(3)
    if len(dbi)<64 or struct.unpack_from('<i',dbi)[0]!=-1: raise ValueError('Unsupported DBI header')
    size=struct.unpack_from('<i',dbi,24)[0]
    if size<0 or 64+size>len(dbi): raise ValueError('DBI module substream outside bounds')
    data=dbi[64:64+size];pos=0;result=[]
    while pos<len(data):
        if pos+64>len(data): raise ValueError('Truncated module descriptor')
        stream=struct.unpack_from('<H',data,pos+34)[0]
        symbols=struct.unpack_from('<I',data,pos+36)[0]
        start=pos+64
        try:
            first=data.index(b'\0',start);second=data.index(b'\0',first+1)
        except ValueError: raise ValueError('Truncated module/object name') from None
        module=data[start:first].decode('utf-8','replace');obj=data[first+1:second].decode('utf-8','replace')
        if stream!=0xffff and (stream>=len(msf.sizes) or symbols>msf.sizes[stream]): raise ValueError('Module symbol stream outside bounds')
        result.append({'module':module,'object':obj,'stream':stream,'symbol_bytes':symbols})
        pos=(second+4)&~3
    return result


def procedures(msf, pe, module):
    stream=module['stream'];length=module['symbol_bytes']
    if stream==0xffff or length==0:return []
    data=msf.read(stream)
    if length<4 or len(data)<length or struct.unpack_from('<I',data)[0]!=4:raise ValueError('Unsupported module symbol signature')
    data=data[:length];pos=4;result=[]
    while pos<len(data):
        if pos+4>len(data): raise ValueError('Truncated procedure symbol header')
        size,kind=struct.unpack_from('<HH',data,pos);end=pos+size+2
        if size<2 or end>len(data): raise ValueError('Procedure symbol record outside bounds')
        if kind in PROC_KINDS:
            if size<38:raise ValueError('Truncated procedure record')
            code_size=struct.unpack_from('<I',data,pos+16)[0]
            offset=struct.unpack_from('<I',data,pos+32)[0]
            section=struct.unpack_from('<H',data,pos+36)[0]
            name=data[pos+39:end].split(b'\0',1)[0].decode('utf-8','replace')
            if not 1<=section<=len(pe.sections):raise ValueError('Procedure section outside original PE')
            rva=pe.sections[section-1][1]+offset
            if code_size:pe.read_rva(rva,code_size) # every selected range must be file-backed
            result.append({'name':name,'rva':rva,'size':code_size,'type_index':struct.unpack_from('<I',data,pos+28)[0],
                           'module':module['module'],'object':module['object']})
        pos=end
    return result


def inventory(msf,pe):
    out=[]
    for module in modules(msf):
        # PDB object identity, rather than a shipped class/address/default table.
        obj=module['object'].replace('/', '\\')
        if '\\Shipping\\Mordhau\\Module.Mordhau.' in obj and obj.endswith('.cpp.obj'):
            out.extend(procedures(msf,pe,module))
    if not out:raise ValueError('No original Mordhau procedures indexed')
    constructors=[]
    for p in out:
        parts=p['name'].split('::')
        if len(parts)>=2 and parts[-1]==parts[-2] and parts[-1]:constructors.append(p)
    if not constructors:raise ValueError('No original constructors indexed')
    return {'procedures':out,'constructors':constructors}
