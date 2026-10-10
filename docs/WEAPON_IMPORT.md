# Weapon data import

The weapon importer reads the supported original MORDHAU installation in place.
An existing installation is sufficient; no reinstall is required. Setup can
detect a running original Shipping EXE or accept its path explicitly. The EXE,
matching PDB and installed paks are required. Original files are never modified.

Build the two helpers from this repository, using the project's serialized Cargo
workflow where available:

```text
cargo build --manifest-path core/Cargo.toml -p mh-pak --bin mh-weapon-packages -j1
cargo build --manifest-path core/Cargo.toml -p mh-sim --bin mh-verify-weapon-import -j1
python tools/setup.py --weapons-only --game-exe "YOUR-INSTALL/Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe" --cache-dir "YOUR-LOCAL-CACHE" --weapon-tool core/target/debug/mh-weapon-packages.exe --weapon-verify-tool core/target/debug/mh-verify-weapon-import.exe --json
```

Python needs `openpyxl` for spreadsheet validation. Helper paths above assume
Cargo's default target directory; use the actual paths for a custom target.
Full setup discovers these helpers in `tools/bin/`, or beside an explicitly
supplied `--pak-tool`. `--check` verifies inputs without generating data.

The import has four checked steps:

1. Recover seven native constructor classes at original PDB addresses. Decode
   scalar writes, enum values, null references and supported containers. Unknown
   writes stay unknown. Original allocation instructions establish zero memory;
   zero is never substituted for an unsupported write.
2. Read weapon Blueprint CDO chains and profile maps directly from original paks.
   Merge ancestors before children, including individual inherited attack-map
   keys. Validate package entry hashes and retain original export identities.
3. Combine only accepted native fields with serialized overrides. Authored
   bindings define names and types, without supplying gameplay numbers.
   Produce a validated spreadsheet, SQLite view and `mh-spec` weapon matrix.
4. Load the matrix through Rust and deserialize every attack, `WeaponData` and
   `EquipmentDef` consumer. Reject missing keys before serde defaults apply.
   Compare imported values, profile maps and parry references; check every move
   in both grips and the complete mode-switch pairs.

Accepted local outputs are `data_gen/weapons/records.json`,
`data_gen/weapons/spec/`, and constructor values plus their field review. The
spreadsheet, database, process logs and receipts stay under the cache's `stages/`.
All outputs remain local and must not be committed or redistributed.
Existing divergent outputs are preserved and rejected; use a separate cache
generation when changing the supported source or importer.

Original-input acceptance covered **279 weapons, 2,523 attacks, 75 alternate-mode
weapons and 4,464 move/grip lookups**. A separate comparison after generation found
115,783 numeric leaves equal by binary32 bits and 1,116 profile or parry
comparisons equal. That prepared reference was never a generation input.
Eight abstract weapons have opaque native catalog names without serialized
labels; the importer reports these instead of inventing names. Blueprint-only
optional fields remain absent when the original Blueprint does not declare them.

`ALT_STAB` selects the left stab side. Alternate grip uses `SecondStabAttack`
and the second animation profile. These are separate choices.

`--weapons-only` returns success when this stage is accepted and reports
`weapon_data_ready:true`. It still reports `runtime_ready:false`: the standalone
weapon matrix is not the full character/stat/motion matrix and must not be used
as `MORDHAU_SPEC_DIR`. Full setup still needs the remaining matrix, particle and
physics dependencies. This import does not replace custom models, weapon meshes,
UI or sounds and does not alter the currently playable development build.

The previous approach accepted a successful reader process while the reader
filled absent native fields from authored defaults. The replacement isolates
original constructor extraction, Blueprint inheritance, matrix generation and
Rust consumption. Each stage has rejection conditions and preserved evidence.
Constructor replay alone does not establish gameplay parity or a complete
decompilation.
