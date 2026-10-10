"""Typed local-only defaults decoded from actual PDB layouts and literal replay bytes."""
import struct
import math

class Layouts:
    def __init__(self, codeview, tpi):
        self.codeview=codeview;self.tpi=tpi;self.types=codeview.Types(tpi)
        self.byname={};self.skipped_fields=[]
        for ti in tpi.complete.values():self.byname.setdefault(tpi.udt(ti)[1],[]).append(ti)
    def layout(self,name):
        rows=self.byname.get(name,[])
        if len(rows)!=1:raise ValueError('Missing/ambiguous original complete type '+name)
        return self.types.layout(rows[0])
    def array_offsets(self,name,at=0,active=()):
        if name in active:raise ValueError('Recursive by-value PDB type')
        result=set();layout=self.layout(name)
        for base in layout['bases']:result.update(self.array_offsets(base['name'],at+base['off'],(*active,name)))
        for f in layout['members']:
            typ=self.types.decl(f['ti']);o=at+f['off']
            if typ.startswith('TArray<float'):result.add(o)
            elif typ.startswith('F') and '*' not in typ and '<' not in typ and typ in self.byname:
                result.update(self.array_offsets(typ,o,(*active,name)))
        return result
    def decode(self,name,raw,arrays,at=0,active=()):
        if name in active:raise ValueError('Recursive by-value PDB type')
        layout=self.layout(name);out={}
        for base in layout['bases']:out.update(self.decode(base['name'],raw,arrays,at+base['off'],(*active,name)))
        for f in layout['members']:
            typ=self.types.decl(f['ti']);o=at+f['off'];key=f['name'];size=self.types.size(f['ti'])
            if o<0 or o+size>len(raw):raise ValueError('PDB field outside reconstructed object')
            bits=self.codeview.bitinfo(self.types,f['ti'])
            if bits:
                value=int.from_bytes(raw[o:o+size],'little');value=(value>>bits[2])&((1<<bits[1])-1)
                out[key]=bool(value) if self.types.decl(bits[0])=='bool' else value
            elif typ=='float':out[key]=struct.unpack_from('<f',raw,o)[0]
            elif typ=='int32':out[key]=struct.unpack_from('<i',raw,o)[0]
            elif typ=='bool':out[key]=bool(raw[o])
            elif typ=='uint8' or (typ.startswith('E') and '*' not in typ and '<' not in typ):out[key]=int.from_bytes(raw[o:o+size],'little')
            elif typ.startswith('TArray<float'):out[key]=[struct.unpack('<f',v.to_bytes(4,'little'))[0] for v in arrays.get(o,[])]
            elif typ in ('FText','FName'):
                self.skipped_fields.append({'class':name,'field':key,'type':typ,'object_offset':o,
                                            'reason':'Opaque runtime text/name semantics not reconstructed; no diagnostic value invented'})
            elif typ.startswith('F') and '*' not in typ and '<' not in typ and typ in self.byname:
                out[key]=self.decode(typ,raw,arrays,o,(*active,name))
            else:self.skipped_fields.append({'class':name,'field':key,'type':typ,'object_offset':o})
        return out

    def decode_reviewed(self,name,raw,arrays,known,stores,at=0,active=(),path=''):
        """Decode only fields whose final bytes are known, retaining a field audit.

        This does not approve a constructor globally. Pointer values are accepted
        only as an actual zero/null; opaque name/text and nonempty unsupported
        containers never acquire invented defaults.
        """
        if name in active:raise ValueError('Recursive by-value PDB type')
        if len(known)!=len(raw):raise ValueError('Known-byte mask has wrong length')
        layout=self.layout(name);out={};review=[]
        for base in layout['bases']:
            values,rows=self.decode_reviewed(base['name'],raw,arrays,known,stores,at+base['off'],(*active,name),path)
            out.update(values);review.extend(rows)
        for field in layout['members']:
            typ=self.types.decl(field['ti']);o=at+field['off'];size=self.types.size(field['ti']);key=field['name']
            if o<0 or o+size>len(raw):raise ValueError('PDB field outside reconstructed object')
            identity=path+key;row={'field':identity,'declaring_class':name,'type':typ,'object_offset':o,'size':size,'accepted':False}
            evidence=[s for s in stores if 'object_offset' in s and o<s['object_offset']+len(bytes.fromhex(s['raw'])) and s['object_offset']<o+size]
            row['stores']=evidence
            nested=typ.startswith('F') and '*' not in typ and '<' not in typ and typ in self.byname and typ not in ('FText','FName')
            if nested:
                values,rows=self.decode_reviewed(typ,raw,arrays,known,stores,o,(*active,name),identity+'.')
                out[key]=values;review.extend(rows);continue
            reason=''
            value=None
            if typ in ('FText','FName'):reason='Opaque runtime name/text not reconstructed'
            elif not all(known[o:o+size]):reason='Final field bytes depend on an unknown or conditional store'
            else:
                bits=self.codeview.bitinfo(self.types,field['ti'])
                if bits:
                    value=(int.from_bytes(raw[o:o+size],'little')>>bits[2])&((1<<bits[1])-1)
                    if self.types.decl(bits[0])=='bool':value=bool(value)
                elif typ=='float':
                    value=struct.unpack_from('<f',raw,o)[0]
                    if not math.isfinite(value):reason='Nonfinite native float'
                elif typ=='int32':value=struct.unpack_from('<i',raw,o)[0]
                elif typ=='bool':
                    if raw[o] not in (0,1):reason='Native bool byte is not zero or one'
                    else:value=bool(raw[o])
                elif typ=='uint8' or (typ.startswith('E') and '*' not in typ and '<' not in typ):value=int.from_bytes(raw[o:o+size],'little')
                elif typ.startswith('TArray<float'):
                    words=arrays.get(o,[]);count=struct.unpack_from('<i',raw,o+8)[0]
                    if count!=len(words):reason='Native array count differs from recovered literal elements'
                    elif any(s.get('conditional') for s in stores if s.get('array_offset')==o):reason='Native array element is conditionally written'
                    else:
                        value=[struct.unpack('<f',v.to_bytes(4,'little'))[0] for v in words]
                        if not all(math.isfinite(v) for v in value):reason='Nonfinite native float array'
                        row['element_stores']=[s for s in stores if s.get('array_offset')==o]
                elif '*' in typ or typ.startswith('TSubclassOf<'):
                    if any(raw[o:o+size]):reason='Non-null native pointer semantics unresolved'
                    else:value={};row['encoding']='actual-native-null-reference'
                elif typ.startswith('TArray<'):
                    if any(raw[o:o+min(size,12)]):reason='Nonempty native container element decoder unavailable'
                    else:value=[];row['encoding']='actual-native-empty-array'
                elif typ.startswith('TSet<'):
                    # Read cardinality by original PDB field identities rather
                    # than mistaking allocator sentinels (e.g. -1/128) for data.
                    try:
                        setlayout=self.types.layout(field['ti'])
                        elements=next(v for v in setlayout['members'] if v['name']=='Elements')
                        sparse=self.types.layout(elements['ti'])
                        data=next(v for v in sparse['members'] if v['name']=='Data')
                        free=next(v for v in sparse['members'] if v['name']=='NumFreeIndices')
                        array=self.types.layout(data['ti'])
                        num=next(v for v in array['members'] if v['name']=='ArrayNum')
                        a=o+elements['off']+data['off']+num['off'];b=o+elements['off']+free['off']
                        count=struct.unpack_from('<i',raw,a)[0];freecount=struct.unpack_from('<i',raw,b)[0]
                        if not all(known[a:a+4]) or not all(known[b:b+4]) or count!=0 or freecount!=0:
                            reason='Native set is nonempty or its count is unknown'
                        else:value=[];row['encoding']='actual-native-empty-typed-set';row['count_offsets']=[a,b]
                    except (ValueError,KeyError,StopIteration,AssertionError):reason='Native set cardinality layout unavailable'
                else:reason='Unsupported native field type'
            if reason:row['reason']=reason
            else:
                out[key]=value;row['accepted']=True
                row['provenance']='original-constructor-store' if evidence else 'verified-UObject-zero-allocation'
            review.append(row)
        return out,review
