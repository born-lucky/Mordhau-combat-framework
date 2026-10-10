import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('install_source', Path(__file__).parents[1] / 'install_source.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def original(self, root):
        exe = root / m.SHIPPING
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b'synthetic original; verification belongs to native_cache')
        return exe

    def test_selected_executable_resolves_existing_installation_without_discovery(self):
        exe = self.original(self.root / 'existing game')
        with patch.object(m, 'running_executables', side_effect=AssertionError('must not discover')):
            result = m.resolve_install(game_exe=exe)
        self.assertEqual(Path(result['game_dir']), self.root / 'existing game')
        self.assertEqual(result['method'], 'selected-executable')

    def test_rewrite_or_relocated_shipping_executable_is_rejected(self):
        for name in ('mordhau.exe', 'MordhauRewrite.exe', 'Mordhau-Win64-Shipping.exe'):
            exe = self.root / name
            exe.write_bytes(b'synthetic')
            with self.assertRaisesRegex(ValueError, 'original'):
                m.resolve_install(game_exe=exe)

    def test_running_original_has_priority_and_duplicates_are_one_installation(self):
        exe = self.original(self.root / 'running')
        with patch.object(m, 'running_executables', return_value=[exe, exe]), \
             patch.object(m, 'installed_executables', side_effect=AssertionError('must prefer running')):
            result = m.resolve_install()
        self.assertEqual(result['method'], 'running-mordhau')
        self.assertEqual(Path(result['game_exe']), exe)

    def test_multiple_running_installations_need_explicit_selection(self):
        exes = [self.original(self.root / name) for name in ('one', 'two')]
        with patch.object(m, 'running_executables', return_value=exes):
            with self.assertRaisesRegex(ValueError, 'Multiple'):
                m.resolve_install()

    def test_existing_steam_installation_is_used_when_game_is_closed(self):
        steam = self.root / 'Steam'
        library = self.root / 'Other library'
        (steam / 'steamapps').mkdir(parents=True)
        (library / 'steamapps').mkdir(parents=True)
        escaped = str(library).replace('\\', '\\\\')
        (steam / 'steamapps/libraryfolders.vdf').write_text(
            '"libraryfolders" { "0" { "path" "' + escaped + '" "apps" { "629760" "123" } } }')
        manifest = library / 'steamapps/appmanifest_629760.acf'
        manifest.write_text('// comment\n"AppState" { "appid" "629760" "installdir" "Mordhau" }')
        exe = self.original(library / 'steamapps/common/Mordhau')
        with patch.object(m, 'running_executables', return_value=[]), patch.object(m, 'steam_roots', return_value=[steam]):
            result = m.resolve_install()
        self.assertEqual(result['method'], 'existing-steam-installation')
        self.assertEqual(Path(result['game_exe']), exe)
        for bad in ('../other', '..', 'C:\\\\other'):
            manifest.write_text('"AppState" { "appid" "629760" "installdir" "' + bad + '" }')
            with patch.object(m, 'steam_roots', return_value=[steam]), self.assertRaisesRegex(ValueError, 'Invalid'):
                m.installed_executables()

    def test_no_existing_installation_has_actionable_error_without_creating_files(self):
        with patch.object(m, 'running_executables', return_value=[]), patch.object(m, 'installed_executables', return_value=[]):
            with self.assertRaisesRegex(ValueError, '--game-exe'):
                m.resolve_install()
        self.assertEqual(list(self.root.iterdir()), [])

    def test_windows_process_query_handles_singleton_array_and_inaccessible_path(self):
        exe = self.original(self.root / 'running')
        for records in ({'ProcessId': 12, 'ExecutablePath': str(exe)}, [{'ProcessId': 12, 'ExecutablePath': str(exe)}]):
            result = subprocess.CompletedProcess([], 0, json.dumps(records), '')
            with patch.object(m.sys, 'platform', 'win32'), patch.object(m.subprocess, 'run', return_value=result) as run:
                self.assertEqual(m.running_executables(), [exe])
                self.assertEqual(run.call_args.kwargs['timeout'], 20)
        with patch.object(m.sys, 'platform', 'win32'), patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '[{"ExecutablePath":null}]', '')):
            with self.assertRaisesRegex(ValueError, '--game-exe'):
                m.running_executables()

    def test_malformed_steam_metadata_is_not_silently_accepted(self):
        path = self.root / 'bad.vdf'
        for text in ('"x" { "y" "z"', '"x" "a" "x" "b"', '"x" bare', '"x" }'):
            path.write_text(text)
            with self.assertRaises(ValueError):
                m.read_vdf(path)


if __name__ == '__main__':
    unittest.main()
