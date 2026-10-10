"""Synthetic orchestration evidence: rejection must precede cache publication."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('weapon_cache_under_test', Path(__file__).with_name('weapon_cache.py'))
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


def digest(path, kind):
    return hashlib.new(kind, Path(path).read_bytes()).hexdigest()


class WeaponImportOrchestration(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.game = self.root / 'existing-original'
        self.game.mkdir()
        self.cache = self.root / 'external-cache'
        self.reader = self.root / 'mh-weapon-packages.exe'
        self.verifier = self.root / 'mh-verify-weapon-import.exe'
        self.reader.write_bytes(b'synthetic reader identity')
        self.verifier.write_bytes(b'synthetic verifier identity')
        self.records = {'SyntheticOne': {'weapon': {'id': 'SyntheticOne'}},
                        'SyntheticTwo': {'weapon': {'id': 'SyntheticTwo'}}}
        self.unknown = [{'package': 'Synthetic/Two', 'field': 'EquipmentName',
                         'reason': 'Opaque native catalog text; no original label invented'}]
        self.defaults = {'schema_version': 1, 'classes': {'SyntheticClass': {'SyntheticField': 1.25}}}
        self.review = {'schema_version': 1, 'classes': {'SyntheticClass': [{'field': 'SyntheticField', 'accepted': True}]}}
        self.packages = {'weapons': ['Synthetic/One', 'Synthetic/Two'], 'records': {}, 'errors': []}
        self.original_paks = [{'path': 'Synthetic.pak', 'bytes': 101, 'mtime_ns': 102}]
        self.current_paks = copy.deepcopy(self.original_paks)
        self.acceptance = {'rust_consumers_accepted': True, 'runtime_ready': False, 'weapon_records': 2}
        self.mutate_constructor = False
        self.mutate_packages = False
        self.identity_mismatch = False
        self.verifier_exit = 0
        self.native = types.SimpleNamespace(
            EXE_SHA1='synthetic-executable', PDB_SHA1='synthetic-symbols', digest=digest,
            verify_install=Mock(return_value=(self.game, None, None, None)),
            commit_generated=Mock(side_effect=self.publish))
        self.constructor = self.patch(stage.constructor_cache, 'generate_reviewed', side_effect=self.generate_native)
        self.package_reader = self.patch(stage.weapon_packages, 'generate', side_effect=self.generate_packages)
        self.package_validate = self.patch(stage.weapon_packages, 'validate', side_effect=lambda value: value)
        self.pak_metadata = self.patch(stage.weapon_packages, 'pak_metadata', side_effect=lambda game: self.current_paks)
        self.record_builder = self.patch(stage.weapon_records, 'build_records', side_effect=lambda *args: (self.records, self.unknown))
        self.matrix = self.patch(stage.weapon_matrix, 'generate', side_effect=self.generate_matrix)
        self.verify_process = self.patch(stage.subprocess, 'run', side_effect=self.run_verifier)

    def patch(self, module, name, **kwargs):
        replacement = patch.object(module, name, **kwargs)
        result = replacement.start()
        self.addCleanup(replacement.stop)
        return result

    def receipt(self):
        return {'exe_sha1': self.native.EXE_SHA1, 'pdb_sha1': self.native.PDB_SHA1}

    def output(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding='utf-8')
        return {'path': str(path), 'sha256': digest(path, 'sha256')}

    def generate_native(self, game, cache):
        cache = Path(cache)
        defaults = self.output(cache / 'defaults.json', self.defaults)
        review = self.output(cache / 'review.json', self.review)
        receipt = {**self.receipt(), 'defaults': defaults['path'], 'review': review['path'],
                   'outputs': [defaults, review]}
        if self.mutate_constructor:
            Path(defaults['path']).write_text('{"mutated": true}', encoding='utf-8')
        return receipt

    def generate_packages(self, game, cache, reader, native):
        output = self.output(Path(cache) / 'packages.json', self.packages)
        receipt = {**self.receipt(), 'outputs': [output], 'original_paks': copy.deepcopy(self.original_paks)}
        if self.identity_mismatch:
            receipt['exe_sha1'] = 'different-original'
        if self.mutate_packages:
            Path(output['path']).write_text('{"mutated": true}', encoding='utf-8')
        return receipt

    def generate_matrix(self, records, work, native, packages):
        matrix = Path(work) / 'synthetic-matrix'
        self.output(matrix / 'index.json', {'counts': {'weapon': len(records)}})
        return {'matrix': str(matrix), 'counts': {'weapon': len(records)}}

    def run_verifier(self, command, **kwargs):
        return subprocess.CompletedProcess(command, self.verifier_exit, json.dumps(self.acceptance),
                                           'synthetic rejection' if self.verifier_exit else '')

    def publish(self, cache, products):
        outputs = []
        for relative, data in products.items():
            path = Path(cache) / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            outputs.append({'path': str(path), 'sha256': digest(path, 'sha256')})
        return outputs

    def generate(self, cache=None):
        return stage.generate(self.game, self.cache if cache is None else cache,
                              self.reader, self.verifier, self.native)

    def assert_unpublished(self):
        self.native.commit_generated.assert_not_called()
        self.assertFalse((self.cache / 'data_gen/weapons/records.json').exists())

    def test_success_consumes_record_unknown_tuple_and_publishes_only_verified_records(self):
        result = self.generate()
        self.assertTrue(result['complete'])
        self.assertTrue(result['weapon_data_ready'])
        self.assertFalse(result['runtime_ready'])
        self.assertEqual(result['catalog_names_unavailable'], self.unknown)
        self.assertEqual(json.loads((self.cache / 'data_gen/weapons/records.json').read_text()), self.records)
        self.assertEqual(self.native.verify_install.call_count, 2)
        self.native.commit_generated.assert_called_once()
        self.assertEqual(self.record_builder.call_args.args, (self.packages, self.defaults, self.review))
        self.assertIs(self.matrix.call_args.args[0], self.records)
        verifier_args = self.verify_process.call_args.args[0]
        self.assertEqual(Path(verifier_args[0]), self.verifier)
        self.assertEqual(json.loads(Path(verifier_args[2]).read_text()), self.records)

    def test_verifier_process_rejection_never_publishes(self):
        self.verifier_exit = 1
        with self.assertRaises(subprocess.CalledProcessError):
            self.generate()
        self.assert_unpublished()
        self.assertEqual(self.native.verify_install.call_count, 1)

    def test_acceptance_flags_are_required_without_full_runtime_claim(self):
        for field, value in [('rust_consumers_accepted', False), ('runtime_ready', True)]:
            with self.subTest(field=field):
                self.acceptance = {'rust_consumers_accepted': True, 'runtime_ready': False, 'weapon_records': 2}
                self.acceptance[field] = value
                with self.assertRaisesRegex(ValueError, 'did not establish acceptance'):
                    self.generate()
                self.assert_unpublished()

    def test_mutated_native_output_is_rejected_before_records_are_built(self):
        self.mutate_constructor = True
        with self.assertRaisesRegex(ValueError, 'constructor output changed'):
            self.generate()
        self.package_reader.assert_not_called()
        self.record_builder.assert_not_called()
        self.verify_process.assert_not_called()
        self.assert_unpublished()

    def test_staged_inputs_mutated_during_successful_verifier_never_publish(self):
        for relative, message in [('native/defaults.json', 'constructor output changed'),
                                  ('native/review.json', 'constructor output changed'),
                                  ('synthetic-matrix/index.json', 'input changed during verification'),
                                  ('weapon_records.json', 'input changed during verification')]:
            with self.subTest(relative=relative):
                def mutate_then_accept(command, **kwargs):
                    target = Path(kwargs['cwd']) / relative
                    self.assertTrue(target.is_file())
                    target.write_text('{"mutated_while_verifier_ran": true}', encoding='utf-8')
                    return self.run_verifier(command, **kwargs)
                self.verify_process.side_effect = mutate_then_accept
                with self.assertRaisesRegex(ValueError, message):
                    self.generate()
                self.assert_unpublished()

    def test_mutated_package_output_is_rejected_before_records_are_built(self):
        self.mutate_packages = True
        with self.assertRaisesRegex(ValueError, 'package records changed'):
            self.generate()
        self.record_builder.assert_not_called()
        self.verify_process.assert_not_called()
        self.assert_unpublished()

    def test_original_native_and_package_identities_must_match(self):
        self.identity_mismatch = True
        with self.assertRaisesRegex(ValueError, 'input identities differ'):
            self.generate()
        self.record_builder.assert_not_called()
        self.assert_unpublished()

    def test_rust_and_source_counts_must_match_records_and_package_census(self):
        for kind in ('rust_count', 'package_count'):
            with self.subTest(kind=kind):
                self.acceptance['weapon_records'] = 3 if kind == 'rust_count' else 2
                self.packages['weapons'] = ['Synthetic/One', 'Synthetic/Two', 'Synthetic/Three'] if kind == 'package_count' else ['Synthetic/One', 'Synthetic/Two']
                with self.assertRaisesRegex(ValueError, 'record counts differ'):
                    self.generate()
                self.assert_unpublished()

    def test_original_pak_change_before_publication_rejects(self):
        self.current_paks[0]['mtime_ns'] += 1
        with self.assertRaisesRegex(ValueError, 'pak inputs changed'):
            self.generate()
        self.assertEqual(self.native.verify_install.call_count, 2)
        self.assert_unpublished()

    def test_original_reverification_failure_never_publishes(self):
        self.native.verify_install.side_effect = [(self.game, None, None, None), ValueError('original updated')]
        with self.assertRaisesRegex(ValueError, 'original updated'):
            self.generate()
        self.assert_unpublished()

    def test_cache_inside_original_or_published_source_is_rejected_before_writes(self):
        for cache in (self.game, self.game / 'nested-cache', stage.ROOT.parents[1] / 'consumer-import-test-forbidden'):
            with self.subTest(cache=str(cache)):
                with self.assertRaisesRegex(ValueError, 'outside|published source'):
                    self.generate(cache)
                self.constructor.assert_not_called()
                self.package_reader.assert_not_called()
                self.verify_process.assert_not_called()
                self.assert_unpublished()
                if cache != self.game:
                    self.assertFalse(cache.exists())


if __name__ == '__main__':
    unittest.main()
