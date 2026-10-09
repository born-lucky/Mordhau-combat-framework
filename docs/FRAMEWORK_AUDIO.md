# Framework audio replacement

The private combat fork previously routed and decoded every sample from Mordhau. Prewarming that audio reduced cold reads but did not replace any recording. The new bank substitutes decoded PCM at the SoundCue leaf; native events, random-node selection, delays, volume/pitch nodes, concurrency, envelopes, attenuation and Doppler remain in use.

Install the source-only recipe with Python 3.11+ and 7-Zip on PATH:

```powershell
python scripts/install_framework_audio.py
```

The destination defaults to the framework's own `Saved/Config/WindowsClient/audio` directory, or `$env:MH_CONFIG_DIR/audio`. The runtime reads this bank automatically. `MH_AUDIO_REPLACEMENTS` can point to another bank. Recordings live outside Git. Archive SHA-256 digests, author, license, source page and selected member paths are recorded in `audio/sources.toml`; installed `bank.json` additionally records sample digests.

The first bank contains 89 recordings in 17 roles: swings, parries, flesh hits, metal/wood/stone/soft impacts, equipment and armor foley, four footstep materials, interface hits, and combat effort/pain/death voices. Samples are predecoded before play, converted to mono for point emitters, given consistent peak headroom, and trimmed of surrounding recording silence with 5 ms padding. No time stretching or event retiming is applied.

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
