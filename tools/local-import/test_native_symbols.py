import importlib.util,struct,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('symbols',Path(__file__).with_name('native_symbols.py'))
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
class Fake:
 def __init__(self,streams):self.streams=streams;self.sizes=[len(x) for x in streams]
 def read(self,i):return self.streams[i]
class PE:
 sections=[('.text',0x1000,4096,0,4096)]
 def read_rva(self,rva,size):
  if not 0x1000<=rva<=rva+size<=0x2000:raise ValueError('range')
  return b'\0'*size
class Tests(unittest.TestCase):
 def fixture(self):
  m=bytearray(64);struct.pack_into('<H',m,34,4)
  obj=b'D:\\Build\\Shipping\\Mordhau\\Module.Mordhau.1_of_9.cpp.obj'
  m+=b'unit\0'+obj+b'\0';m+=bytes((-len(m))%4)
  p=bytearray(39);struct.pack_into('<H',p,2,0x1110);struct.pack_into('<I',p,16,16)
  struct.pack_into('<I',p,32,32);struct.pack_into('<H',p,36,1);p+=b'UFixture::UFixture\0'
  p+=bytes((-len(p))%4);struct.pack_into('<H',p,0,len(p)-2)
  stream=struct.pack('<I',4)+p;struct.pack_into('<I',m,36,len(stream))
  dbi=bytearray(64);struct.pack_into('<i',dbi,0,-1);struct.pack_into('<i',dbi,24,len(m));dbi+=m
  return Fake([b'',b'',b'',dbi,stream])
 def test_original_address_recipe_and_constructor_identity(self):
  result=s.inventory(self.fixture(),PE());self.assertEqual(len(result['constructors']),1)
  self.assertEqual(result['constructors'][0]['rva'],0x1020);self.assertEqual(result['constructors'][0]['size'],16)
 def test_symbol_bounds_reject(self):
  m=self.fixture();m.streams[4]=m.streams[4][:-1]
  with self.assertRaises(ValueError):s.inventory(m,PE())
if __name__=='__main__':unittest.main()
