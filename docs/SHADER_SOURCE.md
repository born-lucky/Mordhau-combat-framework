# Shader source and game data

Two reviewed files are authored WGSL programs in this rewrite:

- core/crates/mh-assets/shaders/ue_tint.wgsl
- core/crates/mh-runtime/src/uepost.wgsl

They implement shading algorithms studied from the original cooked shaders, with
the same arithmetic also represented in the Rust CPU reference. Their comments
retain that reverse-engineering provenance. They are code, not original DXBC/HLSL
listings, shader-cache files, images, LUT payloads or cooked material-instance data.
They can be included as source with an exact-file audit exception.

This corrects the initial staging audit's overly broad classification of every
shader port as extracted asset data. It does not assert that changing programming
language automatically grants third-party rights or authorize original caches.

The original shader caches, decompiler dumps and material/texture records remain
excluded. The shadow-capsule JSON contains actual original body-asset geometry and
must still be read from installed packages and generated into the user's cache.

