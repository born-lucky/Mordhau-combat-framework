# Isolated combat framework presentation

The public-facing fork owns its interface and presentation. The main rewrite remains separate.

- The runtime and launcher default to `TestLevel`. `--menu` opens the authored Steel Lab front end with the test world loaded, rather than opening the original game front end.
- Escape, Enter and the authored resume/perspective/tools/exit buttons belong to the framework. Hidden original UI roots never draw and their VM receives neither native nor synthetic UI input in this host. Gameplay HUD events remain available.
- Health and stamina use the controlled fighter's live values and bounds. Preset strike/stab bindings are included in the controls legend.
- Palms, finger links, joint spheres and fingertip pads are authored procedural geometry driven by the existing hand skeleton. Each character currently has two palms, 30 finger joints, 20 links and ten terminal pads. There is no separate animation clock. This is a training figure, not a finished anatomical character mesh.
- The training sword's handle is centred on the active mode's `GripLocationLocal` in the drawn held component. The collision trace start is no longer treated as the handle socket. Gameplay hitboxes, trace endpoints, animation timing and attack selection are unchanged.
- The default writable settings directory is `%LOCALAPPDATA%/MordhauCombatFramework/Saved/Config/WindowsClient`, independent of the main rewrite. On first use, existing original-game `Input.ini` bindings are copied into an absent framework file. Existing framework preferences and explicit input/config overrides are preserved. Original settings are never written.

## Parry-side investigation

The isolated fork previously launched with a fresh config directory and read pak defaults rather than the user's saved bindings/sensitivity. The new local import removes that mismatch. This is not proof that it explains every reported difference in parry feel.

Native `AMordhauPlayerController::GetAnglingVector` (RVA `0x15d54a0`) and `UMotionSystemComponent::RequestParry` (`0x14d1590`, machine-code branch `0x1414d1685..0x1414d16b1`) select the left block from negative angling X. `FlushPendingAnglingInputs` (`0x15d4880`) replaces X on sign reversal. No new movement-distance threshold or sensitivity multiplier has been added. Action-before-axis ordering is retained. A regression test checks a one-count reversal after a large opposite movement at both saved and pak sensitivities, including X inversion. A separate buffered-parry defect was then reproduced: re-pressing left during an active parry set the buffer, but its successful retry always became right. The owning player's freshly flushed angling X now travels through the runtime and sim into that retry, so it reselects the current side. The initial press retains its original pre-axis ordering. AI/script-only fighters have no player angling sample and retain the regular retry behavior. No timing or sensitivity threshold changes.

The core integration test `buffered_player_parry_reselects_current_mouse_side_after_cooldown` failed before the retry change (expected left, got right), then passed. It exercises an actual parry cooldown and buffered retry for left remaining left, switching to right, switching to left, and no player controller. It requires a locally generated records JSON via `MORDHAU_PARRY_TEST_SPEC`; no fixture is published. Original-game interactive side-by-side feel remains to be judged by the user.

## Local verification, 2026-10-09

The internal developer-cache render exited successfully and was visually inspected: normal first-person grip, stab windup, alternate grip, third person, hand close-up and the new menu. Captures reported zero handle-position error against the native grip point for both fighters in the three sampled poses; this does not establish every weapon/attack or complete animation parity. Escape opened the framework menu, Enter closed it, and the original VM menu stayed closed. The imported input file matched the user's original file byte for byte.

Private ignored evidence: `build/isolation-pass/`, including render receipt, pose JSON, PNGs, presentation checks and unit-test results. Three framework input/menu tests, 17 input tests and two visual-frame tests passed. A final controls-legend-only change includes preset attacks instead of misleadingly showing Strike as unbound.

The evidence uses the labelled internal developer cache and existing private physics bridge. Clean consumer import, replacement bridge runtime acceptance, hit-effects/gore validation and public release are still governed by `RELEASE_BLOCKERS.md`. No original assets or generated cache files are added to Git by this change.

The final compiled runtime also passed a real Bevy mouse-input capture: an initial right parry was followed by one raw left count and a buffered re-press. The current angling X was negative, the buffer waited during the existing cooldown, and the next parry began as left with `wants_block` cleared. Opening the framework menu did not open the hidden original menu. Final private evidence is in `build/isolation-pass/parry-runtime/acceptance.json`.
