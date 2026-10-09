# Rights and notices

This repository is a private development snapshot. No broad open-source license
is granted here while ownership and third-party redistribution review remain open.
Source and dependency provenance review is recorded separately from original-game
assets. The two rewrite WGSL programs are authored source; their review is described
in docs/SHADER_SOURCE.md. Original shader payloads are excluded.

MORDHAU is developed and published by Triternion. Original game code, content,
trademarks, executable files, libraries and extracted or generated records remain
subject to their owners' rights. They are not licensed by this project.

The native bridge references PhysX and normally links to libraries in the user's
installation. The development build additionally compiles NVIDIA PhysX 3.4 extension
sources from a specific 2016 revision. Those files carry separate-license notices;
a later repository's BSD license is not assumed to resolve that exact revision.
No bridge binary, NVIDIA SDK source/header archive, or original PhysX DLL ships here.

Rust dependencies are resolved from crates.io under their respective upstream
licenses. They are not vendored in this snapshot. A future binary release must
include the applicable dependency notices and review embedded Bevy fonts/LUTs and
other upstream runtime resources separately from original game content.

Purchase and support MORDHAU: https://store.steampowered.com/app/629760/MORDHAU/
This project does not endorse piracy or claim official endorsement.
The Steam store's linked EULA is not treated as permission for this rewrite:
https://store.steampowered.com//eula/629760_eula_0
