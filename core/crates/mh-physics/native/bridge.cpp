// Thin C ABI over Mordhau's installed PhysX 3.4.0 DLLs. No UE objects or game memory are accessed.
// The public API/extension headers come from the pinned external NVIDIA SDK (scripts/build_physx.py).
#include <PxPhysicsAPI.h>
#include <extensions/PxMassProperties.h>
#include <extensions/PxD6Joint.h>
#include <malloc.h>
#include <vector>
#include <cstdio>
#include <cstring>
#include <cmath>
#include <cstddef>
using namespace physx;

// Original shipping PDB: wall-cooked-query-abi-{pdb,base-enums-pdb}.json.
// These checks also compile in the guarded build using the unchanged reviewed SDK overlay.
static_assert(sizeof(PxActorShape)==16 && offsetof(PxActorShape,actor)==0 && offsetof(PxActorShape,shape)==8);
static_assert(sizeof(PxQueryHit)==24 && offsetof(PxQueryHit,faceIndex)==16);
static_assert(sizeof(PxLocationHit)==56 && offsetof(PxLocationHit,flags)==24);
static_assert(offsetof(PxLocationHit,position)==28 && offsetof(PxLocationHit,normal)==40 && offsetof(PxLocationHit,distance)==52);
static_assert(sizeof(PxRaycastHit)==64 && offsetof(PxRaycastHit,u)==56 && offsetof(PxRaycastHit,v)==60);
static_assert(sizeof(PxGeometryHolder)==48 && sizeof(PxTriangleMeshGeometry)==48 && sizeof(PxConvexMeshGeometry)==48);
static_assert(offsetof(PxTriangleMeshGeometry,scale)==4 && offsetof(PxTriangleMeshGeometry,meshFlags)==32 && offsetof(PxTriangleMeshGeometry,triangleMesh)==40);
static_assert(offsetof(PxConvexMeshGeometry,scale)==4 && offsetof(PxConvexMeshGeometry,convexMesh)==32 && offsetof(PxConvexMeshGeometry,meshFlags)==40);
static_assert(sizeof(PxMeshScale)==28 && offsetof(PxMeshScale,scale)==0 && offsetof(PxMeshScale,rotation)==12);
static_assert(sizeof(PxTransform)==28 && offsetof(PxTransform,q)==0 && offsetof(PxTransform,p)==16 && sizeof(PxBounds3)==24);
static_assert(sizeof(PxHitFlags)==2 && sizeof(PxMeshGeometryFlags)==1);
static_assert(sizeof(PxVec3)==12 && offsetof(PxVec3,x)==0 && offsetof(PxVec3,y)==4 && offsetof(PxVec3,z)==8);
static_assert(PxTriangleMeshFlag::e16_BIT_INDICES==2);
static_assert(PxGeometryType::eCONVEXMESH==4 && PxGeometryType::eTRIANGLEMESH==5);
static_assert(PxMeshGeometryFlag::eDOUBLE_SIDED==2 && PxHitFlag::eMTD==0x200 && PxHitFlag::eDEFAULT==0x407);

