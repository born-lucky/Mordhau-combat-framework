# Licensed installed-PhysX adapter

This builds our C bridge and three NVIDIA extension translation units from the
owner-published BSD-3-Clause revision
`5e42a5f112351a223c19c17bb331e6c55037b8eb` of
[PhysX-3.4](https://github.com/NVIDIAGameWorks/PhysX-3.4/tree/5e42a5f112351a223c19c17bb331e6c55037b8eb).
SDK sources are acquired into ignored build directories or the user's local
cache. This repository contains our build tools and adaptations; it does not
vendor the SDK or distribute installed game DLLs.

Source preparation, without a compiler or DLL entry:

```powershell
python tools/physics-build/build_bridge.py
# Optional: use an existing Git object store. Its HEAD/worktree is never read.
python tools/physics-build/build_bridge.py --sdk-repository C:/Tools/PhysX-3.4
```

Compilation is a separate explicit gate, requiring Git and MSVC x64 tools:

```powershell
python tools/physics-build/build_bridge.py --compile --guarded-validation --game-dir "C:/Program Files (x86)/Steam/steamapps/common/Mordhau"
```

`--output` must name a fresh child of `build/physics`, or a fresh child of an
explicit `--cache-root` outside the source tree. The script does not overwrite,
install, load or run a DLL. It checks the supported original executable and all
four linked installed DLL fingerprints before compilation. Original DLLs remain
in place; temporary import libraries and export listings stay in the cache.
An explicit game directory also excludes cache/output paths inside it or above
it; this check runs before any output directory is created.

`source-manifest.json` binds every acquired Git blob and its original/adapted
SHA256. `compatibility.patch` records the exact local modifications. All owner
license preambles remain intact. `NVIDIA-PHYSX-NOTICES.txt` retains distinct
dependency copyrights, conditions and disclaimers and must accompany any
eventually distributed compiled adapter. No endorsement is implied.

The source remains 3.4.2. `mh_installed_physx.h` supplies the installed 3.4.0
factory version without rewriting NVIDIA's source-version macro. The bridge's
existing C ABI stays unchanged; an additional `mh_px_licensed_source` export
reports the licensed source revision.

The implemented local edits are:

- Remove later kinematic pair filters and solver offset slop from `PxSceneDesc`.
  Keep native `sceneQueryUpdateMode` and `maxBiasCoefficient`.
- Keep the native cooking `planeTolerance` field, restore installed constness
  and the two-argument mesh-creation methods.
- Remove exactly five later `PxScene` query-update virtual methods and restore
  the visualization getter's reference return.
- Remove the later convex geometry margin field and associated constructor/
  validation additions, preserving installed pointer32/flags40/padding41.
- Restore locked-linear angular lever arms, soft-cone padding and the
  single-swing-free double-cone equation used by the installed solver.

The BSD `PxBaseTask` already has the installed context-ID layout. Its fields
need no adaptation. The licensed `ExtJoint.cpp` is used directly. Headers absent
from the BSD revision are not substituted from an older SDK.

The builder compiles authored descriptor/D6 assertions, retains emitted
assembly, checks 23 critical virtual-call displacements, audits every observed
non-system include against the pinned inventory, and records source/tool/DLL
hashes, commands and input drift. Those compile checks do not establish runtime
safety. Remaining acceptance gates are original cooked geometry/material/ray
readback and teardown, sphere/box/capsule mass/COM/freefall, 16-body/15-D6
immediate/notify/respawn fixtures, solver-specific hard/soft/one-free cases, then
normal-build provenance and actual rendered contact/death. Keep the package
private until the complete installer/import/runtime validation passes.

Untested scope remains explicit: BSD joint relative-transform getters and D6
visualization are retained; they are not used by this bridge. Dynamic convex
body mass parity is not claimed. Primitive dynamic bodies and original cooked
static meshes are the supported validation scope.

Source-only checks (no compiler/native entry):

```powershell
$env:MH_BSD_TEST_REPO = 'C:/Tools/PhysX-3.4'
python -m unittest discover -s tools/physics-build -p test_source_adapter.py
```

The compiler-evidence tests use synthetic assembly and only test the checker.
The five pinned-source tests read the exact owner's Git objects. Actual emitted
assembly is checked only during an explicitly requested build.
