# Private staging validation

Only the original-install gate, launcher argument handling and source packaging
were tested for this staging snapshot. The full Bevy runtime was not rebuilt or
launched, and this is not a clean-machine playable-release acceptance.

- Seven Rust install-gate tests passed: missing install, unsupported EXE hash,
  malformed/wrong-architecture PE headers, missing pak/DLL, wrong build marker,
  incomplete/malformed local records and path traversal rejection.
- One launcher argument test passed.
- The standalone installation verifier passed against the supported original
  installation in place: EXE hash/headers, build marker, pak and five original DLLs.
- Runtime preflight with an absent external local cache failed with exit code 2.
  No game window or game process was launched by validation.
- Seven Python source-audit tests passed. The actual source tree audit passed.
- The explicit public-release audit failed as intended because required release
  blockers remain open.
- Changed Rust source parsed successfully using rustfmt from toolchain 1.95.0.

Focused Cargo tests used the main project's existing serialized build wrapper,
one compiler job and an isolated v1 build directory. The development runtime,
launcher, native bridge and main private repository were left unchanged.

Binary-embedding review of the old development runtime found the full extracted
shadow-capsule table and two shader payloads. That runtime and native bridge were
excluded from this repository. The v1 source reads those inputs from a local cache.
An independent reviewer must audit any future rebuilt runtime before release.

No successful complete consumer importer, full-runtime build, clean-machine
launch, ownership verification or public-distribution rights clearance is claimed.