struct Xf { float p[3], q[4]; };
struct ShapeDesc { unsigned kind; Xf local; float size[3]; float rest_offset, contact_offset; };
struct BodyDesc {
    Xf world; float mass, linear_damping, angular_damping, com_nudge[3], sleep_threshold;
    unsigned position_iterations, velocity_iterations, kinematic;
    float velocity[3]; unsigned group; float max_angular_velocity;
};
struct JointDesc {
    unsigned body0, body1; Xf frame0, frame1; unsigned motion[6];
    float linear_limit, swing1, swing2, twist, restitution[3], contact_distance[3];
    float stiffness[3], damping[3]; unsigned soft[3], disable_collision, projection, parent_dominates;
    float projection_linear, projection_angular;
};
static char error_text[1024] = "";
#if defined(MH_PHYSX_VALIDATION)
#include "validation_allocator.h"
using Alloc = GuardAllocator;
static Alloc allocator;
#else
struct Alloc : PxAllocatorCallback {
    void* allocate(size_t n, const char*, const char*, int) override { return _aligned_malloc(n, 16); }
    void deallocate(void* p) override { _aligned_free(p); }
} allocator;
#endif
static void validate_native_heap() {
#if defined(MH_PHYSX_VALIDATION)
    heap_check(allocator);
#endif
}
#if defined(MH_PHYSX_VALIDATION)
static float original_reciprocal_round_trip(float value) {
    // Installed set/get mass and inertia use two DIVSS instructions (original DLL
    // RVAs0x3db50/0x3ca50 and0x3db70/0x3ca90). Preserve the intermediate rounding.
    volatile float inverse=1.f/value;
    return 1.f/inverse;
}
#endif
struct Errors : PxErrorCallback {
    void reportError(PxErrorCode::Enum c, const char* s, const char*, int) override {
        std::snprintf(error_text, sizeof(error_text), "PhysX error %u: %s", unsigned(c), s);
        std::fprintf(stderr, "%s\n", error_text);
#if defined(MH_PHYSX_VALIDATION)
        stop("unexpected PhysX diagnostic in production-path validation");
#endif
    }
} errors;
struct Dispatcher : PxCpuDispatcher {
    void submitTask(PxBaseTask& t) override { t.run(); t.release(); }
    PxU32 getWorkerCount() const override { return 0; }
};
static PxFoundation* foundation = nullptr;
static PxPhysics* physics = nullptr;
static PxCooking* cooking = nullptr;
static unsigned scenes = 0;
struct Scene {
    Dispatcher dispatcher; PxScene* scene = nullptr; PxMaterial* material = nullptr;
    std::vector<PxRigidDynamic*> bodies; std::vector<PxRigidStatic*> statics;
    std::vector<PxD6Joint*> joints; std::vector<PxBase*> meshes;
    std::vector<PxU32> disabled_pairs;
    // Meshes are released by the existing meshes owner; null entries preserve cooked convex ordinals.
    struct CookedMesh { PxBase* mesh; unsigned kind; };
    std::vector<CookedMesh> cooked;
};
static PxVec3 vec(const float* v) { return PxVec3(v[0],v[1],v[2]); }
static PxTransform xf(const Xf& v) { return PxTransform(vec(v.p), PxQuat(v.q[0],v.q[1],v.q[2],v.q[3]).getNormalized()); }
static Xf out(const PxTransform& t) { return {{t.p.x,t.p.y,t.p.z},{t.q.x,t.q.y,t.q.z,t.q.w}}; }
static PxFilterFlags filter(PxFilterObjectAttributes, PxFilterData a, PxFilterObjectAttributes, PxFilterData b,
                          PxPairFlags& flags, const void* data, PxU32 bytes) {
    // PhysXSimFilterShader 0x33457a0: same-component map KEY presence disables the pair, even when its value is false.
    if(a.word0 && a.word0==b.word0) {
        auto* pairs=static_cast<const PxU32*>(data);
        const auto lo=PxMin(a.word1,b.word1), hi=PxMax(a.word1,b.word1);
        for(PxU32 i=0; i+1<bytes/sizeof(PxU32); i+=2)
            if(pairs[i]==lo && pairs[i+1]==hi) return PxFilterFlag::eKILL;
    }
    flags = PxPairFlag::eCONTACT_DEFAULT;
    return PxFilterFlag::eDEFAULT;
}
extern "C" __declspec(dllexport) const char* mh_px_error() { return error_text; }
extern "C" __declspec(dllexport) unsigned mh_px_version() { return PX_PHYSICS_VERSION; }
extern "C" __declspec(dllexport) unsigned mh_px_abi_revision() { return 0x20261008; }
extern "C" __declspec(dllexport) unsigned mh_px_validation() {
#if defined(MH_PHYSX_VALIDATION)
    return 1;
#else
    return 0;
#endif
}
extern "C" __declspec(dllexport) Scene* mh_px_scene(float gravity_z, float friction, float restitution,
                                                   float scale_length, float scale_speed) {
    error_text[0] = 0;
#if defined(MH_PHYSX_VALIDATION)
    if(!HeapSetInformation(nullptr,HeapEnableTerminationOnCorruption,nullptr,0)) stop("heap termination instrumentation");
    SYSTEM_INFO system; GetSystemInfo(&system);
    if(system.dwPageSize!=GuardAllocator::page) stop("unexpected page size");
#endif
    if (!foundation) foundation = PxCreateFoundation(PX_FOUNDATION_VERSION, allocator, errors);
    if (!foundation) return nullptr;
    PxTolerancesScale scale; scale.length = scale_length; scale.mass = 1000; scale.speed = scale_speed;
    if (!physics) physics = PxCreateBasePhysics(PX_PHYSICS_VERSION, *foundation, scale, false, nullptr);
    if (!physics) return nullptr;
    // FPhysXPlatformModule::GetPhysXCooking RVA0x2e19580: original profile,
    // in addition to the PDB-matched descriptor layout supplied by build_physx.py.
    PxCookingParams params(scale);
    params.meshPreprocessParams = PxMeshPreprocessingFlag::eWELD_VERTICES;
    params.meshWeldTolerance = .1f;
    params.midphaseDesc.setToDefault(PxMeshMidPhase::eBVH33);
    if (!cooking) cooking = PxCreateCooking(PX_PHYSICS_VERSION, *foundation, params);
    if (!cooking) return nullptr;
    Scene* s = new Scene;
    PxSceneDesc desc(scale); desc.gravity = PxVec3(0,0,gravity_z);
    // DefaultEngine.ini PhysicsSettings; InitPhysScene 0x3342340. SDK otherwise enables PCM by default.
    desc.flags &= ~PxSceneFlag::eENABLE_PCM;
    desc.flags &= ~PxSceneFlag::eENABLE_STABILIZATION;
    desc.bounceThresholdVelocity=200;
    desc.cpuDispatcher = &s->dispatcher; desc.filterShader = filter;
    s->scene = physics->createScene(desc);
    s->material = physics->createMaterial(friction, friction, restitution);
    if (!s->scene || !s->material) {
        if (s->scene) s->scene->release(); if (s->material) s->material->release(); delete s; return nullptr;
    }
    ++scenes; validate_native_heap(); return s;
}
extern "C" __declspec(dllexport) void mh_px_release(Scene* s) {
    if (!s) return;
    for (auto* j : s->joints) if (j) j->release();
    for (auto* b : s->bodies) if (b) b->release();
    for (auto* b : s->statics) b->release();
    for (auto* m : s->meshes) m->release();
    s->scene->release(); s->material->release(); delete s;
    if (--scenes == 0) { cooking->release(); cooking=nullptr; physics->release(); physics=nullptr; foundation->release(); foundation=nullptr; }
    validate_native_heap();
#if defined(MH_PHYSX_VALIDATION)
    if(scenes==0) {
        allocator.require_empty();
        std::printf("PHYSX_VALIDATION_RELEASE allocations=%zu peak=%zu live=0\n",allocator.count,allocator.peak);
        std::fflush(stdout);
    }
#endif
}
extern "C" __declspec(dllexport) unsigned mh_px_body(Scene* s, const BodyDesc* d, const ShapeDesc* shapes, unsigned n) {
    if (!s || !d || !n || d->mass<=0) return ~0u;
    auto* b = physics->createRigidDynamic(xf(d->world)); if (!b) return ~0u;
    std::vector<PxMassProperties> masses; std::vector<PxTransform> locals;
    for (unsigned i=0;i<n;++i) {
        const auto& e=shapes[i]; PxGeometryHolder g;
        if (e.kind==0) g.storeAny(PxSphereGeometry(e.size[0]));
        else if(e.kind==1) g.storeAny(PxBoxGeometry(e.size[0],e.size[1],e.size[2]));
        else if(e.kind==2) g.storeAny(PxCapsuleGeometry(e.size[0],e.size[1]));
        else { b->release(); return ~0u; }
        auto* sh=physics->createShape(g.any(),*s->material,true);
        if(!sh) { b->release(); return ~0u; }
        const auto local=xf(e.local); sh->setLocalPose(local);
        sh->setRestOffset(e.rest_offset); sh->setContactOffset(e.contact_offset);
        PxFilterData fd; fd.word0=d->group; fd.word1=unsigned(s->bodies.size()); sh->setSimulationFilterData(fd);
        b->attachShape(*sh); sh->release(); masses.emplace_back(g.any()); locals.push_back(local);
    }
    auto mp=PxMassProperties::sum(masses.data(),locals.data(),unsigned(masses.size()));
    mp=mp*(d->mass/mp.mass); PxQuat inertia_rotation;
    const auto inertia=PxMassProperties::getMassSpaceInertia(mp.inertiaTensor,inertia_rotation);
    b->setMass(d->mass); b->setMassSpaceInertiaTensor(inertia);
    b->setCMassLocalPose(PxTransform(mp.centerOfMass+vec(d->com_nudge),inertia_rotation));
    b->setLinearDamping(d->linear_damping); b->setAngularDamping(d->angular_damping);
    b->setSolverIterationCounts(d->position_iterations,d->velocity_iterations);
    b->setSleepThreshold(d->sleep_threshold); b->setRigidBodyFlag(PxRigidBodyFlag::eKINEMATIC,d->kinematic!=0);
    b->setMaxAngularVelocity(d->max_angular_velocity);
    if (!d->kinematic) b->setLinearVelocity(vec(d->velocity));
#if defined(MH_PHYSX_VALIDATION)
    // Original readback slots checked in the accepted metadata ABI fixture.
    const float actual_mass=b->getMass();
    const PxVec3 actual_inertia=b->getMassSpaceInertiaTensor();
    const PxTransform actual_com=b->getCMassLocalPose();
    const PxVec3 wanted_com=mp.centerOfMass+vec(d->com_nudge);
    std::printf("PHYSX_VALIDATION_BODY_VALUES id=%zu mass_wanted=%.9g mass_actual=%.9g mass_hex=(%a,%a) inertia_wanted=(%.9g,%.9g,%.9g) inertia_actual=(%.9g,%.9g,%.9g) com_wanted=(%.9g,%.9g,%.9g) com_actual=(%.9g,%.9g,%.9g) q_wanted=(%.9g,%.9g,%.9g,%.9g) q_actual=(%.9g,%.9g,%.9g,%.9g)\n",
                s->bodies.size(),d->mass,actual_mass,double(d->mass),double(actual_mass),
                inertia.x,inertia.y,inertia.z,actual_inertia.x,actual_inertia.y,actual_inertia.z,
                wanted_com.x,wanted_com.y,wanted_com.z,actual_com.p.x,actual_com.p.y,actual_com.p.z,
                inertia_rotation.x,inertia_rotation.y,inertia_rotation.z,inertia_rotation.w,
                actual_com.q.x,actual_com.q.y,actual_com.q.z,actual_com.q.w);
    std::fflush(stdout);
    if(!std::isfinite(actual_mass) || actual_mass<=0 || !actual_inertia.isFinite() ||
       actual_inertia.x<=0 || actual_inertia.y<=0 || actual_inertia.z<=0 || !actual_com.isFinite())
        stop("nonfinite/nonpositive production mass properties");
    if(actual_mass!=original_reciprocal_round_trip(d->mass) ||
       actual_inertia.x!=original_reciprocal_round_trip(inertia.x) ||
       actual_inertia.y!=original_reciprocal_round_trip(inertia.y) ||
       actual_inertia.z!=original_reciprocal_round_trip(inertia.z) ||
       (actual_com.p-(mp.centerOfMass+vec(d->com_nudge))).magnitudeSquared()>1e-6f)
        stop("production mass property getter differs from set value");
    std::printf("PHYSX_VALIDATION_BODY id=%zu shapes=%u mass=%.9g inertia=(%.9g,%.9g,%.9g) com=(%.9g,%.9g,%.9g)\n",
                s->bodies.size(),n,actual_mass,actual_inertia.x,actual_inertia.y,actual_inertia.z,
                actual_com.p.x,actual_com.p.y,actual_com.p.z);
    std::fflush(stdout);
#endif
    s->scene->addActor(*b); s->bodies.push_back(b); validate_native_heap(); return unsigned(s->bodies.size()-1);
}
extern "C" __declspec(dllexport) unsigned mh_px_joint(Scene* s, const JointDesc* d) {
    if (!s || d->body0>=s->bodies.size() || d->body1>=s->bodies.size()) return ~0u;
    auto* j=PxD6JointCreate(*physics,s->bodies[d->body0],xf(d->frame0),s->bodies[d->body1],xf(d->frame1));
    if(!j) return ~0u;
    for(unsigned i=0;i<6;++i) j->setMotion(PxD6Axis::Enum(i),PxD6Motion::Enum(d->motion[i]));
    PxJointLinearLimit lin(physics->getTolerancesScale(),d->linear_limit,d->contact_distance[0]);
    PxJointLimitCone cone(d->swing1,d->swing2,d->contact_distance[1]);
    PxJointAngularLimitPair twist(-d->twist,d->twist,d->contact_distance[2]);
    PxJointLimitParameters* limits[]={&lin,&cone,&twist};
    for(unsigned i=0;i<3;++i) {
        limits[i]->restitution=d->restitution[i];
        limits[i]->stiffness=d->soft[i]?d->stiffness[i]:0; limits[i]->damping=d->soft[i]?d->damping[i]:0;
    }
    if(d->motion[0]==1 || d->motion[1]==1 || d->motion[2]==1) j->setLinearLimit(lin);
    if(d->motion[4]==1 || d->motion[5]==1) j->setSwingLimit(cone);
    if(d->motion[3]==1) j->setTwistLimit(twist);
    j->setConstraintFlag(PxConstraintFlag::eCOLLISION_ENABLED,!d->disable_collision);
    j->setConstraintFlag(PxConstraintFlag::ePROJECTION,d->projection!=0);
    j->setProjectionLinearTolerance(d->projection_linear); j->setProjectionAngularTolerance(d->projection_angular);
    if(d->parent_dominates) { j->setInvMassScale0(0); j->setInvInertiaScale0(0); }
    s->joints.push_back(j); validate_native_heap();
#if defined(MH_PHYSX_VALIDATION)
    std::printf("PHYSX_VALIDATION_JOINT id=%zu bodies=%u,%u\n",s->joints.size()-1,d->body0,d->body1);
    std::fflush(stdout);
#endif
    return unsigned(s->joints.size()-1);
}
extern "C" __declspec(dllexport) void mh_px_disable_pairs(Scene* s,const unsigned* pairs,unsigned count) {
    if(!s || !pairs || !count) return;
    for(unsigned i=0;i<count;++i) {
        s->disabled_pairs.push_back(PxMin(pairs[i*2],pairs[i*2+1]));
        s->disabled_pairs.push_back(PxMax(pairs[i*2],pairs[i*2+1]));
    }
    s->scene->setFilterShaderData(s->disabled_pairs.data(),PxU32(s->disabled_pairs.size()*sizeof(PxU32)));
    validate_native_heap();
}
extern "C" __declspec(dllexport) int mh_px_static(Scene* s, const float* points, unsigned count, float radius, float contact_offset) {
    if(!s || !points || !count) return 0;
    PxGeometryHolder g; PxTransform pose(PxIdentity);
    if(count==1) { g.storeAny(PxSphereGeometry(radius)); pose.p=vec(points); }
    else if(count==2) {
        auto a=vec(points), b=vec(points+3), dir=b-a; const float len=dir.magnitude();
        if(len<1e-6f) { g.storeAny(PxSphereGeometry(radius)); pose.p=a; }
        else { dir/=len; auto axis=PxVec3(1,0,0).cross(dir);
            pose=PxTransform((a+b)*.5f,dir.x<-.99999f?PxQuat(PxPi,PxVec3(0,1,0)):PxQuat(axis.x,axis.y,axis.z,1+dir.x).getNormalized());
            g.storeAny(PxCapsuleGeometry(radius,len*.5f)); }
    } else if(count==3) {
        PxTriangleMeshDesc md; unsigned indices[]={0,1,2};
        md.points.count=3; md.points.stride=12; md.points.data=points;
        md.triangles.count=1; md.triangles.stride=12; md.triangles.data=indices;
        auto* m=cooking->createTriangleMesh(md,physics->getPhysicsInsertionCallback()); if(!m) return 0;
        s->meshes.push_back(m); g.storeAny(PxTriangleMeshGeometry(m));
    } else {
        if(radius!=0) return 0;
        PxConvexMeshDesc md; md.points.count=count; md.points.stride=12; md.points.data=points; md.flags=PxConvexFlag::eCOMPUTE_CONVEX;
        auto* m=cooking->createConvexMesh(md,physics->getPhysicsInsertionCallback()); if(!m) return 0;
        s->meshes.push_back(m); g.storeAny(PxConvexMeshGeometry(m));
    }
    auto* b=physics->createRigidStatic(pose); if(!b) return 0;
    auto* sh=physics->createShape(g.any(),*s->material,true); if(!sh) { b->release(); return 0; }
    sh->setContactOffset(contact_offset);
    b->attachShape(*sh); sh->release(); s->scene->addActor(*b); s->statics.push_back(b); validate_native_heap(); return 1;
}
extern "C" __declspec(dllexport) int mh_px_step(Scene* s, float dt) {
    if(!s || !std::isfinite(dt) || dt<=0) return 0;
    s->scene->simulate(dt); PxU32 error_state=0;
    const bool fetched=s->scene->fetchResults(true,&error_state);
    validate_native_heap();
    if(!fetched || error_state) {
        std::snprintf(error_text,sizeof(error_text),"PhysX fetchResults failed: fetched=%u errorState=%u",unsigned(fetched),error_state);
        return 0;
    }
    return 1;
}
extern "C" __declspec(dllexport) int mh_px_pose(Scene* s,unsigned body,Xf* p) {
    if(!s || body>=s->bodies.size() || !s->bodies[body] || !p) return 0;
    *p=out(s->bodies[body]->getGlobalPose()); return 1;
}
extern "C" __declspec(dllexport) int mh_px_state(Scene* s,unsigned body,float* state) {
    if(!s || body>=s->bodies.size() || !s->bodies[body] || !state) return 0;
    auto* b=s->bodies[body]; auto v=b->getLinearVelocity();
    state[0]=b->getMass(); state[1]=b->getLinearDamping(); state[2]=b->getAngularDamping();
    state[3]=b->getSleepThreshold(); state[4]=b->isSleeping()?1.f:0.f;
    state[5]=v.x;state[6]=v.y;state[7]=v.z;return 1;
}
extern "C" __declspec(dllexport) int mh_px_impulse(Scene* s,unsigned body,const float* impulse,const float* point) {
    if(!s || body>=s->bodies.size() || !s->bodies[body]) return 0;
    auto* b=s->bodies[body]; const auto com=b->getGlobalPose().transform(b->getCMassLocalPose().p);
    const auto force=vec(impulse); b->addForce(force,PxForceMode::eIMPULSE);
    b->addTorque((vec(point)-com).cross(force),PxForceMode::eIMPULSE);validate_native_heap();return 1;
}
extern "C" __declspec(dllexport) void mh_px_remove(Scene* s,const unsigned* bodies,unsigned count) {
    if(!s) return;
    // Joints involving a removed actor are explicitly released before the actor; unrelated corpses remain intact.
    for(auto*& j:s->joints) if(j) { PxRigidActor *a,*b; j->getActors(a,b);
        for(unsigned i=0;i<count;++i) if(bodies[i]<s->bodies.size() && (a==s->bodies[bodies[i]] || b==s->bodies[bodies[i]])) {j->release();j=nullptr;break;} }
    for(unsigned i=0;i<count;++i) if(bodies[i]<s->bodies.size()) { auto*& b=s->bodies[bodies[i]]; if(b){b->release();b=nullptr;} }
    validate_native_heap();
}

