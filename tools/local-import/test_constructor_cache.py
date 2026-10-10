"""Synthetic original-code contracts; no generated game values are fixtures."""
import importlib.util,struct,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('constructor_cache_test_module',Path(__file__).with_name('constructor_cache.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class PE:
    base=0x140000000
    def __init__(self,name=b'memset',zero=True):
        start=0x1b886da;target=0x5000;slot=0x6000;name_rva=0x7000
        first=bytes.fromhex('4c63459731d24889f9') if zero else bytes.fromhex('4c63459731c94889f9')
        call=first+b'\xe8'+struct.pack('<i',target-(start+len(first)+5))
        self.blocks={start:call,target:b'\xff\x25'+struct.pack('<i',slot-target-6),slot:struct.pack('<Q',name_rva),
                     name_rva+2:(name+b'\0').ljust(64,b'\0'),
                     0x1b8846b:bytes.fromhex('458b7758448975974489f2')+b'\x90'*40}
    def read_rva(self,rva,size):
        for start,data in self.blocks.items():
            if start<=rva and rva+size<=start+len(data):return data[rva-start:rva-start+size]
        raise ValueError('Fixture read outside explicit bytes')

class Tests(unittest.TestCase):
    def test_original_zero_fill_must_have_zero_argument_and_memset_import(self):
        proof=m.zero_allocation_evidence(PE())
        self.assertEqual(proof['memset_import'],'memset');self.assertEqual(proof['memset_zero_argument'],'EDX=0')
        with self.assertRaises(ValueError):m.zero_allocation_evidence(PE(name=b'other'))
        with self.assertRaises(ValueError):m.zero_allocation_evidence(PE(zero=False))
    def test_schema_reads_authored_property_names_and_no_default_values(self):
        fields=m.weapon_consumer_fields()
        self.assertIn('RightHandEquipOffset',fields['weapon']);self.assertIn('StrikeAttack',fields['weapon'])
        self.assertIn('Damage',fields['attack']);self.assertIn('AngleAdditive',fields['motion'])
    def test_enum_bytes_use_original_names_and_unknown_numeric_value_is_omitted(self):
        class Layouts:
            def layout(self,name):return {'enumerators':[('First',0),('Second',1)]}
        values={'Restriction':1,'Other':5}
        review=[{'field':'Restriction','type':'EFixture','accepted':True},{'field':'Other','type':'EFixture','accepted':True}]
        m.enum_strings(Layouts(),values,review)
        self.assertEqual(values,{'Restriction':'EFixture::Second'});self.assertFalse(review[1]['accepted'])

if __name__=='__main__':unittest.main()
