"""Synthetic source-only decompile/header fixtures, no original default records."""
import importlib.util,tempfile,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('legacy_replay',Path(__file__).with_name('legacy_replay.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class Tests(unittest.TestCase):
 def test_shift_rhs_recursively_resolves_literal_symbol(self):
  oracle=m.Oracle.__new__(m.Oracle);oracle.rows={0x140002000:{'raw':'78563412'}}
  self.assertEqual(oracle.rhs('(ulonglong)_DAT_140002000 << 0x20',{}),0x1234567800000000)
  self.assertEqual(oracle.rhs('(ulonglong)uVar7 << 0x20',{'uVar7':0x1122334455667788}),0x5566778800000000)
  self.assertIsNone(oracle.rhs('(ulonglong)_DAT_140009000 << 0x20',{}))
 def test_source_reader_literal_nested_and_rdata_contract(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp)/'native';(root/'types').mkdir(parents=True);(root/'decomp_r1').mkdir()
   (root/'rdata.tsv').write_text('va\traw\n140002000\t0000a03f\n')
   (root/'types/FSynthetic.h').write_text('struct FSynthetic\n{\n\t/* 0x0000 */ float Field;\n\t/* 0x0004 */ FInner Nested;\n}; // sizeof = 0xc\n')
   (root/'types/FInner.h').write_text('struct FInner\n{\n\t/* 0x0000 */ int32 Count;\n\t/* 0x0004 */ bool Enabled;\n}; // sizeof = 0x8\n')
   (root/'decomp_r1/FSynthetic.cpp').write_text('// FSynthetic::FSynthetic  rva=0x1000\nvoid f(undefined1 *param_1) {\n*(undefined4 *)(param_1 + 0) = _DAT_140002000;\nFInner__FInner(param_1 + 4);\n}\n')
   (root/'decomp_r1/FInner.cpp').write_text('// FInner::FInner  rva=0x1010\nvoid f(undefined1 *param_1) {\n*(undefined4 *)(param_1 + 0) = 9;\n*(undefined1 *)(param_1 + 4) = 1;\n}\n')
   self.assertEqual(m.Oracle(Path(tmp)).defaults('FSynthetic'),dict(Field=1.25,Nested=dict(Count=9,Enabled=True)))
if __name__=='__main__':unittest.main()
