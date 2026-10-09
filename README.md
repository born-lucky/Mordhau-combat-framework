# Mordhau Combat Framework

**Requires a legitimately purchased MORDHAU installation, including its original game EXE and content files. Game assets are not included.**

An experimental melee combat framework in **Rust and Bevy**, inspired by MORDHAU. It brings attack phases, weapon poses, parries, collision, stamina, damage and ragdolls into a reusable simulation with a custom training scene and presentation.

## Immense respect to Triternion

**Triternion created what we consider a revolutionary first-person melee combat system.** MORDHAU makes sword fighting expressive, demanding and deeply rewarding. The connection between camera control, animation, weapon movement and contact is an extraordinary piece of combat design, and it is the reason this framework exists.

The first-person camera is part of how you fight. Looking and turning shape an attack's path relative to your opponent. Real-time swing manipulation gives each attack room for skill: **accels** bring the weapon into contact earlier, while **drags** delay contact and challenge the defender's timing. Footwork, distance and the angle of approach all matter alongside that camera control.

The animations carry the weapon through space, and **weapon traces follow that animated movement to detect contact**. Debug tracers make those paths visible, showing how the pose, blade position and your manipulation shape an attack. That connection makes the combat fascinating to study: an attack has a moving path that players can influence and opponents must read.

Feints, morphs, chambers, parries and stamina add further decisions to that exchange. Together, these systems give MORDHAU a remarkable depth and a distinctive feel. **The credit for that original combat design belongs to Triternion.** This project is a fan's effort to study and learn from their work.

**Please [buy MORDHAU on Steam](https://store.steampowered.com/app/629760/MORDHAU/), play the original game and support Triternion.** Visit [the official MORDHAU website](https://mordhau.com/).

This is an unofficial fan project. We do not endorse piracy or claim Triternion's endorsement.

## See it working

**Kick — first-person animation and third-person contact**

https://github.com/user-attachments/assets/ef919f7f-22ac-48d4-8a0c-fbd5b017b278

**Combat — parries, weapon hits, alternate grips and death ragdolls**

https://github.com/user-attachments/assets/a2eb7c4a-a8f8-44ac-aaed-628f293ad65d

Silent gameplay captures from development builds, using custom training figures.

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

Original content is obtained from your own MORDHAU installation rather than shipped with the source. The current setup diagnostic is:

```text
python tools/setup.py --game-dir "YOUR-MORDHAU-INSTALL" --cache-dir "YOUR-LOCAL-CACHE" --json
```

The supported Windows game EXE is `Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe`. Native import currently also needs its matching PDB. The importer remains incomplete and does not yet prepare a runnable installation; see [local setup status](docs/LOCAL_IMPORT.md).

Explore the [framework API](docs/FRAMEWORK.md), [custom presentation](docs/PRESENTATION.md) and [release checklist](docs/RELEASE_BLOCKERS.md). Original game content remains subject to its owners' rights; see [notices](NOTICE.md).