// Only complete SHA1-verified original PhysXPC bulk payloads enter this API. The Rust owner verifies
// the payload fingerprint and expected bulk size before calling it; this is not an untrusted cooker.
class CookedInput final : public PxInputStream {
public:
    const unsigned char* data; unsigned size, pos=0; bool failed=false;
    CookedInput(const unsigned char* p,unsigned n):data(p),size(n) {}
    PxU32 read(void* dest,PxU32 n) override {
        if(n>size-pos) { failed=true; if(n) std::memset(dest,0,n); return 0; }
        if(n) std::memcpy(dest,data+pos,n); pos+=n; return n;
    }
    bool byte(unsigned char& v) { return read(&v,1)==1; }
    bool word(unsigned& v) { return read(&v,4)==4; }
    bool header(bool triangle) const {
        if(size-pos<16 || std::memcmp(data+pos,"NXS\1",4)) return false;
        if(std::memcmp(data+pos+4,triangle?"MESH":"CVXM",4)) return false;
        unsigned version=0; std::memcpy(&version,data+pos+8,4);
        if(!triangle) return version==13;
        if(version!=14 || size-pos<28) return false;
        unsigned mid=0,flags=0,nv=0,nt=0;
        std::memcpy(&mid,data+pos+12,4); std::memcpy(&flags,data+pos+16,4);
        std::memcpy(&nv,data+pos+20,4); std::memcpy(&nt,data+pos+24,4);
        if(mid>1 || (flags&~63u) || !nv || !nt) return false;
        const unsigned stride=(flags&4)?1:(flags&8)?2:4;
        const unsigned long long minimum=28ull+12ull*nv+3ull*nt*stride+((flags&1)?2ull*nt:0);
        return minimum<=size-pos;
    }
};

