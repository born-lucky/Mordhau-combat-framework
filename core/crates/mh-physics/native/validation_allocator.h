// Diagnostic instrumentation copied from the accepted contact fixture. No physical settings change.
#pragma once
#define NOMINMAX
#include <windows.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include <mutex>
#include <unordered_map>
#include <cstdint>
[[noreturn]] static void stop(const char* message) {
    std::fprintf(stderr,"FAIL %s winerr=%lu\n",message,GetLastError()); std::fflush(nullptr); ExitProcess(90);
}
struct GuardAllocator final : PxAllocatorCallback {
    struct Allocation { void* base; size_t requested,rounded; };
    std::unordered_map<void*,Allocation> live;
    std::mutex lock;
    size_t count=0,peak=0;
    static constexpr size_t page=4096;
    static void verify(void* pointer,const Allocation& allocation) {
        auto p=static_cast<unsigned char*>(pointer);
        for(size_t i=1;i<=16;++i) if(p[-static_cast<ptrdiff_t>(i)]!=0xa5) stop("PhysX allocation prefix overwritten");
        for(size_t i=allocation.requested;i<allocation.rounded;++i) if(p[i]!=0x5a) stop("PhysX allocation suffix overwritten");
    }
    void* allocate(size_t bytes,const char*,const char*,int) override {
        if(bytes>SIZE_MAX-3*page) stop("allocation size overflow");
        const size_t rounded=(bytes+15)&~size_t(15);
        const size_t data_pages=(rounded+16+page-1)/page;
        const size_t reserved=(data_pages+2)*page;
        auto base=static_cast<unsigned char*>(VirtualAlloc(nullptr,reserved,MEM_RESERVE|MEM_COMMIT,PAGE_READWRITE));
        if(!base) stop("guarded VirtualAlloc");
        DWORD old=0;
        if(!VirtualProtect(base,page,PAGE_NOACCESS,&old) || !VirtualProtect(base+(data_pages+1)*page,page,PAGE_NOACCESS,&old)) stop("guard page protection");
        auto result=base+(data_pages+1)*page-rounded;
        std::memset(result-16,0xa5,16);
        if(rounded>bytes) std::memset(result+bytes,0x5a,rounded-bytes);
        std::lock_guard<std::mutex> guard(lock);
        live.emplace(result,Allocation{base,bytes,rounded}); ++count;
        if(live.size()>peak) peak=live.size();
        return result;
    }
    void deallocate(void* pointer) override {
        if(!pointer) return;
        std::lock_guard<std::mutex> guard(lock);
        auto it=live.find(pointer); if(it==live.end()) stop("unknown or double-freed PhysX allocation");
        verify(pointer,it->second); void* base=it->second.base; live.erase(it);
        if(!VirtualFree(base,0,MEM_RELEASE)) stop("guarded VirtualFree");
    }
    void validate() {
        std::lock_guard<std::mutex> guard(lock);
        for(const auto& entry:live) verify(entry.first,entry.second);
    }
    void require_empty() {
        std::lock_guard<std::mutex> guard(lock);
        if(!live.empty()) { std::fprintf(stderr,"outstanding PhysX allocations=%zu\n",live.size());stop("allocations remain after foundation release"); }
    }
};
static void heap_check(GuardAllocator& allocator) {
    allocator.validate();
    if(!HeapValidate(GetProcessHeap(),0,nullptr)) stop("process heap invalid");
    // The former corruption surfaced on a later host allocation: exercise that boundary after each native stage.
    std::vector<void*> blocks;
    for(size_t i=0;i<96;++i) {
        auto p=std::malloc(64+i*43);if(!p)stop("host allocation failed");
        std::memset(p,0x3c,64+i*43);blocks.push_back(p);
    }
    for(auto p:blocks)std::free(p);
    if(!HeapValidate(GetProcessHeap(),0,nullptr)) stop("process heap invalid after host allocation stress");
    allocator.validate();
}
