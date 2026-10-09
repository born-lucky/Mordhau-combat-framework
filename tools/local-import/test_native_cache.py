"""Offline synthetic format fixtures; no original game data is embedded or executed."""
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('native_cache',Path(__file__).with_name('native_cache.py'))
n=importlib.util.module_from_spec(spec);spec.loader.exec_module(n)


def fixture_pe(path):
    data=bytearray(0xc00);data[:2]=b'MZ';struct.pack_into('<I',data,60,128)
    data[128:132]=b'PE\0\0';struct.pack_into('<HH',data,132,0x8664,3);struct.pack_into('<H',data,148,240)
    struct.pack_into('<H',data,152,0x20b);struct.pack_into('<Q',data,176,0x140000000)
    sections=[('.text',0x1000,896,0x400),('.rdata',n.FLOAT_RVA,16,0x800),('.names',min(n.NAME_RVAS),768,0x900)]
    for i,(name,rva,size,raw) in enumerate(sections):
        off=392+i*40;data[off:off+8]=name.encode().ljust(8,b'\0');struct.pack_into('<IIII',data,off+8,size,rva,size,raw)
    struct.pack_into('<I',data,0x800,0x41234567) # Arbitrary synthetic word, not the original constant.
    for i,rva in enumerate(n.NAME_RVAS):
        at=0x900+rva-min(n.NAME_RVAS);text=f'bone_{i}'.encode()+b'\0';data[at:at+len(text)]=text
        ins=0x1000+i*128;disp=rva-(ins+7)
        data[0x400+i*128:0x408+i*128]=b'\x48\x8d\x15'+struct.pack('<i',disp)+b'\xc3'
    path.write_bytes(data)


class FakeMsf:
    def __init__(self):
        self.sizes=[0]*8;self.data=bytearray()
        for i in range(7):
            symbol=f'??__ENAME_Bone_{i}@@YAXXZ'.encode()+b'\0'
            payload=struct.pack('<IIH',2,i*128,1)+symbol
            self.data+=struct.pack('<HH',len(payload)+2,0x110e)+payload
    def read(self,index):
        if index==3:
            dbi=bytearray(64);struct.pack_into('<i',dbi,0,-1);struct.pack_into('<H',dbi,20,7);return bytes(dbi)
        return bytes(self.data)


class NativeCacheTests(unittest.TestCase):
    def test_structured_symbol_instruction_binding_and_raw_pe_cache(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp)/'fixture.exe';fixture_pe(p);pe=n.PE(p)
            names=n.bind_names(pe,n.public_initializers(FakeMsf(),pe))
            self.assertEqual(len(names),7)
            self.assertEqual(names[n.NAME_RVAS[0]][0],'Bone_0') # Case comes from the actual symbol, not title-casing bytes.
            rows=n.constant_tsv(pe,names).decode().splitlines()
            self.assertEqual(len(rows),9)
            self.assertEqual(next(r for r in rows if r.startswith(hex(pe.base+n.FLOAT_RVA)[2:]+'\t')).split('\t')[3],'67452341')

    def test_equivalent_initializer_copies_are_preserved_and_name_conflicts_reject(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'fixture.exe';fixture_pe(p);pe=n.PE(p)
            bindings=n.public_initializers(FakeMsf(),pe)
            # Two original code ranges reference the exact same literal and field identity.
            data=bytearray(p.read_bytes())
            data[0x440:0x448]=b'\x48\x8d\x15'+struct.pack('<i',n.NAME_RVAS[0]-(0x1040+7))+b'\xc3'
            p.write_bytes(data)
            result=n.bind_names(pe,bindings+[('NAME_Bone_0',0x1040)])
            self.assertEqual(result[n.NAME_RVAS[0]][1],(0x1000,0x1040))
            # Case alone is equivalent for literal matching, but conflicting field spelling is rejected.
            with self.assertRaises(n.ImportBlocked):n.bind_names(pe,bindings+[('NAME_bONE_0',0x1000)])

    def test_name_without_its_original_reference_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp)/'fixture.exe';fixture_pe(p)
            with p.open('r+b') as f:f.seek(0x400);f.write(b'\xc3')
            with self.assertRaises(n.ImportBlocked):n.bind_names(n.PE(p),n.public_initializers(FakeMsf(),n.PE(p)))

    def test_bad_pe_ranges_and_symbol_records_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp)/'bad.exe';p.write_bytes(b'MZ')
            with self.assertRaises(n.ImportBlocked):n.PE(p)
            fixture_pe(p);msf=FakeMsf();msf.data+=b'\xff'
            with self.assertRaises(n.ImportBlocked):n.public_initializers(msf,n.PE(p))
            with self.assertRaises(n.ImportBlocked):n.PE(p).read_rva(0,8)

    def test_existing_divergent_output_is_preserved_before_any_commit(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp);(p/'old').write_bytes(b'prior evidence')
            with self.assertRaises(n.ImportBlocked):n.commit_generated(p,{'new':b'new','old':b'different'})
            self.assertEqual((p/'old').read_bytes(),b'prior evidence');self.assertFalse((p/'new').exists())
            files=n.commit_generated(p,{'old':b'prior evidence','new':b'new'})
            self.assertEqual(len(files),2)

    def test_install_is_verified_before_cache_creation(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp);cache=p/'newcache'
            with self.assertRaises((OSError,n.ImportBlocked)):n.generate(p,cache)
            self.assertFalse(cache.exists())

    def test_cache_inside_installation_is_rejected_before_mkdir(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Path(temp);cache=p/'not-created'
            with patch.object(n,'verify_install',return_value=(p,None,None,None)):
                with self.assertRaises(n.ImportBlocked):n.generate(p,cache)
            self.assertFalse(cache.exists())


if __name__=='__main__':unittest.main(verbosity=2)