struct CookedCounts { unsigned first, normal, mirrored, triangles, consumed; };
struct CookedInfo { unsigned kind, vertices, triangles; float minimum[3],maximum[3]; };
struct CookedRayHit { float position[3],normal[3],distance; unsigned face,material,flags; float u,v; };
static_assert(sizeof(CookedCounts)==20 && sizeof(CookedInfo)==36 && sizeof(CookedRayHit)==48);
static_assert(offsetof(CookedRayHit,normal)==12 && offsetof(CookedRayHit,distance)==24 && offsetof(CookedRayHit,face)==28);
static_assert(offsetof(CookedRayHit,material)==32 && offsetof(CookedRayHit,flags)==36 && offsetof(CookedRayHit,u)==40 && offsetof(CookedRayHit,v)==44);

extern "C" __declspec(dllexport) int mh_px_cooked_load(Scene* s,const unsigned char* data,unsigned size,
                                                        unsigned expected_size,CookedCounts* out) {
    if(!s || !data || !out || size!=expected_size || size<13) return 0;
    error_text[0]=0;
    CookedInput input(data,size); unsigned char endian=0; unsigned counts[3]={};
    if(!input.byte(endian) || endian!=1 || !input.word(counts[0]) || !input.word(counts[1]) || !input.word(counts[2])) return 0;
    const unsigned long long total=static_cast<unsigned long long>(counts[0])+counts[1]+counts[2];
    // Reject invalid signed counts and impossible ordinals before any original deserializer entry.
    if(counts[0]>0x7fffffffu || counts[1]>0x7fffffffu || counts[2]>0x7fffffffu || total>size-13 ||
       total>0xffffffffu-s->cooked.size()) return 0;
    const auto mark=s->meshes.size(), first=s->cooked.size();
    auto rollback=[&]() {
        while(s->meshes.size()>mark) { s->meshes.back()->release(); s->meshes.pop_back(); }
        s->cooked.resize(first); validate_native_heap();
        std::snprintf(error_text,sizeof(error_text),"Original cooked collision rejected at byte %u/%u",input.pos,size);
        return 0;
    };
    for(unsigned group=0;group<3;++group) for(unsigned i=0;i<counts[group];++i) {
        PxBase* mesh=nullptr;
        if(group<2) {
            unsigned char present=0; if(!input.byte(present) || present>1) return rollback();
            if(present) {
                if(!input.header(false)) return rollback();
                const auto before=input.pos;
                mesh=physics->createConvexMesh(input); // Original PDB/vcall +0x58; no cooking.
                if(mesh) s->meshes.push_back(mesh);
                if(!mesh || input.failed || input.pos<=before) return rollback();
            }
        } else {
            if(!input.header(true)) return rollback();
            const auto before=input.pos;
            mesh=physics->createTriangleMesh(input); // Original PDB/vcall +0x28; no presence byte.
            if(mesh) s->meshes.push_back(mesh);
            if(!mesh || input.failed || input.pos<=before) return rollback();
        }
        s->cooked.push_back({mesh,group<2?4u:5u});
    }
    *out={static_cast<unsigned>(first),counts[0],counts[1],counts[2],input.pos};
    validate_native_heap(); return 1;
}

