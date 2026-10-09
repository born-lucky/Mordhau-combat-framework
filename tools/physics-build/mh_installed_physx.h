#pragma once
#include <PxPhysicsVersion.h>

// The BSD source version remains 3.4.2. Factory arguments describe the installed
// DLL ABI, not the source license/version. The builder supplies this include.
static_assert(PX_PHYSICS_VERSION == 0x03040200, "pinned BSD PhysX source required");
namespace mh_installed_physx {
constexpr unsigned version = 0x03040000;
constexpr const char* licensed_source = "5e42a5f112351a223c19c17bb331e6c55037b8eb";
}
