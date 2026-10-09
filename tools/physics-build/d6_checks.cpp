// Authored original ABI facts. No solver implementation or original constant
// arrays are copied here; the actual solver is the licensed staged source.
#include <cstddef>
#include <cstdint>
#include <type_traits>
#include <new>
#define private public
#define protected public
#include <PxPhysicsAPI.h>
#include <ExtD6Joint.h>
#undef protected
#undef private
#include "mh_installed_physx.h"
using namespace physx;
using JointData = physx::Ext::JointData;
using D6Data = physx::Ext::D6JointData;
#define AT(F, N) static_assert(offsetof(D6Data, F) == N, #F " offset")
static_assert(sizeof(JointData) == 80 && offsetof(JointData, c2b) == 16);
static_assert(offsetof(JointData, invMassScale) == 0 && sizeof(D6Data) == 400);
AT(motion, 0x50); AT(linearLimit, 0x68); AT(twistLimit, 0x80); AT(swingLimit, 0x9c);
AT(drive, 0xb8); AT(drivePosition, 0x118); AT(driveLinearVelocity, 0x134); AT(driveAngularVelocity, 0x140);
AT(locked, 0x14c); AT(limited, 0x150); AT(driving, 0x154);
AT(thSwingY, 0x158); AT(thSwingZ, 0x15c); AT(thSwingPad, 0x160);
AT(tqSwingY, 0x164); AT(tqSwingZ, 0x168); AT(tqSwingPad, 0x16c);
AT(tqTwistLow, 0x170); AT(tqTwistHigh, 0x174); AT(tqTwistPad, 0x178);
AT(linearMinDist, 0x17c); AT(projectionLinearTolerance, 0x180); AT(projectionAngularTolerance, 0x184);
static_assert(sizeof(PxConstraintShaderTable) == 32 && offsetof(PxConstraintShaderTable, flag) == 24);
using Solver = PxU32 (*)(Px1DConstraint*, PxVec3&, PxU32, PxConstraintInvMassScale&, const void*, const PxTransform&, const PxTransform&);
static_assert(std::is_same<PxConstraintSolverPrep, Solver>::value);
static_assert(std::is_same<decltype(PxConstraintShaderTable::solverPrep), Solver>::value);
static_assert(sizeof(Px1DConstraint) == 80 && offsetof(Px1DConstraint, angular0) == 16);
static_assert(offsetof(Px1DConstraint, angular1) == 48 && offsetof(Px1DConstraint, flags) == 76);
static_assert(sizeof(PxJointLimitParameters) == 20 && offsetof(PxJointLimitParameters, contactDistance) == 16);
static_assert(sizeof(PxJointLinearLimit) == 24 && sizeof(PxJointLimitCone) == 28 && sizeof(PxJointAngularLimitPair) == 28);
static_assert(sizeof(PxD6JointDrive) == 16 && sizeof(PxD6JointDriveFlags) == 4);
static_assert(offsetof(PxD6JointDrive, stiffness) == 0 && offsetof(PxD6JointDrive, damping) == 4);
static_assert(offsetof(PxD6JointDrive, forceLimit) == 8 && offsetof(PxD6JointDrive, flags) == 12);
