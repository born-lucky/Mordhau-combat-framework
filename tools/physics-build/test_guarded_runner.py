"""Pure host source checks: no test child, native DLL, or Windows API entry."""
import unittest
import tempfile
from pathlib import Path
from run_guarded import TESTS, check_loaded_native, check_observations, check_source_stability, layouts
from build_bridge import metadata


def result(test, lines=""):
    return ("running 1 test\ntest " + TESTS[test] + " ... " + lines +
            "ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.01s\n")


def primitives():
    lines = ""
    for kind in range(3):
        lines += ("PHYSX_VALIDATION_BODY_VALUES id=0 mass_wanted=2 mass_actual=2 mass_hex=(2,2) "
                  "inertia_wanted=(80,80,80) inertia_actual=(80,80,80) com_wanted=(4,6,8) "
                  "com_actual=(4,6,8) q_wanted=(0,0,0,1) q_actual=(0,0,0,1)\n"
                  "PHYSX_VALIDATION_BODY id=0 shapes=1 mass=2 inertia=(80,80,80) com=(4,6,8)\n"
                  "PROBE production geometry kind" + str(kind) + " teardown\n"
                  "PHYSX_VALIDATION_RELEASE allocations=50 peak=30 live=0\n")
    return result("primitive", lines)


class RunnerTests(unittest.TestCase):
    def test_windows_x64_layouts_without_api(self):
        layouts()

    def test_interleaved_native_output(self):
        check_observations("primitive", primitives(), "")

    def test_uninterrupted_rejection(self):
        check_observations("reject", result("reject"), "")

    def test_wrong_count_ignored_name_and_duplicate_summary(self):
        for text in (result("reject").replace("1 passed", "0 passed"),
                     result("reject").replace("0 ignored", "1 ignored"),
                     result("reject").replace(TESTS["reject"], TESTS["primitive"]),
                     result("reject") * 2):
            with self.assertRaises(ValueError): check_observations("reject", text, "")

    def test_missing_mass_or_nonzero_release(self):
        for text in (primitives().replace("live=0", "live=1", 1),
                     primitives().replace("mass_wanted=2", "mass_wanted=3", 1),
                     primitives().replace("PROBE production geometry kind1 teardown", "")):
            with self.assertRaises(ValueError): check_observations("primitive", text, "")

    def test_rejection_has_no_native_creation(self):
        with self.assertRaises(ValueError):
            check_observations("reject", result("reject"), "PHYSX_VALIDATION_RELEASE allocations=1 peak=1 live=0\n")

    def test_loaded_dll_identity_checked_before_continue(self):
        expected = dict(path=str(Path("reviewed/mh_physx.dll")), size=100, sha256="a")
        check_loaded_native(expected, [expected])
        check_loaded_native(dict(path="kernel32.dll", size=50, sha256="b"), [expected])
        for actual in (dict(expected, sha256="b"), dict(expected, path=str(Path("wrong/mh_physx.dll")))):
            with self.assertRaises(ValueError): check_loaded_native(actual, [expected])

    def test_runner_and_test_source_stability(self):
        with tempfile.TemporaryDirectory() as folder:
            runner, test = Path(folder)/"runner.py", Path(folder)/"test.rs"
            runner.write_text("reviewed runner"); test.write_text("reviewed test")
            plan = dict(runner_sources=[metadata(runner)], current_test_sources=[metadata(test)])
            check_source_stability(plan)
            for changed in (runner, test):
                original = changed.read_text()
                changed.write_text("source drift")
                with self.assertRaises(ValueError): check_source_stability(plan)
                changed.write_text(original)
            check_source_stability(plan)


if __name__ == "__main__":
    unittest.main()
