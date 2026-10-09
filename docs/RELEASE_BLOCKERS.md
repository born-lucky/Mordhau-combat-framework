# Public release blockers

Release state: BLOCKED. The repository is private. There is no public game release.

1. Implement and verify a complete local importer from the supported original EXE
   and paks. Reconstruct native constructor defaults, Blueprint overrides, matrix
   records, native ancestry, required constants and shadow capsules
   without distributing private generated data or requiring unpublished PDB/decomp files.
2. Build from the actual reviewed BSD PhysX source revision with the required
   original-library ABI and solver compatibility changes, then revalidate physics.
   Do not ship the old 2016 extension binary or relabel its license.
3. Complete source/dependency provenance and binary packaging review. The authored
   rewrite WGSL files have been reviewed as code, not copied shader payloads.
   Particle shaders translated from original cooked materials are a separate local
   import dependency. Implement and verify their generation in the user's cache;
   the developer's translated particle cache must never be bundled as source.
4. Verify startup from a clean machine with only a purchased supported installation.
   The setup step must reject unsupported original inputs; launch must reject
   missing/corrupt required cache and runtime inputs. An EXE check on every launch
   is not required by the intended reference-style contract. Never fall back to stub combat.
5. Build an isolated runtime without embedded original game records and audit
   the final binary/ZIP for original bytes, extracted data, SDK sources, secrets and
   private paths. A source-only tree audit cannot certify a future executable.
6. Independently judge combat behavior against the pinned source baseline and
   declare known parity gaps. Do not release the developer's embedded-data binary.
7. Compile and render the new custom character/weapon presentation and HUD;
   verify live health/stamina changes, alternate grips, respawns, and ragdolls.
   Recheck corrected parry sparks and explicitly distinguish blood effects from
   unimplemented dismemberment/gore behavior.

These are specific findings from the actual combat build, not a general assertion
that all asset-free fan projects are prohibited.
