"""Source-only preflight regressions. No game launch, native DLL load or real assets."""
from pathlib import Path
import json,subprocess,sys,tempfile,unittest
from showcase import preflight,sha

class PreflightTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.data=self.root/'local-data';self.data.mkdir();self.bridge=self.data/'state/physics/mh_physx.dll';self.bridge.parent.mkdir(parents=True);self.bridge.write_bytes(b'not a native DLL: preflight-only fixture')
        self.runtime=self.root/'runtime.fixture';self.runtime.write_bytes(b'not an executable: preflight-only fixture');self.manifest=self.root/'version.json'
        self.version={'schema_version':1,'source_commit':'a'*40,'runtime_sha256':sha(self.runtime),'bridge_sha256':sha(self.bridge)};self.save()
    def save(self):self.manifest.write_text(json.dumps(self.version),encoding='utf-8')
    def run_preflight(self,bridge=None,wall=None):return preflight(self.runtime,self.data,bridge or self.bridge,self.manifest,wall)
    def test_explicit_paths_and_full_identity(self):
        p=self.run_preflight();self.assertEqual(p['data_root'],str(self.data.resolve()));self.assertEqual(p['source_commit'],'a'*40);self.assertIsNone(p['wall_oracle'])
    def test_runtime_drift(self):
        self.runtime.write_bytes(b'changed fixture')
        with self.assertRaisesRegex(RuntimeError,'Runtime differs'):self.run_preflight()
    def test_bridge_drift(self):
        self.bridge.write_bytes(b'changed fixture')
        with self.assertRaisesRegex(RuntimeError,'Bridge differs'):self.run_preflight()
    def test_identical_wrong_bridge_location(self):
        copy=self.root/'different-bridge.fixture';copy.write_bytes(self.bridge.read_bytes())
        with self.assertRaisesRegex(RuntimeError,'exact file'):self.run_preflight(bridge=copy)
    def test_short_source_identity_rejected(self):
        self.version['source_commit']='a'*7;self.save()
        with self.assertRaisesRegex(RuntimeError,'complete hexadecimal'):self.run_preflight()
    def test_schema_rejected(self):
        self.version['schema_version']=2;self.save()
        with self.assertRaisesRegex(RuntimeError,'schema_version'):self.run_preflight()
    def test_optional_wall_requires_explicit_marker(self):
        wall=self.root/'wall.txt';wall.write_text('load_map TestLevel\n',encoding='utf-8')
        with self.assertRaisesRegex(RuntimeError,'marker'):self.run_preflight(wall=wall)
        wall.write_text('load_map TestLevel\ndump_state wall_strike_before\n',encoding='utf-8');self.assertEqual(self.run_preflight(wall=wall)['wall_oracle'],str(wall.resolve()))
    def test_cli_does_not_use_private_defaults(self):
        script=Path(__file__).with_name('showcase.py');p=subprocess.run([sys.executable,str(script),'preflight'],capture_output=True,text=True)
        self.assertEqual(p.returncode,2);self.assertIn('--runtime',p.stderr);self.assertIn('--data-root',p.stderr);self.assertIn('--bridge',p.stderr);self.assertIn('--version-manifest',p.stderr)

if __name__=='__main__':unittest.main()
