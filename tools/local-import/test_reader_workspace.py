import importlib.util,tempfile,unittest
from pathlib import Path
sp=importlib.util.spec_from_file_location('reader_workspace',Path(__file__).with_name('reader_workspace.py'))
m=importlib.util.module_from_spec(sp);sp.loader.exec_module(m)
class Native:
 EXE_SHA1='synthetic-exe';PDB_SHA1='synthetic-pdb'
 class PE:
  base=0x140000000;sections=[('.rdata',0x3f00000,0x3000000,0,0x3000000)]
  def read_rva(self,rva,n):return bytes(n)
 def verify_install(self,p):return Path(p),None,None,self.PE()
class Tests(unittest.TestCase):
 def test_rejected_install_and_output_never_create_cache(self):
  with tempfile.TemporaryDirectory() as tmp:
   cache=Path(tmp)/'cache'
   class Rejected(Native):
    def verify_install(self,p):raise ValueError('Rejected install')
   with self.assertRaises(ValueError):m.prepare(Path(tmp)/'game',cache,Rejected())
   self.assertFalse(cache.exists())
   game=Path(tmp)/'game'
   with self.assertRaises(ValueError):m.prepare(game,game/'generated',Native())
   self.assertFalse(game.exists())
 def test_unique_source_only_project_and_unreviewed_writer_rejected(self):
  with tempfile.TemporaryDirectory() as tmp:
   game=Path(tmp)/'game';cache=Path(tmp)/'cache';one=m.prepare(game,cache,Native());two=m.prepare(game,cache,Native())
   self.assertNotEqual(one['project'],two['project']);self.assertFalse(one['runtime_ready'])
   self.assertEqual(len(one['source_files']),94)
   self.assertIn('data_backend="pak"',(Path(one['project'])/'project.godot').read_text())
   self.assertGreater(one['source_constant_count'],0)
   self.assertTrue((Path(one['project']).parent/'extract/native/rdata.tsv').is_file())
   tool=Path(tmp)/'synthetic.exe';tool.write_text('Not executed')
   with self.assertRaises(ValueError):m.command(tool,one,'weapons')
   args=m.command(tool,one,'settings');self.assertEqual(args[-1],'--only=settings')
   self.assertFalse(game.exists())
if __name__=='__main__':unittest.main()
