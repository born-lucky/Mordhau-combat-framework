# build_matrix.py - Mordhau's matrix build: the game-agnostic spec-kit (tools/spec-kit, speckit.matrix) with this
# repo's settings. The Spreadsheet Method's "generate, do not hand-copy" step (docs/methods/SPREADSHEET-METHOD.md §4,
# docs/SPEC_SHEETS.md): read the workbook, fail on duplicate IDs, broken references, invalid literals, unserialized
# engine objects and schema gaps, write the generated JSON. Our own implementation (the rustports toolkit was not
# downloaded or run).
#
#   python tools/sheets/build_matrix.py            sheets/mordhau_spec.xlsx -> data_gen/spec/ (never hand-edit the JSON)
#   python tools/sheets/build_matrix.py --check    rebuild in memory; exit 1 if data_gen/spec/ is missing or stale
#   python tools/sheets/build_matrix.py --selftest tamper with the workbook in memory; every defect class must be caught
#   options: --workbook <xlsx> --out <dir>
import pathlib, sys

R = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(R / "tools" / "spec-kit"))
from speckit import matrix  # noqa: E402

FORMAT = "mordhau-spec/1"
matrix.configure(root=R, fmt=FORMAT)
# the kit's API, re-exported for callers that import this module
read, outputs, build, selftest = matrix.read, matrix.outputs, matrix.build, matrix.selftest


def main(argv):
    return matrix.main(argv, workbook=R / "sheets" / "mordhau_spec.xlsx", out=R / "data_gen" / "spec")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
