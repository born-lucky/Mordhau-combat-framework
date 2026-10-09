# Mordhau Rewrite in Rust v1 — combat build

A separate development snapshot of the experimental Rust and Bevy combat rewrite.
**Private staging only: there is no public playable release or downloadable game executable yet.**
The combat source baseline is `5e222ec3da7b828804f70d54d33c15a84c12075d`.
This repository has a fresh history and is independent of the main private development repository.
The main installed combat build is not modified by this distribution work.

## Thank you, Triternion

We thank **Triternion and the MORDHAU developers** for creating what we consider a
revolutionary, AAA-quality first-person melee combat system. Their work on precise,
expressive combat is the reason this project exists.

**Buy and support the original game:** [MORDHAU on Steam](https://store.steampowered.com/app/629760/MORDHAU/)
and [the official MORDHAU website](https://mordhau.com/).
We do not endorse game piracy. Use a legitimately purchased installation.
This is an unofficial fan project; Triternion has not endorsed or authorized it.
MORDHAU and its original game content belong to their respective rights holders.

## Original installation required

The v1 launcher and direct Rust runtime validate the supported original Windows
installation before game startup. Set `MORDHAU_DIR` to the original Steam MORDHAU
folder, not the rewrite folder or the EXE itself.
The check requires the original `Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe`,
64-bit AMD64 PE format, its supported SHA-1, installed paks, and the original PhysX libraries.
Supported original build: `702625635`. Other builds fail closed pending compatibility work.

An EXE is not an asset archive. Meshes, animations, textures, audio, levels, and
Blueprint data are read from the original installation's `.pak` files.
This repository supplies no original EXEs, DLLs, paks, extracted assets, decompiled
output, generated combat records, spreadsheets, shadow-capsule tables, or original
shader payloads. Local imported data must stay outside the repository in a
user cache set by `MORDHAU_LOCAL_DATA`.

Installation checks establish compatible local files, **not proof of purchase or
Steam account ownership**. They are not DRM and do not guarantee legal permission
to distribute a rewrite. This project does not bypass the original game's DRM,
license checks, or anti-cheat, and does not replace the original game for online play.

## Why there is no playable download yet

The development runtime currently depends on 38 generated specification files,
native class ancestry, and executable constants. Its old generation chain needs
private local Ghidra/PDB and extraction output. A consumer importer that reconstructs
these solely from the user's supported EXE and installed paks is **not implemented**.
Locally reconstructed shader and capsule inputs are also required. Missing inputs
must stop startup; stub combat is not an acceptable substitute for this build.

The original development executable embeds extracted capsule geometry and shader
ports. It is deliberately not included. The v1 source reads those inputs locally
instead. The existing native physics bridge contains third-party extension code
whose exact redistribution license must be established or replaced before shipping.

See [release blockers](docs/RELEASE_BLOCKERS.md), [distribution rules](docs/DISTRIBUTION.md),
and [rights and third-party notices](NOTICE.md). `scripts/audit_package.py` checks
this source-only snapshot; passing it is not the same as a runnable release.

## Current combat scope

This snapshot preserves the combat source for attack poses, original-data-driven
weapon traces, parry collision, alternate grips, tunable phase timings, environment
hit-stop, effects, and ragdoll integration. Work toward original-game parity is
ongoing. Do not interpret v1 as a claim of complete 1:1 behavior across all weapons,
attack phases, maps, multiplayer, or effects.

## Development validation

The isolated install-gate tests do not require shipping or running the original game.
Run the package audit and its tests with Python 3.11 or later:

```text
python scripts/audit_package.py
python -m unittest discover -s scripts -p "test_*.py"
```

This snapshot includes no end-user build wrapper, redistributable native bridge,
generated data, or complete import/build recipe. Focused installation-gate tests
were run through the main project's internal serialized wrapper with a separate
build directory; that wrapper is not provided here. Do not copy private development output here
to make a release appear complete.

The pinned GitHub Actions audit workflow is saved as a manual template at
`docs/templates/source-audit.yml`. It is not enabled: the current GitHub login
lacks the `workflow` permission. Run the local audits before committing; a later
maintainer with that permission can install the template under `.github/workflows/`.
