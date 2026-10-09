# Distribution boundary

This fresh repository is a source snapshot, not a GitHub fork carrying the private
main repository's history. Only tracked Rust workspace source selected from the
pinned combat commit was copied. `release-base.json` records upstream source hashes
and exclusions; later fork modifications have their own Git history.

The initial selection excluded all WGSL files, extracted shadow capsule JSON,
placed-level TSV, and three scenario JSON fixtures. It included no ignored files,
symlinks/junctions, original game data, compiled binary, SDK, extraction directory,
private decomp output, state/checkpoints, recordings, screenshots or spreadsheets.

The two authored rewrite WGSL programs were subsequently reviewed and restored
as source code; see SHADER_SOURCE.md. Raw original shader bytes remain excluded.
The importer also packages an explicitly reviewed source-only GDScript reader
closure, with no cached records or private decompilation output.

Original game files remain in the user's installation. Locally generated records
remain in an external per-user cache. Neither location belongs in Git, release
archives, CI artifacts, crash attachments, or issue submissions.

A future release archive must be made from an explicit manifest of reviewed source
or independently approved binaries. Do not zip a developer working directory.
Run the tree audit before every commit and packaging step. The public-release gate
for playable binaries must remain closed while any blocker is unresolved. The public source preview and developer demonstration footage do not constitute a playable release; no automatic release job exists.

File checks do not prove ownership. Do not add fake ownership claims, Steam API
impersonation, game DLL replacement, downloaders for game files, or DRM bypasses.