extern "C" __declspec(dllexport) int mh_px_cooked_info(Scene* s,unsigned handle,CookedInfo* out) {
    if(!s || !out || handle>=s->cooked.size()) return 0;
    const auto& entry=s->cooked[handle]; if(!entry.mesh) return 0;
    PxBounds3 bounds; unsigned vertices=0,triangles=0;
    if(entry.kind==5) {
        auto* mesh=static_cast<PxTriangleMesh*>(entry.mesh);
        vertices=mesh->getNbVertices(); triangles=mesh->getNbTriangles(); bounds=mesh->getLocalBounds();
    } else {
        auto* mesh=static_cast<PxConvexMesh*>(entry.mesh);
        vertices=mesh->getNbVertices(); bounds=mesh->getLocalBounds();
    }
    if(!bounds.isValid() || !vertices) return 0;
    *out={entry.kind,vertices,triangles,{bounds.minimum.x,bounds.minimum.y,bounds.minimum.z},
          {bounds.maximum.x,bounds.maximum.y,bounds.maximum.z}};
    validate_native_heap(); return 1;
}

extern "C" __declspec(dllexport) int mh_px_cooked_ray(Scene* s,unsigned handle,const Xf* placement,
        const float* scale,unsigned double_sided,const float* origin,const float* unit_direction,
        float distance,CookedRayHit* out) {
    if(!s || !placement || !scale || !origin || !unit_direction || !out || handle>=s->cooked.size()) return -1;
    const auto& entry=s->cooked[handle]; if(!entry.mesh || double_sided>1) return -1;
    const auto position=vec(origin),direction=vec(unit_direction),sc=vec(scale);
    if(!position.isFinite() || !direction.isFinite() || !sc.isFinite() || !std::isfinite(distance) || distance<=0 ||
       std::fabs(direction.magnitudeSquared()-1)>1e-4f) return -1;
    const auto pose=xf(*placement); if(!pose.isValid()) return -1;
    PxGeometryHolder geometry; PxMeshScale mesh_scale(sc,PxQuat(PxIdentity));
    if(entry.kind==5) {
        PxMeshGeometryFlags flags;
        if(double_sided) flags|=PxMeshGeometryFlag::eDOUBLE_SIDED;
        PxTriangleMeshGeometry g(static_cast<PxTriangleMesh*>(entry.mesh),mesh_scale,flags);
        if(!g.isValid()) return -1; geometry.storeAny(g);
    } else {
        PxConvexMeshGeometry g(static_cast<PxConvexMesh*>(entry.mesh),mesh_scale);
        if(!g.isValid()) return -1; geometry.storeAny(g);
    }
    PxRaycastHit hit;
    hit.u=0; hit.v=0; // eUV is not requested; never publish unspecified query outputs.
    // ORIGINAL UE SceneCastSingleRay 0x2f4a92e: 0x607. MSVC passes the PxHitFlags class indirectly.
    // Keep the SDK C++ declaration here; do not bind its class-valued argument as a raw Rust u16.
    const PxHitFlags flags(static_cast<PxU16>(0x607));
    const auto n=PxGeometryQuery::raycast(position,direction,geometry.any(),pose,distance,flags,1,&hit);
    validate_native_heap(); if(!n) return 0;
    if(!hit.position.isFinite() || !hit.normal.isFinite() || !std::isfinite(hit.distance)) return -1;
    unsigned material=0;
    if(entry.kind==5) {
        auto* mesh=static_cast<PxTriangleMesh*>(entry.mesh);
        if(hit.faceIndex>=mesh->getNbTriangles()) return -1;
        material=mesh->getTriangleMaterialIndex(hit.faceIndex); // Original material getter +0x68.
    }
    *out={{hit.position.x,hit.position.y,hit.position.z},{hit.normal.x,hit.normal.y,hit.normal.z},hit.distance,
          hit.faceIndex,material,static_cast<unsigned>(static_cast<PxU16>(hit.flags)),hit.u,hit.v};
    return 1;
}

