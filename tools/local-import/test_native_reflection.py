"""Synthetic original-symbol/typed-layout contracts, no original enum values."""
import importlib.util,struct,unittest
from pathlib import Path
sp=importlib.util.spec_from_file_location('native_reflection',Path(__file__).with_name('native_reflection.py'))
m=importlib.util.module_from_spec(sp);sp.loader.exec_module(m)
class PE:
 base=0x140000000;sections=[('.rdata',0x1000,4096,0,4096)]
 def __init__(self):self.data=bytearray(4096)
 def read_rva(self,rva,n):
  if not 0x1000<=rva<=rva+n<=0x2000:raise ValueError('File-backed range')
  return bytes(self.data[rva-0x1000:rva-0x1000+n])
def symbol(name,off):
 body=struct.pack('<HIIH',0x110d,0x1001,off,1)+name.encode()+b'\0'
 return struct.pack('<H',len(body))+body
class Tests(unittest.TestCase):
 entry_layout={'size':16,'members':[{'name':'NameUTF8','off':0},{'name':'Value','off':8}]}
 def fixture(self):
  pe=PE();struct.pack_into('<Qi',pe.data,8,pe.base+0x1100,2)
  struct.pack_into('<QqQq',pe.data,0x100,pe.base+0x1200,7,pe.base+0x1300,-3)
  pe.data[0x200:0x200+len(b'ESynthetic::One\0')]=b'ESynthetic::One\0'
  pe.data[0x300:0x300+len(b'ESynthetic::Two\0')]=b'ESynthetic::Two\0'
  records=m.data_records(symbol('Z_Synthetic::EnumParams',0)+symbol('Z_Synthetic::Enumerators',0x100),pe)
  layout={'size':32,'members':[{'name':'EnumeratorParams','off':8},{'name':'NumEnumerators','off':16}]}
  return pe,records,layout
 def test_exact_data_symbol_and_original_typed_offsets(self):
  pe,records,layout=self.fixture();result=m.read_enum(pe,records,'Z_Synthetic',layout,self.entry_layout)
  self.assertEqual(result['rows'],[{'name':'ESynthetic::One','value':7},{'name':'ESynthetic::Two','value':-3}])
  self.assertEqual(records[0]['rva'],0x1000);self.assertEqual(records[0]['type_index'],0x1001)
 def test_wrong_pointer_ambiguous_symbols_and_missing_layout_rejected(self):
  pe,records,layout=self.fixture();struct.pack_into('<Q',pe.data,8,pe.base+0x1108)
  with self.assertRaises(ValueError):m.read_enum(pe,records,'Z_Synthetic',layout,self.entry_layout)
  pe,records,layout=self.fixture();records.append(dict(records[0],rva=0x1080))
  with self.assertRaises(ValueError):m.read_enum(pe,records,'Z_Synthetic',layout,self.entry_layout)
  with self.assertRaises(ValueError):m.read_enum(pe,records[:2],'Z_Synthetic',{'size':32,'members':[]},self.entry_layout)
 def test_symbol_bounds_and_name_terminator_rejected(self):
  pe,_,_=self.fixture();raw=symbol('X',0)
  with self.assertRaises(ValueError):m.data_records(raw[:-1],pe)
  raw=bytearray(raw);raw[-1]=ord('x')
  with self.assertRaises(ValueError):m.data_records(raw,pe)
if __name__=='__main__':unittest.main()
