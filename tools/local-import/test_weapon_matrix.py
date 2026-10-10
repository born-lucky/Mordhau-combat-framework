"""Synthetic repeat-import contracts; no original game data is a fixture."""
import copy
from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest


ROOT = Path(__file__).parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fixtures = load('test_weapon_records')
weapon_matrix = load('weapon_matrix')


def records():
    packages, defaults, review = fixtures.complete_fixture()
    return fixtures.w.build_records(packages, defaults, review)[0]


def receipts(stage):
    native = {'exe_sha1': 'synthetic-executable', 'pdb_sha1': 'synthetic-symbols',
              'review': str(stage / 'constructor-review.json'),
              'outputs': [{'path': str(stage / 'constructor-defaults.json'), 'sha256': 'a' * 64},
                          {'path': str(stage / 'constructor-review.json'), 'sha256': 'b' * 64}]}
    package = {'outputs': [{'path': str(stage / 'weapon-packages.json'), 'sha256': 'c' * 64}]}
    return native, package


def matrix_bytes(directory):
    return {p.relative_to(directory).as_posix(): p.read_bytes()
            for p in sorted(directory.rglob('*.json'))}


def database_rows(path):
    with closing(sqlite3.connect(path)) as db:
        return {table: db.execute('SELECT * FROM ' + table + ' ORDER BY 1,2').fetchall()
                for table in ('entities', 'fields', 'entity_values')}


class RepeatImportTests(unittest.TestCase):
    def test_stage_paths_do_not_change_validated_matrix_or_database(self):
        with tempfile.TemporaryDirectory() as temporary:
            stages = [Path(temporary) / name for name in ('first-import', 'later-import')]
            results = []
            for stage in stages:
                stage.mkdir()
                native, package = receipts(stage)
                results.append(weapon_matrix.generate(records(), stage, native, package))
            first, later = results
            first_files = matrix_bytes(Path(first['matrix']))
            self.assertEqual(first_files, matrix_bytes(Path(later['matrix'])))
            self.assertEqual(database_rows(first['database']), database_rows(later['database']))
            self.assertEqual(first['counts'], later['counts'])
            self.assertEqual(first['counts']['entities'], 10)
            sources = first_files['sources.json'].decode()
            for stage in stages:
                self.assertNotIn(str(stage), sources)
                self.assertNotIn(stage.name, sources)

    def test_original_constructor_digest_changes_provenance_and_content_identity(self):
        native, package = receipts(Path('synthetic-stage'))
        first = weapon_matrix.make_book(records(), native, package)
        changed = copy.deepcopy(native)
        changed['outputs'][1]['sha256'] = 'd' * 64
        second = weapon_matrix.make_book(records(), changed, package)
        with tempfile.TemporaryDirectory() as temporary:
            generated = []
            for index, book in enumerate((first, second)):
                headers, rows = weapon_matrix.book.sheets(book)
                path = Path(temporary) / f'{index}.xlsx'
                weapon_matrix.book.write_book(path, headers, rows)
                errors, files = weapon_matrix.matrix.outputs(weapon_matrix.matrix.read(path))
                self.assertEqual(errors, [])
                generated.append(files)
            a, b = generated
            self.assertNotEqual(a['sources.json'], b['sources.json'])
            self.assertNotEqual(json.loads(a['index.json'])['content_sha1'],
                                json.loads(b['index.json'])['content_sha1'])
            self.assertEqual(a['entities/weapon.json'], b['entities/weapon.json'])
            self.assertEqual(a['entities/attack.json'], b['entities/attack.json'])


if __name__ == '__main__':
    unittest.main()
