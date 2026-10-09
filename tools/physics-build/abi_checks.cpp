// Authored facts about the supported installed x64 interfaces. Compile-only;
// the resulting assembly is reviewed before any native entry.
#include <cstddef>
#include <cstdint>
#include <type_traits>
#include <new>
#define private public
#define protected public
#include <PxPhysicsAPI.h>
#undef protected
#undef private
#include "mh_installed_physx.h"
using namespace physx;
#define SIZE(T, N) static_assert(sizeof(T) == N, #T " size")
#define AT(T, F, N) static_assert(offsetof(T, F) == N, #T "." #F " offset")
SIZE(void*, 8);
static_assert(PX_FOUNDATION_VERSION == 0x01000000);
static_assert(PX_USE_CLOTH_API == 1 && PX_USE_PARTICLE_SYSTEM_API == 1 && PX_SUPPORT_GPU_PHYSX == 1);
SIZE(PxSceneFlags, 4); SIZE(PxPairFlags, 2); SIZE(PxFilterFlags, 2);
SIZE(PxBaseTask, 24); AT(PxBaseTask, mContextID, 8); AT(PxBaseTask, mTm, 16);
SIZE(PxBase, 16); AT(PxBase, mConcreteType, 8); AT(PxBase, mBaseFlags, 10);
SIZE(PxTolerancesScale, 12); AT(PxTolerancesScale, mass, 4); AT(PxTolerancesScale, speed, 8);
SIZE(PxTransform, 28); AT(PxTransform, q, 0); AT(PxTransform, p, 16);
SIZE(PxSceneDesc, 288);
AT(PxSceneDesc, gravity, 0); AT(PxSceneDesc, simulationEventCallback, 0x10);
AT(PxSceneDesc, contactModifyCallback, 0x18); AT(PxSceneDesc, ccdContactModifyCallback, 0x20);
AT(PxSceneDesc, filterShaderData, 0x28); AT(PxSceneDesc, filterShaderDataSize, 0x30);
AT(PxSceneDesc, filterShader, 0x38); AT(PxSceneDesc, filterCallback, 0x40);
AT(PxSceneDesc, broadPhaseType, 0x48); AT(PxSceneDesc, broadPhaseCallback, 0x50);
AT(PxSceneDesc, limits, 0x58); AT(PxSceneDesc, frictionType, 0x7c);
AT(PxSceneDesc, bounceThresholdVelocity, 0x80); AT(PxSceneDesc, frictionOffsetThreshold, 0x84);
AT(PxSceneDesc, ccdMaxSeparation, 0x88); AT(PxSceneDesc, flags, 0x8c);
AT(PxSceneDesc, cpuDispatcher, 0x90); AT(PxSceneDesc, gpuDispatcher, 0x98);
AT(PxSceneDesc, staticStructure, 0xa0); AT(PxSceneDesc, dynamicStructure, 0xa4);
AT(PxSceneDesc, dynamicTreeRebuildRateHint, 0xa8); AT(PxSceneDesc, sceneQueryUpdateMode, 0xac);
AT(PxSceneDesc, userData, 0xb0); AT(PxSceneDesc, solverBatchSize, 0xb8);
AT(PxSceneDesc, nbContactDataBlocks, 0xbc); AT(PxSceneDesc, maxNbContactDataBlocks, 0xc0);
AT(PxSceneDesc, maxBiasCoefficient, 0xc4); AT(PxSceneDesc, contactReportStreamBufferSize, 0xc8);
AT(PxSceneDesc, ccdMaxPasses, 0xcc); AT(PxSceneDesc, wakeCounterResetValue, 0xd0);
AT(PxSceneDesc, sanityBounds, 0xd4); AT(PxSceneDesc, gpuDynamicsConfig, 0xec);
AT(PxSceneDesc, gpuMaxNumPartitions, 0x10c); AT(PxSceneDesc, gpuComputeVersion, 0x110);
AT(PxSceneDesc, tolerancesScale, 0x114);
SIZE(PxCookingParams, 68);
AT(PxCookingParams, targetPlatform, 0); AT(PxCookingParams, skinWidth, 4);
AT(PxCookingParams, areaTestEpsilon, 8); AT(PxCookingParams, planeTolerance, 12);
AT(PxCookingParams, convexMeshCookingType, 0x10); AT(PxCookingParams, suppressTriangleMeshRemapTable, 0x14);
AT(PxCookingParams, buildTriangleAdjacencies, 0x15); AT(PxCookingParams, buildGPUData, 0x16);
AT(PxCookingParams, scale, 0x18); AT(PxCookingParams, meshPreprocessParams, 0x24);
AT(PxCookingParams, meshCookingHint, 0x28); AT(PxCookingParams, meshSizePerformanceTradeOff, 0x2c);
AT(PxCookingParams, meshWeldTolerance, 0x30); AT(PxCookingParams, midphaseDesc, 0x34);
AT(PxCookingParams, gaussMapLimit, 0x40);
SIZE(PxTriangleMeshDesc, 72); SIZE(PxConvexMeshDesc, 80); SIZE(PxMidphaseDesc, 12);
AT(PxTriangleMeshDesc, materialIndices, 0x38); AT(PxConvexMeshDesc, flags, 0x48);
AT(PxMidphaseDesc, mType, 8);
using TriangleCreate = PxTriangleMesh* (PxCooking::*)(const PxTriangleMeshDesc&, PxPhysicsInsertionCallback&) const;
using ConvexCreate = PxConvexMesh* (PxCooking::*)(const PxConvexMeshDesc&, PxPhysicsInsertionCallback&) const;
static_assert(std::is_same<decltype(&PxCooking::createTriangleMesh), TriangleCreate>::value);
static_assert(std::is_same<decltype(&PxCooking::createConvexMesh), ConvexCreate>::value);
static_assert(std::is_same<decltype(&PxScene::getVisualizationCullingBox), const PxBounds3& (PxScene::*)() const>::value);
using Simulate = void (PxScene::*)(PxReal, PxBaseTask*, void*, PxU32, bool);
static_assert(std::is_same<decltype(&PxScene::simulate), Simulate>::value);
static_assert(std::is_same<decltype(&PxScene::fetchResults), bool (PxScene::*)(bool, PxU32*)>::value);
using GeometryRay = PxU32 (*)(const PxVec3&, const PxVec3&, const PxGeometry&, const PxTransform&, PxReal, PxHitFlags, PxU32, PxRaycastHit*);
static_assert(std::is_same<decltype(&PxGeometryQuery::raycast), GeometryRay>::value);

