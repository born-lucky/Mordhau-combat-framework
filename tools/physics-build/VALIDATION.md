# Validation gates after compilation

A successful compilation is a prepared candidate, not permission to install or
execute it. Keep each candidate, source manifest, compatibility patch, notices,
compiler logs, assembly, commands and provenance immutable. Preserve failed
attempts in their own directories. Native dispatch belongs to the coordinating
reviewer after inspection of that candidate and the exact fixture sources.

Before a guarded test, require the candidate's output SHA256, source pin,
installed-DLL fingerprints, zero input drift and guarded mode in its provenance.
Review actual emitted factory version `0x03040000`, foundation version,
descriptor offsets, 23 checked virtual displacements and the typed eight-argument
geometry ray call with flags `0x607`. The restricted ray-output pointer must
match the installed export. Review extension assembly for the implemented D6
equations. Header declarations alone are insufficient. The diagnostic guard
must never become the installed production adapter.

Run one narrowly filtered fixture per isolated process, with an absolute
`MH_PHYSX_VALIDATION_BRIDGE`, explicit original `MORDHAU_DIR`, required original
data and a single test thread. Check process exit, diagnostics, actual readbacks
and allocator live-zero release lines; a test-name match or zero exit alone does
not establish the behavior. Do not use a missing-input skip as evidence.

The existing reviewed Rust fixtures provide these bounded gates:

1. `mh-physics` library test
   `production_open_rejects_guarded_bridge_before_creation`: ordinary production
   entry rejects instrumentation mode before calling the factory. It requires the
   previously accepted production gate; do not change that gate for a test.
2. `mh-physics` library test
   `production_body_geometry_mass_and_teardown`: separate sphere, box and
   capsule scenes use the production mass/inertia/COM path, actual offset shapes
   and COM nudge, 120 gravity steps, removal, empty-scene step and full drop.
   Require three mass/inertia/COM guard observations and three live-zero releases.
3. `mh-physics` integration test `cooked_original`, exact test
   `original_wall_full_arrays_rays_and_teardown`: verified installed-pak payload,
   full native vertex/index/material hash, native material totals, closest/front/
   back/double-sided/miss rays, a convex-versus-triangle distinction and two clean
   releases. Corrupt/truncated inputs must reject before deserialization. The ray
   placement is a mesh-local diagnostic; it is not a world-level gameplay oracle.
4. `mh-sim` integration test `sim`, exact test
   `original_physx_death_bodies_fall_and_notify_starts_simulation`: original pak/
   character/matrix inputs exercise immediate and notify-delayed deaths, 16
   bodies and 15 D6 anchors each, floor contact, finite poses, hip fall, original
   notify timing and respawn cleanup. Require two live-zero releases and actual
   body/joint observations. Preserve the existing numerical assertions. This
   proves bounded lifecycle/connectivity, not identical original trajectories.

Typical root-coordinated commands, after building the test executables in a
separately scheduled memory window, are:

```powershell
cargo test -p mh-physics --features native-validation --lib tests::production_open_rejects_guarded_bridge_before_creation -- --ignored --exact --nocapture --test-threads=1
cargo test -p mh-physics --features native-validation --lib tests::production_body_geometry_mass_and_teardown -- --ignored --exact --nocapture --test-threads=1
cargo test -p mh-physics --features native-validation --test cooked_original original_wall_full_arrays_rays_and_teardown -- --ignored --exact --nocapture --test-threads=1
cargo test -p mh-sim --features native-validation --test sim original_physx_death_bodies_fall_and_notify_starts_simulation -- --ignored --exact --nocapture --test-threads=1
```

Library tests live inside a `tests` module: for strict `--exact`, use their full
`tests::...` names, as reported by the compiled test executable's `--list` output.
Alternatively, run the already-built, source-bound test executable directly;
never infer its source/features from its filename alone.

Additional required parity work remains separate. Exercise hard/soft cone limits,
one swing axis free and locked-linear lever arms against independently frozen
original equations or an accepted local original-solver reference. Compare row
count, axes, lever arms, geometric error, flags and limit parameters with a
predeclared numeric criterion; do not weaken it after a failed observation. The
existing death fixture can pass without covering every changed solver branch.

Only after these gates and independent review should a fresh normal-mode build
use the same physical sources and patches. Recheck its provenance and mode, then
test actual original-data startup, rendered contact/death, map/respawn lifetime
and teardown in the separate installation/cache. Keep the package private until
the full setup/import/launch and source/binary artifact audits pass. The main
canonical runtime and installed DLLs are not replaced by these tools.
