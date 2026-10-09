//! UE units -> Y-up metres. The decoders return UE data as stored: centimetres, Z up, left-handed (X forward,
//! Y right), quaternions (x, y, z, w). The extract/gltf exports (CUE4Parse's glTF writer) and the Godot port convert
//! with a Y/Z swap and a 0.01 scale (CUE4Parse-Conversion/Writers/Gltf/Gltf.cs:25 UnitScale, :72, :129-131 joints,
//! :244-250 SwapYZ; ue_static_mesh.gd to_godot, ue_skeletal_mesh.gd bone_rest, ue_anim_sequence.gd godot_rot/pos).
//! The swap also mirrors handedness, so UE's left-handed space becomes a right-handed Y-up one (glTF / Bevy / Godot),
//! and the stored triangle corner order (clockwise front faces in UE's left-handed space) becomes counter-clockwise:
//! the glTF exports keep UE's order (i0, i1, i2) as is (ue_static_mesh.gd array_mesh), and so can a Bevy mesh. Only a
//! clockwise-front consumer (Godot's ArrayMesh) needs `flip_winding`.

/// cm -> m (Gltf.cs:25 UnitScale)
pub const UNIT_SCALE: f32 = 0.01;

/// UE position (cm, Z up) -> Y-up metres: (X, Z, Y) * 0.01
pub fn pos(v: [f32; 3]) -> [f32; 3] {
    [v[0] * UNIT_SCALE, v[2] * UNIT_SCALE, v[1] * UNIT_SCALE]
}

/// UE direction (normal, tangent) -> Y-up, normalized: (X, Z, Y)
pub fn dir(v: [f32; 3]) -> [f32; 3] {
    let o = [v[0], v[2], v[1]];
    let l = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    if l > 0.0 { [o[0] / l, o[1] / l, o[2] / l] } else { o }
}

/// UE scale -> Y-up: (X, Z, Y)
pub fn scale(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], v[1]]
}

/// UE quaternion (x, y, z, w) -> Y-up (x, z, y, -w), normalized (Gltf.cs:129-131 SwapYZ of a rotation)
pub fn quat(q: [f32; 4]) -> [f32; 4] {
    let o = [q[0], q[2], q[1], -q[3]];
    let l = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2] + o[3] * o[3]).sqrt();
    if l > 0.0 { [o[0] / l, o[1] / l, o[2] / l, o[3] / l] } else { [0.0, 0.0, 0.0, 1.0] }
}

/// Triangle list (i0, i1, i2) -> (i0, i2, i1), for a clockwise-front consumer (Godot's importer does this to glTF
/// triangles; ue_static_mesh.gd array_mesh). glTF / Bevy (counter-clockwise front) take UE's order unchanged.
pub fn flip_winding(idx: &mut [u32]) {
    for t in idx.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_and_scales() {
        assert_eq!(pos([100.0, 200.0, 300.0]), [1.0, 3.0, 2.0]);
        assert_eq!(scale([1.0, 2.0, 3.0]), [1.0, 3.0, 2.0]);
        assert_eq!(quat([0.0, 0.0, 0.0, 1.0]), [0.0, 0.0, 0.0, -1.0]);
        let mut i = [0u32, 1, 2, 3, 4, 5];
        flip_winding(&mut i);
        assert_eq!(i, [0, 2, 1, 3, 5, 4]);
    }
}
