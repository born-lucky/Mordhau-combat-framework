//! FNetMotion (extract/native/types/FNetMotion.h): Id, MotionType, MotionParam0..2, MotionDynamicParam, one byte
//! each. One per fighter (UMotionSystemComponent +0xc4 NetMotion). Port of MotionSystem.NetMotion and the static
//! helpers of godot/game/net/net_motion_rep.gd.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FNetMotion {
    /// Id (+0x0): UMotionSystemComponent::AssignNetMotion rva=0x14b34c0 sets NetMotion.Id + 1
    pub id: u8,
    pub motion_type: u8,
    pub param0: u8,
    pub param1: u8,
    pub param2: u8,
    pub dynamic_param: u8,
}

impl FNetMotion {
    pub fn new(motion_type: u8, p0: u8, p1: u8, p2: u8, dynamic_param: u8) -> Self {
        FNetMotion { id: 0, motion_type, param0: p0, param1: p1, param2: p2, dynamic_param }
    }

    /// FNetMotion as 6 bytes in member order (types/FNetMotion.h): the wire form of the RPC parameter / property
    pub fn to_bytes(&self) -> [u8; 6] {
        [self.id, self.motion_type, self.param0, self.param1, self.param2, self.dynamic_param]
    }

    pub fn from_bytes(b: [u8; 6]) -> Self {
        FNetMotion { id: b[0], motion_type: b[1], param0: b[2], param1: b[3], param2: b[4], dynamic_param: b[5] }
    }
}

/// The field compare HandleNetMotionUpdate inlines (the PDB declares FNetMotion::Compare static uint8): 0 = equal,
/// 1 = only MotionDynamicParam differs, 2 = Id, MotionType or a Param differs.
pub fn compare(a: &FNetMotion, b: &FNetMotion) -> u8 {
    if a.id != b.id || a.motion_type != b.motion_type || a.param0 != b.param0 || a.param1 != b.param1 || a.param2 != b.param2 {
        return 2;
    }
    if a.dynamic_param == b.dynamic_param {
        0
    } else {
        1
    }
}
