# Kick rebuild — 2026-10-09

The acceptance bar is a visible first-person foot, a real F-key kick against a living fighter, and damage attributed to the kick weapon. Starting BP_KickMotion alone is not sufficient.

## Findings and changes

1. UKickMotion::OverrideTrace (RVA 0x166a3c0) reads KickTracerStart/KickTracerEnd on the character mesh. OverrideAdditionalTrace (0x166a250) reads AdditionalKickTracerStart/AdditionalKickTracerEnd. OverrideIsUsingAdditionalTracers (0x166a3b0) enables that second sweep. The rewrite previously prepared the held weapon's sockets instead. Load the original skeleton/mesh socket records and evaluate both sweep pairs on the animated character bones.
2. PrepareForTracing (0x16378e0) maintains previous/current sweep positions; SampleTracers (0x163c430) samples the primary then additional sweep. The additional sweep has no hand/environment-only prefix. Keep both sweeps in the shared body/shield/clash and world-contact pipeline.
3. AB_MordhauCharacterAnimation FullBodySlot (graph node 545) precedes first-person TwoBoneIK nodes 465/464. Their targets are the Position-to-foot virtual bones, which are also affected by a full-body montage. The rewrite retained the lower-body idle target outside that slot and then pulled the kicking foot back toward it. Carry the virtual targets through the same montage weights, then apply the existing first-person IK.
4. Preserve the original animation assets, F binding, camera and shared evaluated pose. Record right leg/foot positions in the existing camera probe so screenshots can be checked against the pose.

## Verification

`scripts/verify_framework_kick.py` exercises the actual F key, captures windup/release/recovery in first or third person, and requires a kick-weapon hit and reduced target health. Its screenshots still need visual review; numeric contact alone does not certify visible first-person animation.

Build from a saved Git archive into the existing development executable location. Keep the executable hash and source commit in the build receipt. Do not report this fixed until runtime contact and the first-person screenshots both pass.

## Saved-build result

Runtime source revision: `7aa02be2d1ad7d617e8dc214658d4c136b72a583`.
Runtime SHA-256: `f79e3890f4f1ff34d72a12a8f55d2a3ddc863476d57d5ee5d6d5f74fcb29962c`.

- Four real simulation collision tests passed: animated character socket lookup, additional body sweep, previous-sample invalidation, and existing first-world-contact/hand-filter behavior.
- The native F-key contact test reduced the living target's health from 100 to 88, with a BP_KickWeapon hit on LeftUpLeg. This checks the actual damage/armor route; it does not replace the damage value with a fixture constant.
- The out-of-range first-person test caused no damage. Its release screenshot visibly shows the extended right leg and foot. At montage position 0.9454, the camera-relative RightFoot is approximately (forward 102.53, right 6.81, up -45.39) cm, instead of being held at the standing IK target.
- The close contact stops the montage before full extension. Keep this separate from the unobstructed animation check rather than claiming one image proves both.

Evidence remains in the developer's ignored `build/kick-proof-1p-retry` and `build/kick-proof-1p-miss` directories. Windows allocation failures interrupted separate startup/capture attempts; those failed runs are not acceptance evidence. Broader weapon, jump-kick, combo/riposte and timing parity remain outside this check.
