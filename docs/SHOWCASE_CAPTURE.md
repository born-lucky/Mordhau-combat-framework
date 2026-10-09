# Public showcase capture checks

Do not reuse a baseline recording to demonstrate a later presentation fix. A valid combat-event recording can still be visually obsolete. The original public combat clip was withdrawn because it preceded the articulated hands and corrected weapon frame.

Before publishing a replacement:

1. Match the captured executable hash to its saved-source build receipt. Confirm the requested fixes exist in that source revision.
2. Visually inspect the rendered hands and weapon during idle, attacks and parries. Source ancestry alone is insufficient.
3. Inspect the actual drawn-part diagnostics: finger geometry, native-Y guard alignment and the blade width frame. Verify normal and alternate grips.
4. Require actual parry, damage and death events for clips described as demonstrating those behaviors. Keep the complete attack phases and avoid speed changes that misrepresent the mechanics.
5. Decode the final video, upload it, then inspect the published README video players and public attachment access. Update the footage-hosting issue as well as the README so its current body does not advertise the retired clip.

## Replacement recorded on 2026-10-09

Source revision: `7aa02be2d1ad7d617e8dc214658d4c136b72a583`.
Executable SHA-256: `f79e3890f4f1ff34d72a12a8f55d2a3ddc863476d57d5ee5d6d5f74fcb29962c`.
Combat video SHA-256: `fe970ac5caf6150349aa24658d602151ebf6192986fbe3062aa100bf29e31fc7`.

The capture shows two palms, 30 finger joints, 20 finger links and 10 fingertips. Across reviewed scene snapshots the guard alignment exceeds 0.9999998 and the blade width matches the converted native Y direction. Both normal and alternate grips were checked. The scene records include an actual parry, damage and death; the rendered hand and weapon frames were visually reviewed. The complete H.264 video decodes successfully: 498 frames, 1280 by 720, 16.6 seconds at 30 FPS.

Local capture recipes, screenshots, state dumps and build receipts remain under the ignored `build/showcase` directory. Public videos are hosted as GitHub attachments, outside the source tree.
