# Public release blockers

Release state: BLOCKED. The repository is private. There is no public game release.

1. Implement and verify a complete local importer from the supported original EXE
   and paks. Reconstruct native constructor defaults, Blueprint overrides, matrix
   records, native ancestry, required constants, shadow capsules and shader inputs
   without distributing private generated data or requiring unpublished PDB/decomp files.
2. Establish redistribution rights for the exact native physics extension source,
   or replace it with independently written compatible code and revalidate physics.
3. Review rights and provenance of all reverse-engineered source/shader algorithms
   and dependencies before making source or binaries public. Developer credit and
   an EXE requirement do not supply distribution permission.
4. Verify startup from a clean machine with only a purchased supported installation.
   Both direct runtime and launcher must reject absent/changed original files and
   missing/corrupt local import data. Never fall back to stub combat.
5. Build an isolated runtime without embedded original game records and audit
   the final binary/ZIP for original bytes, extracted data, SDK sources, secrets and
   private paths. A source-only tree audit cannot certify a future executable.
6. Independently judge combat behavior against the pinned source baseline and
   declare known parity gaps. Do not release the developer's embedded-data binary.

These are specific findings from the actual combat build, not a general assertion
that all asset-free fan projects are prohibited.
