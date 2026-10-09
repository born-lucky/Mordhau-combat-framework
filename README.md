# Mordhau Combat Framework

**Requires a legitimately purchased MORDHAU installation, including its original game EXE and content files. Game assets are not included.**

An experimental melee combat framework in **Rust and Bevy**, inspired by MORDHAU. It brings attack phases, weapon poses, parries, collision, stamina, damage and ragdolls into a reusable simulation with a custom training scene and presentation.

Huge respect to **Triternion** for creating what we consider a revolutionary first-person melee combat system. MORDHAU's expressive attacks, precise defense and depth are the reason this project exists. Please **[buy MORDHAU on Steam](https://store.steampowered.com/app/629760/MORDHAU/)**, play the original game and support its developers. Visit [the official MORDHAU website](https://mordhau.com/).

This is an unofficial fan project. We do not endorse piracy or claim Triternion's endorsement.

## See it working

Gameplay videos are being prepared from the verified development build.

## Combat sandbox

- First- and third-person combat driven by the evaluated character pose.
- Swings, stabs, parries, alternate grips, damage and stamina.
- Weapon collision, tracer debugging and environmental hit-stop.
- Custom training figures, weapons, health and stamina display.
- Physical death ragdolls and hit effects.
- Per-weapon timing values and curves for experimentation.

Development is ongoing; complete original-game parity is not claimed.

## Setup and development

This repository is a **source preview**. The complete consumer setup-to-play path is still in development; there is no playable download yet. The footage uses the developer's locally prepared game data.

Like [ELDEN-RING-Combat-Rewrite](https://github.com/Funny-Bones/ELDEN-RING-Combat-Rewrite), original content is obtained from the user's own installation rather than shipped with the source. The current setup diagnostic is:

```text
python tools/setup.py --game-dir "YOUR-MORDHAU-INSTALL" --cache-dir "YOUR-LOCAL-CACHE" --json
```

The supported Windows game EXE is `Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe`. Native import currently also needs its matching PDB. The importer remains incomplete and does not yet prepare a runnable installation; see [local setup status](docs/LOCAL_IMPORT.md).

Explore the [framework API](docs/FRAMEWORK.md), [custom presentation](docs/PRESENTATION.md) and [release checklist](docs/RELEASE_BLOCKERS.md). Original game content remains subject to its owners' rights; see [notices](NOTICE.md).
