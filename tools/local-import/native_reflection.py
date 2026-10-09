"""Read original UHT enum tables through structured installed-PDB data symbols.

No enum values, exported records or private label files are inputs to this module.
The FEnumParams field offsets must come from the original complete CodeView type.
"""
import struct

DATA_KINDS={0x110c,0x110d} # S_LDATA32, S_GDATA32 (type, section offset, section, name)

def data_records(raw,pe,start=0,origin=''):
    result=[];pos=start
    while pos<len(raw):
        if pos+4>len(raw):raise ValueError('Truncated data symbol header')
        size,kind=struct.unpack_from('<HH',raw,pos);end=pos+size+2
        if size<2 or end>len(raw):raise ValueError('Data symbol record outside stream')
        if kind in DATA_KINDS:
            if size<13:raise ValueError('Truncated typed data symbol')
            ti,offset,section=struct.unpack_from('<IIH',raw,pos+4)
            if not 1<=section<=len(pe.sections):raise ValueError('Data symbol section outside PE')
            name=raw[pos+14:end]
            if b'\0' not in name:raise ValueError('Unterminated data symbol name')
            name=name.split(b'\0',1)[0].decode('utf-8')
            result.append({'name':name,'rva':pe.sections[section-1][1]+offset,
                           'type_index':ti,'record_kind':hex(kind),'record_offset':pos,'origin':origin})
        pos=end
    return result

def collect(msf,pe,module_parser):
    dbi=msf.read(3)
    if len(dbi)<64 or struct.unpack_from('<i',dbi)[0]!=-1:raise ValueError('Unsupported DBI header')
    stream=struct.unpack_from('<H',dbi,20)[0]
    if stream>=len(msf.sizes):raise ValueError('Global symbol stream outside bounds')
    result=data_records(msf.read(stream),pe,origin='DBI.SymRecordStream')
    for module in module_parser.modules(msf):
        obj=module['object'].replace('/','\\')
        if '\\Shipping\\Mordhau\\Module.Mordhau.' not in obj or not obj.endswith('.cpp.obj'):continue
        size=module['symbol_bytes'];index=module['stream']
        if not size or index==0xffff:continue
        data=msf.read(index)
        if size<4 or len(data)<size or struct.unpack_from('<I',data)[0]!=4:
            raise ValueError('Unsupported module data symbol signature')
        result.extend(data_records(data[:size],pe,4,module['object']))
    return result

def read_enum(pe,records,label,params_layout,enumerator_layout):
    """Exact selected UHT identity, file-backed original arrays, typed field count."""
    def symbol(suffix):
        names={label+'::'+suffix,label+'_Statics::'+suffix}
        matches=[r for r in records if r['name'] in names]
        if not matches or len({r['rva'] for r in matches})!=1:
            raise ValueError('Missing/ambiguous original enum '+suffix)
        return matches[0]['rva'],matches
    enumerators,enum_symbols=symbol('Enumerators');params,param_symbols=symbol('EnumParams')
    fields={f['name']:f for f in params_layout['members']}
    if not {'EnumeratorParams','NumEnumerators'}<=fields.keys():raise ValueError('Original FEnumParams fields unavailable')
    ptr_offset=fields['EnumeratorParams']['off'];count_offset=fields['NumEnumerators']['off']
    size=params_layout['size']
    if not 0<=ptr_offset<=ptr_offset+8<=size or not 0<=count_offset<=count_offset+4<=size:
        raise ValueError('Original FEnumParams member outside layout')
    raw=pe.read_rva(params,size);pointer=struct.unpack_from('<Q',raw,ptr_offset)[0]
    count=struct.unpack_from('<i',raw,count_offset)[0]
    if pointer!=pe.base+enumerators or not 0<count<=65536:raise ValueError('Original enum pointer/count invalid')
    entry_fields={f['name']:f for f in enumerator_layout['members']}
    if not {'NameUTF8','Value'}<=entry_fields.keys():raise ValueError('Original FEnumeratorParam fields unavailable')
    stride=enumerator_layout['size'];name_offset=entry_fields['NameUTF8']['off'];value_offset=entry_fields['Value']['off']
    if not 0<=name_offset<=name_offset+8<=stride or not 0<=value_offset<=value_offset+8<=stride:
        raise ValueError('Original FEnumeratorParam member outside layout')
    rows=[];seen=set()
    for i in range(count):
        entry=pe.read_rva(enumerators+stride*i,stride)
        pointer=struct.unpack_from('<Q',entry,name_offset)[0];value=struct.unpack_from('<q',entry,value_offset)[0]
        name=pe.read_rva(pointer-pe.base,256);nul=name.find(b'\0')
        if nul<0:raise ValueError('Unterminated original enumerator')
        name=name[:nul].decode('utf-8')
        if not name or name in seen:raise ValueError('Missing/duplicate original enumerator name')
        seen.add(name);rows.append({'name':name,'value':value})
    return {'label':label,'rows':rows,'symbols':enum_symbols+param_symbols,
            'params_rva':params,'enumerators_rva':enumerators,'params_layout':params_layout,'enumerator_layout':enumerator_layout}
