import contextlib
import importlib.util
import io
import json
from pathlib import Path
import types
import tempfile
import unittest
from unittest.mock import patch

sp=importlib.util.spec_from_file_location('local_setup',Path(__file__).parents[1]/'setup.py')
m=importlib.util.module_from_spec(sp);sp.loader.exec_module(m)


class Tests(unittest.TestCase):
    def run_setup(self,failure=None,check=False,pak=True,weapons=False,weapons_only=False,incomplete_weapon=False):
        calls=[]
        def stage(name):
            def generate(*args):
                calls.append(name)
                if failure==name:raise ValueError('Synthetic '+name+' stage rejection')
                return {'stage':name,'complete':not (name=='weapons' and incomplete_weapon),'runtime_ready':False}
            return types.SimpleNamespace(generate=generate)
        def verify(game):
            calls.append('verify')
            if failure=='verify':raise ValueError('Synthetic invalid original')
            return None,None,None,None
        native=stage('native');native.verify_install=verify;native.EXE_SHA1='synthetic';native.PDB_SHA1='synthetic'
        modules={'native_cache':native,'enum_cache':stage('enums'),'shadow_cache':stage('shadow'),'mode_cache':stage('mode'),'weapon_cache':stage('weapons')}
        args=['--game-dir','synthetic-install','--cache-dir','synthetic-cache']
        if pak:args.extend(['--pak-tool','synthetic-mh-pak.exe'])
        if weapons:args.extend(['--weapon-tool','synthetic-mh-weapon-packages.exe','--weapon-verify-tool','synthetic-mh-verify-weapon-import.exe'])
        if check:args.append('--check')
        if weapons_only:args.append('--weapons-only')
        output=io.StringIO()
        with patch.object(m,'load_stage',side_effect=lambda name:modules[name]),contextlib.redirect_stdout(output):
            exit_code=m.main(args)
        return exit_code,json.loads(output.getvalue()),calls
    def test_verified_order_and_four_stages_never_claim_full_readiness(self):
        exit_code,result,calls=self.run_setup()
        self.assertEqual(calls,['verify','native','enums','shadow','mode']);self.assertEqual(exit_code,2)
        self.assertEqual(result['mode_stage']['stage'],'mode');self.assertFalse(result['runtime_ready'])
        self.assertIn('full generated spec matrix and reviewed constructor fields',result['remaining'])
    def test_stage_rejection_preserves_completed_receipts_and_stops_later_stages(self):
        for failed,expected in [('verify',['verify']),('native',['verify','native']),('enums',['verify','native','enums']),('shadow',['verify','native','enums','shadow']),('mode',['verify','native','enums','shadow','mode'])]:
            exit_code,result,calls=self.run_setup(failed)
            self.assertEqual(exit_code,2);self.assertEqual(calls,expected);self.assertFalse(result['runtime_ready'])
            self.assertEqual(result['native_cache_generated'],failed not in ('verify','native'))
            if failed=='mode':self.assertEqual(result['shadow_stage']['stage'],'shadow')
            self.assertIsNone(result['mode_stage'])
    def test_check_never_generates_data(self):
        exit_code,result,calls=self.run_setup(check=True)
        self.assertEqual(exit_code,0);self.assertEqual(calls,['verify']);self.assertFalse(result['runtime_ready'])

    def test_full_import_includes_verified_weapon_stage_without_claiming_runtime_ready(self):
        code,result,calls=self.run_setup(weapons=True)
        self.assertEqual(code,2)
        self.assertEqual(calls,['verify','native','enums','shadow','mode','weapons'])
        self.assertEqual(result['weapon_stage']['stage'],'weapons')
        self.assertFalse(result['runtime_ready'])
        self.assertFalse(any('verified weapon data:' in r for r in result['remaining']))

    def test_focused_weapon_import_runs_only_original_verification_and_weapon_stage(self):
        code,result,calls=self.run_setup(weapons=True,weapons_only=True)
        self.assertEqual(code,0)
        self.assertEqual(calls,['verify','weapons'])
        self.assertTrue(result['weapon_cache_generated'])
        self.assertFalse(result['native_cache_generated'])
        self.assertFalse(result['runtime_ready'])
        self.assertIsNone(result['native_stage'])
        self.assertIsNone(result['enum_stage'])
        self.assertIsNone(result['shadow_stage'])
        self.assertIsNone(result['mode_stage'])

    def test_check_with_weapon_tools_still_performs_no_generation(self):
        code,result,calls=self.run_setup(check=True,weapons=True)
        self.assertEqual(code,0)
        self.assertEqual(calls,['verify'])
        self.assertFalse(result['native_cache_generated'])
        self.assertFalse(result['runtime_ready'])

    def test_weapon_stage_failure_preserves_prior_completed_stages(self):
        code,result,calls=self.run_setup(failure='weapons',weapons=True)
        self.assertEqual(code,2)
        self.assertEqual(calls,['verify','native','enums','shadow','mode','weapons'])
        self.assertEqual(result['mode_stage']['stage'],'mode')
        self.assertIsNone(result['weapon_stage'])
        self.assertFalse(result['runtime_ready'])

    def test_incomplete_focused_weapon_stage_is_not_success(self):
        code,result,calls=self.run_setup(weapons=True,weapons_only=True,incomplete_weapon=True)
        self.assertEqual(code,2)
        self.assertEqual(calls,['verify','weapons'])
        self.assertIn('did not establish',result['error'])
        self.assertIsNone(result['weapon_stage'])
        self.assertFalse(result['runtime_ready'])

    def test_focused_import_reports_missing_reader_and_verifier(self):
        with patch.object(m,'discover_weapon_tools'):
            code,result,calls=self.run_setup(weapons_only=True)
        self.assertEqual(code,2)
        self.assertEqual(calls,['verify'])
        self.assertIn('--weapon-tool',result['error'])
        self.assertIn('--weapon-verify-tool',result['error'])

    def test_weapon_tool_discovery_is_limited_to_bundled_tools_and_supplied_pak_sibling(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/'mh-weapon-packages.exe').write_bytes(b'synthetic')
            (root/'mh-verify-weapon-import.exe').write_bytes(b'synthetic')
            args=types.SimpleNamespace(pak_tool=root/'mh-pak.exe',weapon_tool=None,weapon_verify_tool=None)
            m.discover_weapon_tools(args)
            self.assertEqual(args.weapon_tool,root/'mh-weapon-packages.exe')
            self.assertEqual(args.weapon_verify_tool,root/'mh-verify-weapon-import.exe')
            explicit=types.SimpleNamespace(pak_tool=root/'mh-pak.exe',weapon_tool=Path('selected-reader.exe'),weapon_verify_tool=Path('selected-verifier.exe'))
            m.discover_weapon_tools(explicit)
            self.assertEqual(explicit.weapon_tool,Path('selected-reader.exe'))
            self.assertEqual(explicit.weapon_verify_tool,Path('selected-verifier.exe'))

    def test_detected_installation_is_still_verified_and_rejected_without_generating_cache(self):
        calls=[]
        def verify(game):
            calls.append(str(game));raise ValueError('Unsupported original EXE hash')
        native=types.SimpleNamespace(verify_install=verify)
        selected={'method':'running-mordhau','game_dir':'existing-install','game_exe':'existing-shipping.exe'}
        output=io.StringIO()
        with patch.object(m,'select_install',return_value=selected),patch.object(m,'load_stage',return_value=native),contextlib.redirect_stdout(output):
            code=m.main(['--cache-dir','synthetic-cache','--check'])
        result=json.loads(output.getvalue())
        self.assertEqual(code,2);self.assertEqual(calls,['existing-install'])
        self.assertEqual(result['installation'],selected)
        self.assertFalse(result['original_inputs_verified']);self.assertFalse(result['native_cache_generated'])
        self.assertIn('Unsupported original',result['error'])


if __name__=='__main__':unittest.main()