// Keep wrappers separate so compiler-emitted vtable displacements can be
// mechanically checked; SDK declarations alone do not verify a runtime ABI.
#define PROBE extern "C" __declspec(noinline)
PROBE void mh_abi_scene_release(PxScene* s) { s->release(); }
PROBE void mh_abi_scene_add(PxScene* s, PxActor& a) { s->addActor(a); }
PROBE void mh_abi_scene_filter(PxScene* s, const void* p, PxU32 n) { s->setFilterShaderData(p, n); }
PROBE void mh_abi_scene_simulate(PxScene* s, float dt, PxBaseTask* t, void* p, PxU32 n, bool c) { s->simulate(dt, t, p, n, c); }
PROBE bool mh_abi_scene_fetch(PxScene* s, bool block, PxU32* error) { return s->fetchResults(block, error); }
PROBE PxTriangleMesh* mh_abi_cook_triangle(const PxCooking* c, const PxTriangleMeshDesc& d, PxPhysicsInsertionCallback& i) { return c->createTriangleMesh(d, i); }
PROBE PxConvexMesh* mh_abi_cook_convex(const PxCooking* c, const PxConvexMeshDesc& d, PxPhysicsInsertionCallback& i) { return c->createConvexMesh(d, i); }
PROBE PxScene* mh_abi_physics_scene(PxPhysics* p, const PxSceneDesc& d) { return p->createScene(d); }
PROBE PxRigidDynamic* mh_abi_physics_body(PxPhysics* p, const PxTransform& x) { return p->createRigidDynamic(x); }
PROBE PxRigidStatic* mh_abi_physics_static(PxPhysics* p, const PxTransform& x) { return p->createRigidStatic(x); }
PROBE PxConstraint* mh_abi_physics_constraint(PxPhysics* p, PxRigidActor* a, PxRigidActor* b, PxConstraintConnector& c, const PxConstraintShaderTable& s, PxU32 n) { return p->createConstraint(a, b, c, s, n); }
PROBE PxMaterial* mh_abi_physics_material(PxPhysics* p, float a, float b, float c) { return p->createMaterial(a, b, c); }
PROBE PxTriangleMesh* mh_abi_physics_triangle(PxPhysics* p, PxInputStream& s) { return p->createTriangleMesh(s); }
PROBE PxConvexMesh* mh_abi_physics_convex(PxPhysics* p, PxInputStream& s) { return p->createConvexMesh(s); }
PROBE PxTransform mh_abi_body_pose(const PxRigidDynamic* p) { return p->getGlobalPose(); }
PROBE float mh_abi_body_mass(const PxRigidDynamic* p) { return p->getMass(); }
PROBE PxVec3 mh_abi_body_inertia(const PxRigidDynamic* p) { return p->getMassSpaceInertiaTensor(); }
PROBE void mh_abi_body_mass_set(PxRigidDynamic* p, float m) { p->setMass(m); }
PROBE void mh_abi_body_inertia_set(PxRigidDynamic* p, const PxVec3& v) { p->setMassSpaceInertiaTensor(v); }
PROBE void mh_abi_body_com(PxRigidDynamic* p, const PxTransform& x) { p->setCMassLocalPose(x); }
PROBE void mh_abi_body_attach(PxRigidDynamic* p, PxShape& s) { p->attachShape(s); }
PROBE void mh_abi_shape_local(PxShape* s, const PxTransform& x) { s->setLocalPose(x); }
PROBE void mh_abi_shape_filter(PxShape* s, const PxFilterData& f) { s->setSimulationFilterData(f); }
PROBE void mh_abi_scene_descriptor(PxSceneDesc* d, const PxTolerancesScale& s) { new(d) PxSceneDesc(s); }
PROBE void mh_abi_cooking_descriptor(PxCookingParams* p, const PxTolerancesScale& s) { new(p) PxCookingParams(s); }
PROBE PxPhysics* mh_abi_factory(PxFoundation& f, const PxTolerancesScale& s) { return PxCreateBasePhysics(mh_installed_physx::version, f, s, false, nullptr); }
PROBE PxCooking* mh_abi_cooking_factory(PxFoundation& f, const PxCookingParams& p) { return PxCreateCooking(mh_installed_physx::version, f, p); }
PROBE PxU32 mh_abi_geometry_ray(const PxVec3& p, const PxVec3& d, const PxGeometry& g, const PxTransform& x, float distance, PxRaycastHit* hit) {
    return PxGeometryQuery::raycast(p, d, g, x, distance, PxHitFlags(static_cast<PxU16>(0x607)), 1, hit);
}