// Expose the ORIGINAL loaded arrays only to the isolated guarded fixture. Production keeps the
// same symbol but rejects it, so the test cannot silently inspect a different unguarded DLL.
extern "C" __declspec(dllexport) int mh_px_cooked_readback(Scene* s,unsigned handle,float* vertices,
        unsigned vertex_count,unsigned* indices,unsigned triangle_count,unsigned* materials) {
#if defined(MH_PHYSX_VALIDATION)
    if(!s || !vertices || !indices || !materials || handle>=s->cooked.size()) return 0;
    const auto& entry=s->cooked[handle]; if(!entry.mesh || entry.kind!=5) return 0;
    auto* mesh=static_cast<PxTriangleMesh*>(entry.mesh);
    if(vertex_count!=mesh->getNbVertices() || triangle_count!=mesh->getNbTriangles()) return 0;
    const auto* source=mesh->getVertices(); // Original +0x30, PxVec3 is exactly 12 bytes.
    const auto* triangles=mesh->getTriangles(); // Original +0x50.
    if(!source || !triangles) return 0;
    const bool narrow=(static_cast<PxU8>(mesh->getTriangleMeshFlags())&2)!=0; // Original +0x58.
    for(unsigned i=0;i<vertex_count;++i) {
        if(!source[i].isFinite()) return 0;
        vertices[i*3]=source[i].x; vertices[i*3+1]=source[i].y; vertices[i*3+2]=source[i].z;
    }
    for(unsigned i=0;i<triangle_count;++i) {
        for(unsigned j=0;j<3;++j) {
            const auto index=narrow?static_cast<const PxU16*>(triangles)[i*3+j]:static_cast<const PxU32*>(triangles)[i*3+j];
            if(index>=vertex_count) return 0; indices[i*3+j]=index;
        }
        materials[i]=mesh->getTriangleMaterialIndex(i); // Original +0x68, never render-section guesses.
    }
    validate_native_heap(); return 1;
#else
    (void)s; (void)handle; (void)vertices; (void)vertex_count; (void)indices; (void)triangle_count; (void)materials;
    return 0;
#endif
}
