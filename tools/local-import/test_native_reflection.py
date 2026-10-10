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
def symbol(name,off,kind=0x110d):
 body=struct.pack('<HIIH',kind,0x1001,off,1)+name.encode()+b'\0'
 return struct.pack('<H',len(body))+body
def scoped_symbols(name,rows,start=0):
 body=struct.pack('<H8IHB',0x1110,0,0,0,1,0,0,0x1000,0,1,0)+name.encode()+b'\0'
 proc=bytearray(struct.pack('<H',len(body))+body)
 end=start+len(proc)+len(rows);struct.pack_into('<I',proc,8,end)
 return bytes(proc)+rows+struct.pack('<HH',2,6)
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
 def test_unmapped_unrelated_symbols_are_retained_but_selected_enum_is_rejected(self):
  pe,records,layout=self.fixture()
  raw=bytearray(symbol('Unrelated::Data',0));struct.pack_into('<H',raw,12,10)
  unmapped=m.data_records(raw,pe)
  self.assertIsNone(unmapped[0]['rva']);self.assertEqual(unmapped[0]['section'],10)
  self.assertEqual(len(m.read_enum(pe,records+unmapped,'Z_Synthetic',layout,self.entry_layout)['rows']),2)
  for section in (0,10):
   raw=bytearray(symbol('Z_Synthetic::EnumParams',0));struct.pack_into('<H',raw,12,section)
   with self.assertRaises(ValueError):m.read_enum(pe,records+m.data_records(raw,pe),'Z_Synthetic',layout,self.entry_layout)
 def test_procedure_scope_qualifies_local_static_names_without_confusing_other_enums(self):
  pe,_,layout=self.fixture()
  raw=scoped_symbols('Z_Synthetic',symbol('Enumerators',0x100,0x110c)+symbol('EnumParams',0,0x110c))
  raw+=scoped_symbols('Z_Other',symbol('Enumerators',0x180,0x110c)+symbol('EnumParams',0x80,0x110c),len(raw))
  records=m.data_records(raw,pe)
  self.assertEqual(records[0]['name'],'Z_Synthetic::Enumerators')
  self.assertEqual(records[0]['raw_name'],'Enumerators')
  self.assertEqual(len(m.read_enum(pe,records,'Z_Synthetic',layout,self.entry_layout)['rows']),2)
  raw=bytearray(raw);struct.pack_into('<I',raw,8,len(raw)+100)
  with self.assertRaises(ValueError):m.data_records(raw,pe)
 def test_pdb_enum_crosscheck_rejects_wrong_values_and_same_short_names_in_wrong_type(self):
  rows=[{'name':'ESynthetic::One','value':7},{'name':'ESynthetic::Two','value':-3}]
  layout={'kind':'enum','name':'ESynthetic','enumerators':[('One',7),('Two',-3)]}
  m.check_enum_type(rows,layout)
  for bad in ([dict(rows[0],value=8),rows[1]],[dict(rows[0],name='EOther::One'),rows[1]],rows+rows[:1]):
   with self.assertRaises(ValueError):m.check_enum_type(bad,layout)
if __name__=='__main__':unittest.main()
