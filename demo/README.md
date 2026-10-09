# Captioned combat capture

This Windows verification harness records an explicitly supplied Rust + Bevy combat runtime. It does not build or demonstrate the setup UI. Ordinary scripted game inputs produce actual offscreen frames; captions are added below the unchanged gameplay image. No music, interpolated poses, accelerated playback, Steam recording, or synthetic damage is used.

All runtime binaries, original content, bridge DLLs, configuration and output stay outside the source package. Python 3 and FFmpeg/FFprobe must be installed locally. Source-only checks use Python’s standard library.

Provide `--runtime`, `--data-root`, `--bridge` and `--version-manifest` explicitly. There are no private directory defaults. The manifest is a local build/provenance record with this schema (replace each placeholder with the complete real value):

```json
{
  "schema_version": 1,
  "source_commit": "<40 hexadecimal characters from the actual source build>",
  "runtime_sha256": "<64 hexadecimal characters from the actual executable>",
  "bridge_sha256": "<64 hexadecimal characters from the actual bridge>",
  "scope": "Versioned combat runtime; setup UI not represented"
}
```

```powershell
python demo/showcase.py preflight --runtime C:/local/runtime.exe --data-root C:/local/data --bridge C:/local/data/state/physics/mh_physx.dll --version-manifest C:/local/build-version.json
python demo/showcase.py prepare --runtime C:/local/runtime.exe --data-root C:/local/data --bridge C:/local/data/state/physics/mh_physx.dll --version-manifest C:/local/build-version.json
python demo/showcase.py record --plan build/showcase/<timestamp>/plan.json --case combat-baseline
python demo/showcase.py record --plan build/showcase/<timestamp>/plan.json --case melee-contact
python demo/edit_showcase.py verify --plan build/showcase/<timestamp>/plan.json
python demo/edit_showcase.py encode --plan build/showcase/<timestamp>/plan.json
python demo/test_showcase.py
```

Preflight validates paths and file hashes without starting a game or loading a DLL. It checks identities declared by the supplied build manifest; it does not prove how that build was produced. This runtime loads its bridge from `data-root/state/physics/mh_physx.dll`, so the explicit bridge argument must identify that exact file. Required extracted/imported content must already exist in the supplied data root. Native game DLL discovery follows the runtime’s installation configuration, including `MORDHAU_DIR` when supplied; the recorder logs that value. It does not install or import content.

An environment collision shot is optional: add `--wall-oracle C:/local/reviewed-wall.txt` to `prepare` only when a locally reviewed map placement recipe exists. The recipe must contain the `dump_state wall_strike_before` marker. Its placement stays in ignored generated scripts; no automatic private oracle is used. Then record `--case wall`.

Each launch requires at least 5 GiB free Windows commit and verifies executable/native bridge file hashes before starting one isolated offscreen process. The recorder clears inherited `MH_*` camera/physics overrides, supplies isolated configuration and stock timing files, and observes its own process’s loaded physics DLLs when Windows permits it. Configuration, stock overrides, screenshots, receipts, and exported JSON live under ignored `build/showcase/`. Existing executable files and user game processes are untouched. Offscreen audio is disabled.

The recorder captures 120 Hz simulation every four frames, yielding actual 30 fps playback. The verifier checks process file pins, simulation cadence, attack/grip state, complete recovery, trace visibility and actual scene-bounded contacts. A previous hit cannot prove a later scene. A physical corpse requires real native bodies and blend weight with no diagnostic errors. Numeric checks remain separate from visual review: inspect the images and final movie before publishing a feature claim. Future captions identify the supplied source revision; the existing baseline demo remains its original version `5e222ec`.

Failed attempts and their original source snapshots remain in ignored output directories. Retry plans must name fresh case directories. Original legacy capture plans remain evidence and may be inspected/encoded, but new recording requires the explicit portable prepare schema. All images and movies remain ignored by Git. Publication is limited to an individually reviewed rendered video attachment; this harness does not publish or bundle game files.

Original MORDHAU content shown by the runtime belongs to Triternion. Demo footage does not grant permission to redistribute source assets.
