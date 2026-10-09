# Local import status

The setup executable's local-import action calls:

```
python tools/setup.py --game-dir "C:/Program Files (x86)/Steam/steamapps/common/Mordhau" --cache-dir "USER-LOCAL-CACHE" --json
```

`--verify-tool PATH` optionally calls an existing `verify-install.exe` before the Python check. `--check` verifies original EXE/PDB/version/pak/DLL inputs without creating any cache. The implemented normal action generates the native cache stage and returns exit 2 because the complete consumer importer remains unfinished. Its final JSON explicitly reports `runtime_ready:false`; it must never be interpreted as permission to launch. The setup host separately checks the full runtime gate before launch.

The supported original Shipping EXE and PDB are read from the selected installation and verified by SHA1. The installed PDB exists on the development installation; another installation lacking this matching PDB is rejected explicitly. No private symbol dump, generated record, original asset, or decompiler output is bundled as a fallback. The original installation is read-only.

The native stage uses a source-only MSF/CodeView decoder to read actual complete PDB motion types and write 13 native headers. The required native float and seven name records are read directly from file-backed PE ranges. Each name must bind to exactly one original field identity through structured PDB dynamic initializers (public symbol where available, otherwise a genuine DBI module procedure record) whose original instructions reference that exact name literal. Equivalent translation-unit initializer copies are preserved as a set of RVAs; conflicting field identities are rejected. Capstone 5.0.7 decodes only those bounded initializers. No original numeric value or bone-name value is embedded in the importer. Output is eight `rdata.tsv` rows, 13 headers, and a local receipt recording original hashes and generated file hashes. Existing divergent output is preserved and rejected; unchanged output is idempotent. Generated outputs cannot be placed in the game installation or published source directories.

Remaining launch blockers are the full generated spec matrix, body shadow capsules unless the pak stage completes, and a permitted native physics bridge build. The reviewed rewrite WGSL programs ship as our source code; raw original cooked shader bytes never ship or serve as an import fallback. This native stage supplies actual runtime inputs; it is not a replacement for those missing stages.

The existing private generation chain cannot simply be relabeled a consumer installer. `build.py native` expects `modules.txt`, public-label/global/local tables, PDB layout/prototype tables and additional script outputs. Several scripts still hardcode a developer PDB path. `gen_spec_src.gd` needs the legacy Godot record-reader source, constructor decompilation and full native layouts; some enumeration paths read extracted JSON. `sheets_populate.py` consumes additional bytecode, map and record outputs before `tools/sheets/build_matrix.py` can generate every indexed entity file. Capsules are generated separately; the reviewed WGSL program sources are our own code and need no original shader import. Packaging the source tools is feasible, but their dependencies must be reconstructed locally and their outputs checked against actual consumers before runtime-ready status or public release.

A PDB-addressed literal constructor decoder is now implemented as a diagnostic source tool, followed by the packaged pak-backed record writers once its defaults are judged. A full Ghidra pipeline would need a clean local project, locally generated procedure/type metadata, pinned tool versions and serialized resource use. No Ghidra job or original-game launch is performed by the current native step.

`--pak-tool PATH` additionally uses the source-built `mh-pak` CLI to read the original shadow PhysicsAsset and write only stored capsules into the local cache. It validates exact same-package body export references and every required geometry component. Synthetic tests do not establish original package correctness; the original-input reader and consumer format must also be checked.

The CLI also discovers `tools/bin/mh-pak.exe` when no `--pak-tool` is supplied. This must be built from the accompanying reader source; no private binary or original game executable is substituted. The development proof used an isolated Cargo target and generated all 41 original capsules on 23 bones. Their binary32 geometry and bone identities matched the existing local-only reference as a multiset; source output follows original body-reference order and the renderer resolves each proxy by bone name. The native stage regenerated eight required records and 13 motion layouts matching the local reference. Those observations establish these two stages only, not full setup or game launch.

The constructor diagnostic command is:

```
python tools/local-import/constructor_cache.py --game-dir "GAME-INSTALL" --cache-dir "USER-LOCAL-CACHE" --class-name FAttackInfo
```

It generates a fresh DBI procedure inventory and complete PDB layouts, then replays literal stores without executing original code. It tracks object aliases, nested direct constructor calls, immediate/RIP scalar and vector stores, and the legacy reader's lexical float-array append contract. Unsupported instructions/calls/control flow and skipped field types are recorded explicitly. Folded constructors qualify only when their original RVA and code size agree; all aliases remain in the receipt. These diagnostic outputs are not installed as consumer defaults and never declare the matrix ready.

`reader-source/` contains 94 tracked rewrite source files, with the original source revision and individual file hashes kept in the local development proof. Its AI-tree and map enumerations use the installed pak backend; no private JSON directory or manifest is assumed. The external-cache headless tool harness and full matrix conversion remain under reconstruction. No Godot import/export job has yet been accepted as a completed consumer path.

A separate, bounded development oracle executed the existing reader for four native types in an isolated Godot project. It reads the developer's local original-derived files only for comparison; the setup command does not read those files. This caught two errors in an independent Python translation of the legacy reader and additional packed scalar/vector instructions missing from the fresh decoder. Failed comparisons remain preserved. Agreement between readers alone does not prove unresolved native branches or make a constructor cache usable: missing fields, new fields, unsupported operations and actual consuming source must also be reviewed. In particular, an undecoded native text object must not be presented as an empty dictionary or fabricated text default.

`native_reflection.py` implements another source-only prerequisite: structured PDB data-symbol lookup and typed `FEnumParams` table reads for enum records. Pointer identity, original member offsets, bounds and ambiguity are checked. Synthetic tests cover those contracts; actual original-input acceptance and matrix integration are still pending.
