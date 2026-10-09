import importlib.util
from pathlib import Path
import struct
import unittest

sp=importlib.util.spec_from_file_location('ghidra_constants',Path(__file__).with_name('ghidra_constants.py'))
g=importlib.util.module_from_spec(sp);sp.loader.exec_module(g)


class PE:
    base=0x140000000
    sections=[('.rdata',0x4000,16,0,16)]
    def read_rva(self,rva,size):
        if not 0x4000<=rva<rva+size<=0x4010:raise ValueError('bounds')
        return bytes(range(16))[rva-0x4000:rva-0x4000+size]


class Tests(unittest.TestCase):
    def test_exact_little_endian_raw_words_and_duplicate_reference_identity(self):
        raw,accepted,skipped=g.reconstruct(PE(),[('A.cpp','_DAT_140004000'),('B.cpp','_DAT_140004000')])
        fields=raw.decode().splitlines()[1].split('\t')
        self.assertEqual(fields[3],'0001020304050607');self.assertEqual(int(fields[7]),0x0706050403020100)
        self.assertEqual(float(fields[4]),struct.unpack('<f',bytes(range(4)))[0])
        self.assertEqual(accepted[0]['files'],['A.cpp','B.cpp']);self.assertEqual(skipped,[])
    def test_writable_or_partial_tail_reference_is_reported_not_zero_filled(self):
        raw,accepted,skipped=g.reconstruct(PE(),[('A.cpp','_DAT_140004000 + _DAT_140008000 + _DAT_140004009')])
        self.assertEqual(len(accepted),1);self.assertEqual(len(skipped),2)
        self.assertEqual(len(raw.decode().splitlines()),2)
    def test_no_reference_is_not_successful_dependency_generation(self):
        with self.assertRaisesRegex(ValueError,'No raw'):g.reconstruct(PE(),[('A.cpp','no original refs')])


if __name__=='__main__':unittest.main()
