import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest

sp=importlib.util.spec_from_file_location('ghidra_inputs',Path(__file__).with_name('ghidra_inputs.py'))
g=importlib.util.module_from_spec(sp);sp.loader.exec_module(g)


class PE:
    base=0x140000000
    sections=[('.text',0x1000,0x1000,0,0x1000)]
    def __init__(self):
        # First original-like ctor calls a nested ctor, then an unknown function.
        self.raw={0x1010:b'\xe8'+struct.pack('<i',0x1040-0x1015)+b'\xe8'+struct.pack('<i',0x1080-0x101a)+b'\xc3',
                  0x1040:b'\xc3'}
    def read_rva(self,rva,size):return self.raw[rva][:size]


def row(name,rva,size):return {'name':name,'rva':rva,'size':size,'object':'synthetic-unit.obj'}


class Tests(unittest.TestCase):
    def inventory(self):
        rows=[row('UOuter::UOuter',0x1010,11),row('FInner::FInner',0x1040,1),row('FFolded::FFolded',0x1040,1)]
        return {'constructors':rows,'procedures':rows+[row('UnresolvedFunction',0x1080,1),row('UnrelatedProcedure',0x1090,1)]}
    def test_original_offsets_nested_closure_aliases_and_unknown_calls(self):
        labels,functions,rows,calls=g.constructor_rows(PE(),self.inventory(),['UOuter'])
        self.assertEqual([r['name'] for r in rows],['UOuter::UOuter','FFolded::FFolded','FInner::FInner'])
        self.assertIn('16\tUOuter::UOuter\n',labels)
        self.assertNotIn('UnrelatedProcedure',labels)
        self.assertIn('FInner::FInner\t.text\t64\t1\tsynthetic-unit.obj\n',functions)
        self.assertEqual([(r['target_rva'],r['known_game_constructor']) for r in calls],[(0x1040,True),(0x1080,False)])
    def test_ambiguous_overload_and_folded_sizes_rejected(self):
        inv=self.inventory();inv['constructors'].append(row('UOuter::UOuter',0x1020,1))
        with self.assertRaisesRegex(ValueError,'overload'):g.constructor_rows(PE(),inv,['UOuter'])
        inv=self.inventory();inv['constructors'][2]['size']=2
        with self.assertRaisesRegex(ValueError,'size ambiguity'):g.constructor_rows(PE(),inv,['FInner'])
    def test_missing_class_and_nontext_ranges_rejected(self):
        with self.assertRaisesRegex(ValueError,'missing'):g.constructor_rows(PE(),self.inventory(),['UNothing'])
        pe=PE();pe.sections=[('.nottext',0x1000,0x1000,0,0x1000)]
        with self.assertRaisesRegex(ValueError,'.text'):g.constructor_rows(pe,self.inventory(),['UOuter'])
    def test_incomplete_instruction_scan_is_not_a_complete_call_closure(self):
        with self.assertRaisesRegex(ValueError,'coverage incomplete'):
            g.constructor_rows(PE(),self.inventory(),['UOuter'],decoder=lambda raw,va:[])
    def test_header_bases_and_nested_structs_preserve_complete_layouts(self):
        class Types:
            def decl(self,ti):return {1:'FNested',2:'FOpaque*'}[ti]
        class Layouts:
            types=Types();byname={'UChild':1,'UBase':2,'FNested':3,'FOpaque':4}
            def layout(self,name):return {'name':name,'bases':[{'name':'UBase'}] if name=='UChild' else [],
                                          'members':[{'ti':1},{'ti':2}] if name=='UChild' else []}
        self.assertEqual(set(g.header_closure(Layouts(),['UChild'])),{'UChild','UBase','FNested'})
    def test_explicit_heap_no_autoanalysis_no_library_load_and_version_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tool=root/'tool';stage=root/'stage';stage.mkdir();(tool/'Ghidra').mkdir(parents=True)
            (tool/'support').mkdir();launcher=tool/'support/launch.bat';launcher.write_text('Not executed')
            props=tool/'Ghidra/application.properties';props.write_text('application.version=11.3.2\n')
            exe=root/'original.exe';exe.write_bytes(b'Not executed');scripts=root/'scripts';scripts.mkdir()
            command=g.headless_command(tool,stage,exe,scripts)
            self.assertEqual(command[:5],[str(launcher.resolve()),'fg','jdk','Ghidra-Headless','1G'])
            self.assertIn('-noanalysis',command);self.assertEqual(command[command.index('-max-cpu')+1],'1')
            self.assertEqual(command[command.index('-loader-loadLibraries')+1],'false')
            self.assertNotIn('ApplyTypes.java',command);self.assertEqual(launcher.read_text(),'Not executed')
            props.write_text('application.version=unreviewed\n')
            with self.assertRaisesRegex(ValueError,'version'):g.headless_command(tool,stage,exe,scripts)


if __name__=='__main__':unittest.main()
