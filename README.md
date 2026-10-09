# Mordhau Combat Framework — Rust v1

An experimental Rust and Bevy melee combat framework built from the MORDHAU
rewrite. It separates combat simulation, locally imported game records, and
presentation so hosts and mods can reuse the combat systems.

The distribution follows the structure of
[ELDEN-RING-Combat-Rewrite](https://github.com/Funny-Bones/ELDEN-RING-Combat-Rewrite):
source code, a local Python setup step, a Cargo application, custom stand-in
visuals, and scripted combat demonstrations. This is a separate repository with
fresh history; development here does not replace the main installed combat build.

**Private staging: the complete setup-to-play path is still being built and tested.
There is no public playable download yet.**

## Combat and framework

The source includes attack phases and pose evaluation, normal/alternate grips,
posed weapon traces, parry collision, stamina and damage rules, environmental
hit-stop, effects, and physical death ragdolls. Combat timing tools expose
per-weapon values and curves as a separate mod layer.

mh-framework provides an entry point to the engine-neutral simulation and typed
data APIs. The Bevy application supplies input, camera, rendering, and the combat
lab. The authored setup interface and health/stamina presentation are implemented;
the bars read live fighter values and bounds rather than introducing new rules.
The fork enables authored training figures and generic weapons; see
[presentation and current limitations](docs/PRESENTATION.md).

Original-game parity is ongoing. This is not a claim of complete 1:1 behavior
across every weapon, attack phase, effect, map, or multiplayer mode. The existing
recorded baseline showcase does not verify the new distribution. A separate
498-frame developer-input render has now checked custom figures, normal/alternate
Greatsword grips, live HUD damage values, blood impacts, and a physical death.
Clean consumer setup and clearly visible parry sparks remain unverified.

## Local setup

Game content is not supplied by this repository. Use a legitimately purchased
supported installation. The importer reads local game files and writes generated
records to an external user cache. No downloader or private exported-data fallback
is included.

The current diagnostic setup command is:

```text
python tools/setup.py --game-dir "YOUR-MORDHAU-INSTALL" --cache-dir "YOUR-LOCAL-CACHE" --json
```

Native records, shadow capsules, and all 150 mode-bytecode entries have been
generated from the installed original files and checked. Supplying our
source-built reader with `--pak-tool PATH` enables the shadow and mode stages.
Full combat-matrix generation, local particle inputs, and the redistributable
native physics bridge remain unfinished. Accordingly, setup currently returns
runtime_ready:false; **this command does not yet produce a runnable game**.
See [local import status](docs/LOCAL_IMPORT.md) for dependencies and stage details.

Matching original EXE/PDB files are currently needed for the native import step.
They identify the supported layout, not account ownership. An EXE check on every
launch is not a design requirement. The current runtime still reads original paks
and PhysX libraries locally; a self-contained cache-only sandbox is future work.
Launch checks the required local cache, paks, and libraries. The EXE is checked
during import and is not required again on every launch.

## Thank you, Triternion

We thank **Triternion and the MORDHAU developers** for creating what we consider a
revolutionary, AAA-quality first-person melee combat system. Its precise,
expressive combat inspired this project.

**Buy and support the original game:** [MORDHAU on Steam](https://store.steampowered.com/app/629760/MORDHAU/)
and [the official website](https://mordhau.com/).
We do not endorse piracy. This is an unofficial fan project with no claim of
Triternion endorsement. Original game content is not licensed by this project.

## Development

See [framework APIs](docs/FRAMEWORK.md), [release acceptance checks](docs/RELEASE_BLOCKERS.md),
[distribution boundaries](docs/DISTRIBUTION.md), and [notices](NOTICE.md).
The combat baseline is 5e222ec3da7b828804f70d54d33c15a84c12075d.

```text
python scripts/audit_package.py
python -m unittest discover -s scripts -p "test_*.py"
```

The source audit rejects original binaries, assets, generated tables, secrets,
and unreviewed payloads. Passing it does not establish a working application.
Captures and imported records stay out of Git. The GitHub Actions audit is a
manual template at docs/templates/source-audit.yml; it is not enabled.
The repository becomes public only after the runnable distribution passes its
functional, provenance, and packaging checks.
