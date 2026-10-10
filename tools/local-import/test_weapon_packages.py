"""The package stage must never substitute absent native values or accept partial reads."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('weapon_packages', Path(__file__).with_name('weapon_packages.py'))
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


def fixture():
    records = {}
    weapons = []
    for root, count in stage.ROOT_COUNTS.items():
        for index in range(count):
            path = f'Mordhau/Content/Synthetic/{root}_{index:03}'
            weapons.append(path)
            records[path] = {'package': path, 'native_root': root, 'chain_child_first': [path],
                             'serialized_defaults': {'StrikeAttack': {'Windup': 0.5}}, 'maps_merged': {},
                             'native_defaults_included': False, 'decode_errors': [],
                             'original_packages': [{'package': path, 'exports': [{'export_index': 1,
                                                  'record': {'Name': 'Default__Synthetic_C', 'Properties': {}}}]}]}
    return {'schema': 1, 'source': 'original-paks', 'runtime_ready': False,
            'weapons': sorted(weapons), 'records': records, 'errors': []}


class WeaponPackageValidation(unittest.TestCase):
    def test_serialized_deltas_remain_without_fabricated_native_fields(self):
        payload = fixture()
        validated = stage.validate(payload)
        attack = validated['records'][validated['weapons'][0]]['serialized_defaults']['StrikeAttack']
        self.assertEqual(attack, {'Windup': 0.5})
        self.assertNotIn('Release', attack)
        self.assertFalse(validated['runtime_ready'])

    def test_missing_weapon_or_wrong_native_root_rejected(self):
        for change in ('missing_record', 'wrong_root', 'duplicate_weapon', 'wrong_census'):
            with self.subTest(change=change):
                payload = fixture()
                path = payload['weapons'][0]
                if change == 'missing_record':
                    del payload['records'][path]
                elif change == 'wrong_root':
                    payload['records'][path]['native_root'] = 'MordhauEquipment'
                elif change == 'duplicate_weapon':
                    payload['weapons'].append(path)
                else:
                    payload['weapons'].remove(path)
                with self.assertRaises(ValueError):
                    stage.validate(payload)

    def test_decode_failures_and_unsupported_values_rejected(self):
        for change in ('error', 'decode', 'unsupported', 'nonfinite', 'native_defaults', 'runtime_ready'):
            with self.subTest(change=change):
                payload = fixture()
                record = payload['records'][payload['weapons'][0]]
                if change == 'error':
                    payload['errors'] = [{'error': 'missing superclass'}]
                elif change == 'decode':
                    record['decode_errors'] = [{'error': 'unsupported struct'}]
                elif change == 'unsupported':
                    record['serialized_defaults']['Damage'] = {'__unsupported__': 'struct'}
                elif change == 'nonfinite':
                    record['serialized_defaults']['Length'] = float('nan')
                elif change == 'native_defaults':
                    record['native_defaults_included'] = True
                else:
                    payload['runtime_ready'] = True
                with self.assertRaises(ValueError):
                    stage.validate(payload)

    def test_broken_inheritance_or_missing_cdo_rejected(self):
        for change in ('cycle', 'wrong_provenance', 'missing_cdo', 'invalid_path', 'missing_exports'):
            with self.subTest(change=change):
                payload = fixture()
                path = payload['weapons'][0]
                record = payload['records'][path]
                if change == 'cycle':
                    record['chain_child_first'] += [path]
                elif change == 'wrong_provenance':
                    record['original_packages'][0]['package'] = 'Mordhau/Content/Synthetic/Other'
                elif change == 'missing_cdo':
                    record['original_packages'][0]['exports'][0]['record']['Name'] = 'Function'
                elif change == 'invalid_path':
                    record['chain_child_first'] += ['Mordhau/Content/../Other']
                else:
                    record['original_packages'][0]['exports'] = []
                with self.assertRaises(ValueError):
                    stage.validate(payload)

    def test_each_ancestor_has_cdo_and_is_read_parent_first(self):
        payload = fixture()
        path = payload['weapons'][0]
        record = payload['records'][path]
        parent = 'Mordhau/Content/Synthetic/Parent'
        original = copy.deepcopy(record['original_packages'][0])
        original['package'] = parent
        record['chain_child_first'].append(parent)
        record['original_packages'].insert(0, original)
        self.assertIs(stage.validate(payload), payload)
        record['original_packages'].reverse()
        with self.assertRaises(ValueError):
            stage.validate(payload)


if __name__ == '__main__':
    unittest.main()
