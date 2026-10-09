# Framework audio replacement

The private combat fork previously routed and decoded every sample from Mordhau. Prewarming that audio reduced cold reads but did not replace any recording. The new bank substitutes decoded PCM at the SoundCue leaf; native events, random-node selection, delays, volume/pitch nodes, concurrency, envelopes, attenuation and Doppler remain in use.

Install the source-only recipe with Python 3.11+ and 7-Zip on PATH:

```powershell
python scripts/install_framework_audio.py
```

The destination defaults to the framework's own `Saved/Config/WindowsClient/audio` directory, or `$env:MH_CONFIG_DIR/audio`. The runtime reads this bank automatically. `MH_AUDIO_REPLACEMENTS` can point to another bank. Recordings live outside Git. Archive SHA-256 digests, author, license, source page and selected member paths are recorded in `audio/sources.toml`; installed `bank.json` additionally records sample digests.

The first bank was rejected in listening tests. Its broad material mapping and uniform peak normalization did not preserve the originals' character or balance. In particular, sheath-squeeze and belt-buckle recordings replaced armor movement and release foley, producing an inappropriate loud windup noise. These 13 substitutions have been removed from the recipe. The entire unapproved bank has been withdrawn from both local game configurations, preserving its manifest and recordings for comparison. There are currently no automatic replacements; the owner's native samples play through the existing spatial mixer.

All cues currently use the owner's original samples, pending individually auditioned independent replacements. Do not add a windup effect. Replacement banks substitute only leaves of existing SoundCue events; they must preserve quiet/silent cues and distinguish material, duration and level rather than treating any leather/metal recording as equivalent. The loader's uniform peak normalization also needs review before new samples are activated. No time stretching or event retiming is applied.

The download recipe preserves 76 unapproved recordings in 16 `candidate_roles`. The default installer consumes only `roles`, currently empty, so rerunning setup cannot restore the rejected sounds and requires no downloads. To prepare a separate review bank, use `--candidate-bank build/audio-audition`; this does not activate it in the game. Test it explicitly with `MH_AUDIO_REPLACEMENTS` only during an audition. Archive popularity is not a substitute for listening against each native cue.

The scope is combat and the training range. Spoken voice commands, music, breathing/heartbeat loops, animals, siege engines and ambient map recordings still need appropriate replacements. Unmapped events retain the original sample; missing or invalid banks log an explicit error/warning. This preserves sound behavior while remaining honest about incomplete replacement coverage.

Evidence: every audio log row has `sample_source`, either `framework:<role>:<file>` or `original`. Script snapshots include counts across all started samples. A build is not accepted solely because downloaded files exist: validate substitution during actual attacks, parries, hits and death in the compiled executable.

## Sources

All selected downloads offer CC0. These facts were checked on the creators' pages on 2026-10-09; favorites indicate community use, not an objective quality guarantee.

* StarNinjas, [20 Sword Sound Effects](https://opengameart.org/content/20-sword-sound-effects-attacks-and-clashes): 25 favorites and positive comments; swings and clashes.
* Vehicle, [Fantasy Weapons and Apparel SFX Library](https://opengameart.org/content/fantasy-weapons-and-apparel-sfx-library): 16 favorites and positive comments; leather/equipment foley.
* Kenney, [Impact Sounds](https://kenney.nl/assets/impact-sounds): native creator download; material impacts and footsteps. No favorites/comments metric is provided on this page.
* HaelDB, [Male Grunt/Yelling sounds](https://opengameart.org/content/male-gruntyelling-sounds): 20 favorites and positive comments; combat voices. The offered CC0 option is selected.
* Iwan 'qubodup' Gabovitch, [Impact](https://opengameart.org/content/impact): 62 favorites and positive comments; flesh and stone impacts.

Credits are retained even where CC0 does not require attribution. These free recordings do not make the remaining private importer or playable release ready for publication.

## 2026-10-09 windup correction

Product question: does starting an attack introduce the rejected squeeze/buckle sound? Failure mode: an existing quiet armor cue was replaced with the wrong recording and boosted. Change: withdraw the rejected bank, preserving native events and original PCM. False if a new run plays any rejected framework sample or starts a weapon whoosh during windup.

Native evidence: `UAttackMotion::OnBegin_Implementation` RVA `0x162eda0` calls `PlayNonSnappyArmorFoley`; `OnTick_Implementation` RVA `0x16328c0` starts the weapon whoosh in release. The attack yell threshold is `WindupEnd + PlayAttackYellTimeReleaseOffset + max(-LagInduction, 0)`, with constructor offset `-0.05`; do not turn it into an attack-start sound. The loud squeeze/buckle recording was our substitution, not a native windup effect.

Run `scripts/verify_framework_windup_audio.py --data <owner-local-data>` to compare swing and stab scheduling with an original-only bank using the same canonical executable. Evidence is private under `build/windup-audio-proof`; the check validates routing and timing, not subjective acceptance of the remaining sound bank.

## Spatial playback check

The `verify_framework_spatial_audio.rs` probe links to the reviewed runtime's existing libraries and renders actual native weapon PCM through `channel_map` and `Voice::mix_stereo`. It checks left/right channel energy, a 180-degree listener rotation, distance attenuation and centered nonspatial playback; it writes private WAV evidence, never distributable game assets.

The first probe failed: exactly-left and exactly-right emitters both mapped to azimuth 90 and the right speaker. Native `FAudioDevice::GetAzimuth` RVA `0x2ef3db0` really has this equality branch (`0x2ef3fce` jumps past sign correction when the forward dot is zero). This is an intentional boundary correction for the rewrite's frequently axis-aligned camera, not a claim that the branch was misread. Preserve the side when the forward dot is exactly zero, making left azimuth 270 and right 90; retain the other native branches. A regression check also covers both sides of this boundary and a rotated camera.

Combat cues use world-space emitters: the whoosh sits along the weapon trace, impact sounds use the hit position, and character sounds use their speaker position. The listener follows camera position/orientation; native attenuation and the stereo channel map feed the same software mixer used by device playback. This check does not establish full vanilla mixer parity or subjective replacement quality.

Saved build and validation on 2026-10-09: compiled source `baf056332a5aa0c5c88feaf7049b08ba8e361d14`, runtime SHA-256 `7425379b5297c83419de28b9787eeafb09a5ba1340cf5014418bffcb6b488a24`, same `build/install-gate/debug/mordhau.exe` path. Three angle/panning regression tests passed. Actual native PCM renders left at azimuth 270, right at 90, reverses sides after camera rotation, and attenuates to silence at 100 m under the native whoosh settings. The full combat run observed an actual parry, hits and death: 39 original plays, zero framework substitutions, no missing audio cues. Separate swing/stab runs preserved original-only event timing and played whooshes after release. Default setup activated zero candidates and downloaded zero archives. Evidence stays under ignored `build/audio-proof-native-final`, `build/windup-audio-proof-final`, and `build/windup-audio-proof-native-restored/spatial-fixed`.

The playback verifier's `--source-receipt` checks the executable digest against its saved build revision; checkout HEAD is recorded separately because other project work may be present. The main rewrite executable was not changed. New independent recordings remain pending per-cue audition; withdrawing bad audio is not completion of the custom sound redesign.
