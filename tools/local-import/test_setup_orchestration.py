import contextlib
import importlib.util
import io
import json
from pathlib import Path
import types
import unittest
from unittest.mock import patch

sp=importlib.util.spec_from_file_location('local_setup',Path(__file__).parents[1]/'setup.py')
m=importlib.util.module_from_spec(sp);sp.loader.exec_module(m)


class Tests(unittest.TestCase):
    def run_setup(self,failure=None,check=False,pak=True):
        calls=[]
        def stage(name):
            def generate(*args):
                calls.append(name)
                if failure==name:raise ValueError('Synthetic '+name+' stage rejection')
                return {'stage':name,'complete':True,'runtime_ready':False}
            return types.SimpleNamespace(generate=generate)
        def verify(game):
            calls.append('verify')
            if failure=='verify':raise ValueError('Synthetic invalid original')
            return None,None,None,None
        native=stage('native');native.verify_install=verify;native.EXE_SHA1='synthetic';native.PDB_SHA1='synthetic'
        modules={'native_cache':native,'shadow_cache':stage('shadow'),'mode_cache':stage('mode')}
        args=['--game-dir','synthetic-install','--cache-dir','synthetic-cache']
        if pak:args.extend(['--pak-tool','synthetic-mh-pak.exe'])
        if check:args.append('--check')
        output=io.StringIO()
        with patch.object(m,'load_stage',side_effect=lambda name:modules[name]),contextlib.redirect_stdout(output):
            exit_code=m.main(args)
        return exit_code,json.loads(output.getvalue()),calls
    def test_verified_order_and_three_stages_never_claim_full_readiness(self):
        exit_code,result,calls=self.run_setup()
        self.assertEqual(calls,['verify','native','shadow','mode']);self.assertEqual(exit_code,2)
        self.assertEqual(result['mode_stage']['stage'],'mode');self.assertFalse(result['runtime_ready'])
        self.assertIn('full generated spec matrix and reviewed constructor fields',result['remaining'])
    def test_stage_rejection_preserves_completed_receipts_and_stops_later_stages(self):
        for failed,expected in [('verify',['verify']),('native',['verify','native']),('shadow',['verify','native','shadow']),('mode',['verify','native','shadow','mode'])]:
            exit_code,result,calls=self.run_setup(failed)
            self.assertEqual(exit_code,2);self.assertEqual(calls,expected);self.assertFalse(result['runtime_ready'])
            self.assertEqual(result['native_cache_generated'],failed not in ('verify','native'))
            if failed=='mode':self.assertEqual(result['shadow_stage']['stage'],'shadow')
            self.assertIsNone(result['mode_stage'])
    def test_check_never_generates_data(self):
        exit_code,result,calls=self.run_setup(check=True)
        self.assertEqual(exit_code,0);self.assertEqual(calls,['verify']);self.assertFalse(result['runtime_ready'])


if __name__=='__main__':unittest.main()
