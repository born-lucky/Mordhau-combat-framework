# Working with the combat framework

The project separates reusable combat behavior from the Bevy application and from
the original game's locally imported records. The API is experimental and is not
promised stable across future releases.

Use the engine-neutral entry crate, mh-framework, from a host or mod:

```rust
use mh_framework::{FighterDesc, SimInput};

let pawn = FighterDesc { name: "Player".into(), ..Default::default() };
let input = SimInput {
    attack: Some((2, 90.0)),
    ..Default::default()
};
```

The snippet describes input, not a complete world. A working host must supply a
valid locally imported weapon/specification, collision geometry and evaluated pose.
The input's move number is the native EAttackMove value; angles are degrees.

| Layer | Responsibility |
| --- | --- |
| mh-framework | One import surface for the engine-neutral APIs |
| mordhau-core | Combat state, attack phases, stamina, parry and timing rules |
| mh-sim | Movement, posed traces, grip/animation evaluation, collision and ragdoll integration |
| mh-host and mh-spec | Typed records, data loading and mod overlays |
| mh-pak, mh-assets, mh-level | Read the user's original packages and decode their assets |
| mh-runtime | Bevy rendering, input, camera, presentation and combat lab |
| mh-install and mh-setup | Original-install validation, setup UI and launch preflight |
| tools/setup.py | Generate required native and asset records into a local cache |
| demo | Reproducible showcase capture recipes |

Feed player or bot input through the same simulation API. Update the evaluated
component-space pose before the late weapon tracing phase. Render what the
simulation reports; do not run a second animation clock for the hit geometry.

For a mod layer, InstalledData::with_overlay applies typed entity/field changes
before building a world's specification. The runtime's combat timing tools expose
per-weapon phase lengths and curves. Preserve the imported baseline and store user
changes as a separate layer so the original values can be restored.

Core tests can use synthetic owned records. Tests against the original game need
the user's locally generated reference data; that data does not belong in Git.
Neither this facade nor the presence of an EXE proves ownership. Setup uses the
user's supported original files to produce a local cache. The intended launch
contract follows the reference's setup-and-cache flow; checking an EXE on every
launch is not required. Our current runtime still needs local paks and PhysX DLLs.

The complete consumer matrix importer and licensed native bridge are still being
finished. The framework source does not imply that the setup-to-play release has
passed its acceptance checks.

