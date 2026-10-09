"""Read all importer-source named constants from verified original PE ranges.

Only decoder addresses are collected from source calls; no numeric game value is embedded.
The generated table stays local. Constructor .rdata reads do not depend on this table.
"""
import re
import struct
from pathlib import Path


def recipes(source):
    addresses={}
    for file in sorted(Path(source).rglob('*.gd')):
        text=file.read_text(encoding='utf-8')
        # Named literal reader recipes and direct explicit scalar reads.
        for match in re.finditer(r'_k\(\s*"([^"]+)"\s*,\s*(0x[0-9a-fA-F]+)\s*,\s*\[([^\]]*)\]',text,re.S):
            va=int(match[2],16)
            functions=re.findall(r'"([^"\n]+)"',match[3])
            for ident in re.findall(r'\b_[A-Za-z_]\w*\b',match[3]):
                binding=re.search(r'(?m)^const\s+'+re.escape(ident)+r'\s*:?=\s*"([^"\n]+)"',text)
                if binding:functions.append(binding[1])
                else:raise ValueError('Unresolved constant citation alias '+ident)
            record=addresses.setdefault(va,{'width':4,'fields':set(),'functions':set()})
            record['fields'].add(match[1]);record['functions'].update(functions)
        for match in re.finditer(r'UeRdata\.(f32|f64|i32)\(\s*(0x[0-9a-fA-F]+)\s*\)',text):
            va=int(match[2],16);record=addresses.setdefault(va,{'width':4,'fields':set(),'functions':set()})
            record['width']=max(record['width'],8 if match[1]=='f64' else 4)
            record['fields'].add(file.name+':'+match[1])
    if not addresses:raise ValueError('No original constant recipes in importer source')
    return addresses


def table(pe, native_rows, source):
    plan=recipes(source)
    rows=native_rows.decode().splitlines();covered={int(r.split('\t')[0],16) for r in rows[1:]}
    section=next(s for s in pe.sections if s[0]=='.rdata')
    for va,record in sorted(plan.items()):
        if va in covered:continue
        rva=va-pe.base
        if not section[1]<=rva or rva+8>section[1]+section[4]:raise ValueError('Constant recipe outside original file-backed .rdata')
        raw=pe.read_rva(rva,8);width=record['width']
        funcs=';'.join(sorted(record['functions'])) or 'local source constant recipe'
        values=[hex(va)[2:],hex(rva-section[1])[2:],str(width),raw[:width].hex(),str(struct.unpack('<f',raw[:4])[0]),
                str(struct.unpack('<d',raw)[0]),str(struct.unpack('<i',raw[:4])[0]),str(struct.unpack('<q',raw)[0]),funcs]
        rows.append('\t'.join(values))
    return ('\n'.join(rows)+'\n').encode(),plan
