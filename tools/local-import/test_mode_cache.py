import importlib.util
from pathlib import Path
import tempfile
import unittest
import json
import os
import sys
import subprocess

sp=importlib.util.spec_from_file_location('mode_cache',Path(__file__).with_name('mode_cache.py'))
m=importlib.util.module_from_spec(sp);sp.loader.exec_module(m)


class Tests(unittest.TestCase):
    def fixture(self):return {'Fixture':'Mordhau/Content/Fixtures/BP_Fixture'},{'key':('Fixture','Function')}
    def test_recipe_identity_reads_own_source_without_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'source.py'
            p.write_text("GM='Mordhau/Content/Fixtures/'\nPKGS={'Fixture':GM+'BP_Fixture'}\nSPEC={'key':('Fixture','Function',r'pattern',int,0)}\nraise RuntimeError('MUST NOT EXECUTE')\n")
            self.assertEqual(m.recipes(p),self.fixture())
    def test_stored_value_and_exact_statement_citation_validation(self):
        packages,bindings=self.fixture();table={'key':{'value':[1.25,-3.0],'src':'BP_Fixture:Function@12'}}
        self.assertEqual(m.validate(table,packages,bindings),table)
        for bad in [{'key':{'value':float('nan'),'src':'BP_Fixture:Function@12'}},
                    {'key':{'value':1,'src':'BP_Other:Function@12'}},
                    {'key':{'value':1,'src':'BP_Fixture:Function@-1'}},
                    {'unknown':{'value':1,'src':'BP_Fixture:Function@12'}}]:
            with self.assertRaises(ValueError):m.validate(bad,packages,bindings)
    def test_packaged_original_recipes_are_named_not_cached_values(self):
        packages,bindings=m.recipes(m.source_script())
        self.assertEqual(len(packages),29);self.assertGreater(len(bindings),100)
        self.assertEqual(packages['StatusBar'],'Mordhau/Content/Mordhau/UI/BP_StatusBar')
    def test_invalid_install_rejected_before_cache_creation_or_tool_launch(self):
        class Native:
            def verify_install(self,path):raise ValueError('Rejected original install')
        with tempfile.TemporaryDirectory() as tmp:
            cache=Path(tmp)/'cache'
            with self.assertRaisesRegex(ValueError,'Rejected original'):
                m.generate(Path(tmp)/'game',cache,Path(tmp)/'mh-pak.exe',Native())
            self.assertFalse(cache.exists())
    def test_failed_own_tool_log_preserves_status_before_rejecting(self):
        with tempfile.TemporaryDirectory() as tmp:
            stage=Path(tmp)
            with self.assertRaises(Exception):
                m.run_logged([sys.executable,'-c',"import sys;print('synthetic failure',file=sys.stderr);sys.exit(7)"],dict(os.environ),stage,1,10)
            record=json.loads((stage/'process-001.json').read_text())
            self.assertEqual(record['exit_code'],7);self.assertIn('synthetic failure',record['stderr'])
    def test_missing_raw_consumer_generator_refuses_mdx_and_asset_exports(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);output=root/'mode.json';raw=root/'raw';raw.mkdir()
            env=dict(os.environ,MORDHAU_MODE_RAW=str(raw),MORDHAU_MODE_OUT=str(output),
                     MORDHAU_MODE_REQUIRE_RAW='1',MORDHAU_MODE_CONSTANTS_ONLY='1')
            result=subprocess.run([sys.executable,str(m.source_script())],env=env,cwd=root,capture_output=True,text=True,timeout=10)
            self.assertNotEqual(result.returncode,0);self.assertIn('Required locally read original package bytes missing',result.stderr)
            self.assertFalse(output.exists());self.assertEqual(list(raw.iterdir()),[])


if __name__=='__main__':unittest.main()
