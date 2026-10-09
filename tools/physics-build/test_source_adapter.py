"""Source/ABI-evidence tests only. Never invoke a compiler or load a DLL."""
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from compatibility import PIN, BASE_SHA256, adapt
from verify_callsites import EXPECTED, check
from build_bridge import output_directory


class OutputSafetyTests(unittest.TestCase):
    def test_game_overlap_rejected_before_mkdir(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            for cache, output, game in (
                (base / "game/cache", base / "game/cache/build", base / "game"),
                (base / "cache", base / "cache/build", base / "cache/build/game"),
                (base / "cache", base / "cache/build", base / "cache/game"),
            ):
                args = SimpleNamespace(cache_root=cache, output=output, game_dir=game)
                with self.subTest(cache=cache, output=output, game=game):
                    with patch.object(Path, "mkdir") as mkdir:
                        with self.assertRaisesRegex(ValueError, "installed game"):
                            output_directory(args)
                        mkdir.assert_not_called()

    def test_disjoint_cache_creates_only_fresh_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            args = SimpleNamespace(cache_root=base / "cache", output=base / "cache/build", game_dir=base / "game")
            with patch.object(Path, "mkdir") as mkdir:
                self.assertEqual(output_directory(args), args.output.resolve())
                mkdir.assert_called_once_with(parents=True, exist_ok=False)


class CompilerEvidenceTests(unittest.TestCase):
    def fixture(self):
        return "\n".join(f"{n} PROC\n mov rax, QWORD PTR [rcx]\n jmp QWORD PTR [rax+{v}]\n{n} ENDP\n" for n, v in EXPECTED.items())

    def test_all_observed_slots_required(self):
        self.assertEqual(len(check(self.fixture())), 23)
        with self.assertRaises(ValueError):
            check(self.fixture().replace("mh_abi_scene_fetch PROC", "wrong PROC"))

    def test_shifted_slot_refused(self):
        text = self.fixture().replace("jmp QWORD PTR [rax+488]", "jmp QWORD PTR [rax+528]")
        with self.assertRaises(ValueError):
            check(text)

    def test_duplicate_call_refused(self):
        with self.assertRaises(ValueError):
            check(self.fixture().replace("jmp QWORD PTR [rax+488]", "call QWORD PTR [rax+488]\n jmp QWORD PTR [rax+488]"))

    def test_msvc_rex_tail_jump_preserves_exact_offset_checks(self):
        text = self.fixture().replace("jmp QWORD PTR", "rex_jmp QWORD PTR")
        self.assertEqual(len(check(text)), 23)
        with self.assertRaises(ValueError):
            check(text.replace("rex_jmp QWORD PTR [rax+488]", "rex_jmp QWORD PTR [rax+528]"))


@unittest.skipUnless(os.environ.get("MH_BSD_TEST_REPO"), "set MH_BSD_TEST_REPO for pinned source integrity tests")
class PinnedSourceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        repo = Path(os.environ["MH_BSD_TEST_REPO"])
        cls.base = {p:subprocess.check_output(["git", "-C", str(repo), "show", PIN+":"+p]) for p in BASE_SHA256}
        cls.patched = {p:adapt(p, data)[0] for p, data in cls.base.items()}

    def file(self, suffix):
        return next(data.decode("utf-8") for p,data in self.patched.items() if p.endswith(suffix))

    def test_license_and_origin_retained_and_changed_input_refused(self):
        for path, base in self.base.items():
            with self.subTest(path=path):
                prefix = base.split(b"#include" if path.endswith(".cpp") else b"#ifndef", 1)[0]
                self.assertTrue(self.patched[path].startswith(prefix))
                with self.assertRaises(ValueError):
                    adapt(path, base + b"\n")

    def test_scene_changes_five_slots_not_other_methods(self):
        path = next(p for p in self.base if p.endswith("PxScene.h"))
        pattern = r"\bvirtual[^;]*?\b(\w+)\s*\([^;]*?=\s*0\s*;"
        before = re.findall(pattern, self.base[path].decode())
        after = re.findall(pattern, self.patched[path].decode())
        removed = {"setSceneQueryUpdateMode", "getSceneQueryUpdateMode", "sceneQueriesUpdate", "checkQueries", "fetchQueries"}
        self.assertEqual([n for n in before if n not in removed], after)
        self.assertEqual(len(before) - len(after), 5)
        self.assertIn("const PxBounds3&", self.file("PxScene.h"))

    def test_preserve_installed_descriptor_fields(self):
        scene = self.file("PxSceneDesc.h")
        self.assertIn("sceneQueryUpdateMode", scene)
        self.assertIn("maxBiasCoefficient", scene)
        for name in ("kineKineFilteringMode", "staticKineFilteringMode", "solverOffsetSlop"):
            self.assertNotRegex(scene, r"\b"+name+r"\s*(?:;|\()")
        cooking = self.file("PxCooking.h")
        self.assertIn("planeTolerance", cooking)
        for name in ("createTriangleMesh", "createConvexMesh"):
            declaration = re.search(r"virtual[^\n]*\b"+name+r"\([^\n]*", cooking)[0]
            self.assertEqual(declaration.count(","), 1)
            self.assertTrue(declaration.endswith(" const = 0;"))

    def test_solver_equations_are_bounded(self):
        solver = self.file("ExtD6JointSolverPrep.cpp")
        self.assertIn("const PxReal pad = data.tqSwingPad;", solver)
        self.assertIn("return Ps::tanHalf(s, 1.0f - s*s);", solver)
        self.assertEqual(solver.count("mhInstalledTanHalfFromSin("), 3)
        helper = self.file("ExtConstraintHelper.h")
        self.assertIn("mRa.cross(axis)", helper)
        self.assertNotIn("(mRa + errorVec).cross(axis)", helper)

    def test_convex_descriptor_has_original_field_order_without_later_margin(self):
        text = self.file("PxConvexMeshGeometry.h")
        declarations = re.findall(r"^\t[^\n;]*\b(scale|convexMesh|meshFlags|paddingFromFlags);", text, re.MULTILINE)
        self.assertEqual(declarations, ["scale", "convexMesh", "meshFlags", "paddingFromFlags"])
        self.assertNotIn("maxMargin", text)
        self.assertNotIn("float margin", text)
        self.assertIn("convexMesh\t(NULL)", text)


if __name__ == "__main__":
    unittest.main()
