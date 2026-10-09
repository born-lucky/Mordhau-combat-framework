import importlib.util,struct,tempfile,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('source_constants',Path(__file__).with_name('source_constants.py'))
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
class PE:
 base=0x140000000;sections=[('.rdata',0x2000,64,0,64)]
 def read_rva(self,rva,size):
  if rva!=0x2010:raise ValueError('unmapped')
  return struct.pack('<ff',1.25,-2.5)[:size]
class Tests(unittest.TestCase):
 def test_named_and_direct_recipes_get_original_bytes(self):
  with tempfile.TemporaryDirectory() as temp:
   root=Path(temp);(root/'helper.gd').write_text('const _CALL := "USynthetic::Read"\nvar f = _k("scalar",0x140002010,[_CALL])\nvar d = UeRdata.f64(0x140002010)\n')
   table,plan=s.table(PE(),b'va\trdata_off\twidth\traw\tf32\tf64\ti32\ti64\tfuncs\n',root)
   row=table.decode().splitlines()[1].split('\t');self.assertEqual(row[3],struct.pack('<ff',1.25,-2.5).hex());self.assertEqual(row[2],'8');self.assertEqual(row[8],'USynthetic::Read')
   self.assertEqual(plan[0x140002010]['fields'],{'scalar','helper.gd:f64'})
 def test_unbound_alias_and_outside_rdata_reject(self):
  with tempfile.TemporaryDirectory() as temp:
   root=Path(temp);p=root/'helper.gd';p.write_text('var f=_k("test",0x140002010,[_MISSING])')
   with self.assertRaises(ValueError):s.recipes(root)
   p.write_text('var f=_k("test",0x140003010,["Call"])')
   with self.assertRaises(ValueError):s.table(PE(),b'header\n',root)
if __name__=='__main__':unittest.main()
