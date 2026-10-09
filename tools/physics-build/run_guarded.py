"""Prepare, or explicitly dispatch, ONE reviewed original-PhysX Rust guard test.

Default preparation performs file verification only. --run is a separate root
dispatch gate. No library is loaded into this Python process except Windows OS
APIs; native entry happens only in the new, suspended/job-capped test child.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import time

from build_bridge import DLL_SHA256, HERE, ROOT, PIN, digest, installed_files, metadata, pe64, save

TESTS = {
    "reject": "tests::production_open_rejects_guarded_bridge_before_creation",
    "primitive": "tests::production_body_geometry_mass_and_teardown",
}
PVD_SHA256 = "39a7088bb1cb810fc2d6e461d3517339b34769d61ae5447df9ec3a629e048cf5"
LIMIT = 1 << 30
TIMEOUT = 30.0
U32, U64, I64, PTR = C.c_uint32, C.c_uint64, C.c_int64, C.c_void_p


class Memory(C.Structure):
    _fields_ = [("length", U32), ("load", U32)] + [(n, U64) for n in ("total_phys", "available_phys", "total_page", "available_page", "total_virtual", "available_virtual", "available_extended")]


class BasicLimit(C.Structure):
    _fields_ = [("process_time", I64), ("job_time", I64), ("flags", U32), ("minimum_ws", C.c_size_t), ("maximum_ws", C.c_size_t), ("active_processes", U32), ("affinity", C.c_size_t), ("priority", U32), ("scheduling", U32)]


class ExtendedLimit(C.Structure):
    _fields_ = [("basic", BasicLimit), ("io", U64 * 6), ("process_memory", C.c_size_t), ("job_memory", C.c_size_t), ("peak_process", C.c_size_t), ("peak_job", C.c_size_t)]


class Startup(C.Structure):
    _fields_ = [("cb", U32), ("reserved", C.c_wchar_p), ("desktop", C.c_wchar_p), ("title", C.c_wchar_p)] + [(n, U32) for n in ("x", "y", "x_size", "y_size", "x_chars", "y_chars", "fill", "flags")] + [("show", C.c_uint16), ("reserved_size", C.c_uint16), ("reserved_data", PTR), ("stdin", PTR), ("stdout", PTR), ("stderr", PTR)]


class Process(C.Structure):
    _fields_ = [("process", PTR), ("thread", PTR), ("pid", U32), ("tid", U32)]


class ExceptionRecord(C.Structure):
    _fields_ = [("code", U32), ("flags", U32), ("record", PTR), ("address", PTR), ("count", U32), ("information", C.c_size_t * 15)]


class ExceptionInfo(C.Structure):
    _fields_ = [("record", ExceptionRecord), ("first_chance", U32)]


class CreateInfo(C.Structure):
    _fields_ = [("file", PTR), ("process", PTR), ("thread", PTR), ("base", PTR), ("debug_offset", U32), ("debug_size", U32), ("tls", PTR), ("start", PTR), ("image_name", PTR), ("unicode", C.c_uint16)]


class LoadInfo(C.Structure):
    _fields_ = [("file", PTR), ("base", PTR), ("debug_offset", U32), ("debug_size", U32), ("image_name", PTR), ("unicode", C.c_uint16)]


class DebugUnion(C.Union):
    _fields_ = [("exception", ExceptionInfo), ("created", CreateInfo), ("loaded", LoadInfo), ("thread", PTR), ("exit_code", U32), ("opaque", C.c_byte * 160)]


class DebugEvent(C.Structure):
    _fields_ = [("code", U32), ("pid", U32), ("tid", U32), ("data", DebugUnion)]


def require(value, why):
    if not value:
        raise ValueError(why)


def layouts():
    require(C.sizeof(PTR) == 8, "x64 host required")
    require(C.sizeof(ExtendedLimit) == 144 and ExtendedLimit.job_memory.offset == 120, "JobObject layout")
    require(C.sizeof(Startup) == 104 and Startup.stdout.offset == 88, "STARTUPINFO layout")
    require(C.sizeof(Process) == 24, "PROCESS_INFORMATION layout")
    require(C.sizeof(ExceptionRecord) == 152 and C.sizeof(ExceptionInfo) == 160, "Exception layout")
    require(C.sizeof(DebugEvent) == 176 and DebugEvent.data.offset == 16, "DEBUG_EVENT layout")
    require(C.sizeof(LoadInfo) == 40 and LoadInfo.base.offset == 8, "LOAD_DLL layout")


def normalized(path):
    if path.startswith("\\\\?\\UNC\\"):
        path = "\\\\" + path[8:]
    elif path.startswith("\\\\?\\"):
        path = path[4:]
    return Path(path).resolve(strict=True)


def check_observations(test, stdout, stderr):
    # Native printf can interrupt Rust's `test NAME ... ok` line. Require the
    # exact selected start, one successful result, and its terminal standalone
    # ok (or uninterrupted ok), rather than assuming contiguous output.
    name = TESTS[test]
    require(len(re.findall(r"(?m)^test " + re.escape(name) + r" \.\.\. ", stdout)) == 1,
            "Missing/duplicate exact test start")
    summaries = re.findall(r"(?m)^test result: (.+)$", stdout)
    require(len(summaries) == 1 and re.fullmatch(
        r"ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in .+", summaries[0]),
        "Exact test did not pass once without ignores")
    require(re.search(r"(?m)^(?:test " + re.escape(name) + r" \.\.\. )?ok\s*$", stdout),
            "Missing selected test terminal ok")
    combined = re.sub(r"(?m)^test " + re.escape(name) + r" \.\.\. ", "", stdout) + "\n" + stderr
    releases = re.findall(r"(?m)^PHYSX_VALIDATION_RELEASE allocations=\d+ peak=\d+ live=(\d+)\s*$", combined)
    if test == "reject":
        require(not releases and "PHYSX_VALIDATION_BODY" not in combined,
                "Production rejection unexpectedly created native bodies/foundation")
    else:
        require(releases == ["0"] * 3, "Expected exactly three native live-zero releases")
        bodies = re.findall(r"(?m)^PHYSX_VALIDATION_BODY id=\d+ shapes=1 mass=2 inertia=\([^\n]+\) com=\([^\n]+\)\s*$", combined)
        values = re.findall(r"(?m)^PHYSX_VALIDATION_BODY_VALUES id=\d+ mass_wanted=2 mass_actual=2 mass_hex=[^\n]+$", combined)
        require(len(bodies) == len(values) == 3, "Missing/extra primitive mass/inertia/COM readbacks")
        for kind in range(3):
            require(combined.count("PROBE production geometry kind" + str(kind) + " teardown") == 1,
                    "Missing/duplicate primitive production observation")


def check_loaded_native(row, expected):
    # Called while the LOAD_DLL event still suspends this child, before its
    # DLL entry point can execute. Unknown OS DLLs are observations only.
    matching = [want for want in expected if Path(want["path"]).name.lower() == Path(row["path"]).name.lower()]
    if matching:
        require(len(matching) == 1 and row == matching[0], "Actual native DLL path/hash differs: " + row["path"])


def check_source_stability(plan):
    for row in plan["runner_sources"] + plan["current_test_sources"]:
        require(metadata(Path(row["path"])) == row, "Reviewed runner/test source drift: " + row["path"])


def prepare(args):
    layouts()
    candidate = args.candidate.resolve(strict=True)
    provenance = candidate / "provenance.json"
    require(digest(provenance) == args.provenance_sha256, "Reviewed provenance fingerprint differs")
    prov = json.loads(provenance.read_text(encoding="utf-8"))
    require(prov["licensed_source_pin"] == PIN and prov["guarded_validation"] is True, "Not the reviewed guarded BSD candidate")
    require(prov["changed_inputs"] == [] and prov["scope"] == "COMPILED_ONLY_NOT_ACCEPTED_OR_INSTALLED", "Unaccepted compiler provenance")
    bridge = (candidate / "mh_physx.dll").resolve(strict=True)
    require(bridge == Path(prov["output"]["path"]).resolve(strict=True), "Provenance output path differs")
    require(metadata(bridge) == prov["output"], "Guarded DLL fingerprint differs")
    pe64(bridge)
    # Recheck every compiled source/local dependency/original input, not just the DLL.
    for row in prov["inputs"]:
        path = Path(row["path"]).resolve(strict=True)
        require(path.is_file() and path.stat().st_size == row["size"] and digest(path) == row["sha256"], "Compiled input drift: " + str(path))
    exe, dlls = installed_files(args.game_dir)
    game = args.game_dir.resolve(strict=True)
    pvd = (game / "Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015/PxPvdSDK_x64.dll").resolve(strict=True)
    require(pvd.is_relative_to(game) and digest(pvd) == PVD_SHA256, "Original PVD keepalive differs")
    pe64(pvd)
    test = args.test_exe.resolve(strict=True)
    require(digest(test) == args.test_sha256, "Reviewed test executable fingerprint differs")
    pe64(test)
    command = [str(test), TESTS[args.test], "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    native = [bridge, *dlls, pvd]
    test_sources = [ROOT / p for p in ("core/crates/mh-physics/src/lib.rs", "core/crates/mh-physics/src/cooked.rs", "core/crates/mh-physics/Cargo.toml", "core/Cargo.lock")]
    return dict(scope="PREPARED_NOT_EXECUTED", test=args.test, command=command, candidate_provenance=metadata(provenance), native_inputs=[metadata(p) for p in native], original_exe=metadata(exe), compiled_inputs=prov["inputs"], test_exe=metadata(test), current_test_sources=[metadata(p) for p in test_sources], test_source_binding_limit="Current file hashes plus root's separate Cargo build receipt; executable filename alone is not source provenance", runner_sources=[metadata(HERE/n) for n in ("run_guarded.py", "build_bridge.py", "compatibility.py", "verify_callsites.py")], job_limit_bytes=LIMIT, timeout_seconds=TIMEOUT, required_free_commit_gib=4.5, environment={"MORDHAU_DIR":str(game), "MH_PHYSX_VALIDATION_BRIDGE":str(bridge)}, loaded_module_requirement="Actual child LOAD_DLL events; all six native paths+hashes required")


class Windows:
    def __init__(self):
        require(os.name == "nt", "Windows required")
        self.k = C.WinDLL("kernel32", use_last_error=True)
        self.ps = C.WinDLL("psapi", use_last_error=True)
        def bind(lib, name, restype, args):
            function = getattr(lib, name)
            function.restype, function.argtypes = restype, args
            return function
        self.memory = bind(self.k, "GlobalMemoryStatusEx", C.c_int, [C.POINTER(Memory)])
        self.create_job = bind(self.k, "CreateJobObjectW", PTR, [PTR, C.c_wchar_p])
        self.set_job = bind(self.k, "SetInformationJobObject", C.c_int, [PTR, C.c_int, PTR, U32])
        self.query_job = bind(self.k, "QueryInformationJobObject", C.c_int, [PTR, C.c_int, PTR, U32, PTR])
        self.assign = bind(self.k, "AssignProcessToJobObject", C.c_int, [PTR, PTR])
        self.kill_job = bind(self.k, "TerminateJobObject", C.c_int, [PTR, U32])
        self.create = bind(self.k, "CreateProcessW", C.c_int, [C.c_wchar_p, C.c_wchar_p, PTR, PTR, C.c_int, U32, PTR, C.c_wchar_p, C.POINTER(Startup), C.POINTER(Process)])
        self.resume = bind(self.k, "ResumeThread", U32, [PTR])
        self.kill = bind(self.k, "TerminateProcess", C.c_int, [PTR, U32])
        self.close = bind(self.k, "CloseHandle", C.c_int, [PTR])
        self.wait = bind(self.k, "WaitForDebugEvent", C.c_int, [C.POINTER(DebugEvent), U32])
        self.next = bind(self.k, "ContinueDebugEvent", C.c_int, [U32, U32, U32])
        self.path = bind(self.k, "GetFinalPathNameByHandleW", U32, [PTR, C.c_wchar_p, U32, U32])
        self.module_path = bind(self.ps, "GetModuleFileNameExW", U32, [PTR, PTR, C.c_wchar_p, U32])

    def ok(self, result, what):
        if not result:
            raise OSError(C.get_last_error(), what)

    def free_commit(self):
        m = Memory(); m.length = C.sizeof(m)
        self.ok(self.memory(C.byref(m)), "Memory guard unavailable")
        return m.available_page / 2**30

    def loaded_path(self, file, process, base):
        buffer = C.create_unicode_buffer(32768)
        size = self.path(file, buffer, len(buffer), 0) if file else 0
        if not (0 < size < len(buffer)):
            size = self.module_path(process, base, buffer, len(buffer))
        require(0 < size < len(buffer), "Actual loaded module path unavailable/truncated")
        return normalized(buffer.value)


def run(plan, out):
    import msvcrt
    win = Windows()
    initial = win.free_commit()
    require(initial >= 4.5, "Free commit below4.5GiB; no child started")
    job, child, resumed, exited = None, Process(), False, False
    loaded, events, failure = [], [], None
    started = time.monotonic()
    peak = None
    with (out/"stdout.log").open("xb") as stdout, (out/"stderr.log").open("xb") as stderr, open(os.devnull,"rb") as stdin, (out/"events.jsonl").open("x",encoding="utf-8") as journal:
        def event(value):
            value = dict(value, seconds=time.monotonic()-started)
            events.append(value); journal.write(json.dumps(value)+"\n"); journal.flush()
        try:
            check_source_stability(plan)
            event(dict(kind="reviewed_sources_stable_before_child"))
            job = win.create_job(None, None); win.ok(job, "CreateJobObject")
            limits = ExtendedLimit(); limits.basic.flags = 0x200 | 0x2000
            limits.job_memory = LIMIT  # total job commit, not a working-set hint
            win.ok(win.set_job(job, 9, C.byref(limits), C.sizeof(limits)), "SetInformationJobObject")
            startup = Startup(); startup.cb = C.sizeof(startup); startup.flags = 0x100
            handles = [msvcrt.get_osfhandle(f.fileno()) for f in (stdin,stdout,stderr)]
            for handle in handles: os.set_handle_inheritable(handle, True)
            startup.stdin, startup.stdout, startup.stderr = handles
            env = {k:v for k,v in os.environ.items() if not k.upper().startswith(("MH_", "MORDHAU_"))}
            env.update(plan["environment"])
            environment = C.create_unicode_buffer("\0".join(k+"="+v for k,v in sorted(env.items(),key=lambda row:row[0].upper()))+"\0\0")
            command = C.create_unicode_buffer(subprocess.list2cmdline(plan["command"]))
            # Only this new child is debugged. Suspension prevents entry before
            # assignment to the 1GiB, kill-on-close JobObject succeeds.
            flags = 0x2 | 0x4 | 0x400 | 0x08000000
            win.ok(win.create(plan["command"][0], command, None, None, True, flags, environment, str(out), C.byref(startup), C.byref(child)), "CreateProcess suspended")
            event(dict(kind="created_suspended",pid=child.pid))
            win.ok(win.assign(job,child.process), "AssignProcessToJobObject")
            require(win.resume(child.thread) != 0xffffffff, "ResumeThread failed")
            resumed = True; event(dict(kind="resumed_job_capped",pid=child.pid,limit_bytes=LIMIT))
            while not exited:
                require(time.monotonic()-started < TIMEOUT, "30s isolated test timeout")
                debug = DebugEvent()
                if not win.wait(C.byref(debug),50):
                    require(C.get_last_error() == 121, "WaitForDebugEvent failed")
                    continue
                require(debug.pid == child.pid, "Unexpected debuggee process")
                status = 0x10002
                try:
                    if debug.code == 3:
                        info = debug.data.created
                        actual = win.loaded_path(info.file,child.process,info.base)
                        require(actual == Path(plan["test_exe"]["path"]), "Actual test process image differs")
                        event(dict(kind="actual_test_image",image=metadata(actual)))
                        if info.file: win.close(info.file)
                        # The debugger receives additional handles. Keep only
                        # the distinct CreateProcess API handles we own.
                        for handle in (info.process,info.thread):
                            if handle and handle not in (child.process,child.thread): win.close(handle)
                    elif debug.code == 6:
                        info = debug.data.loaded
                        try:
                            actual = win.loaded_path(info.file,child.process,info.base)
                            row = metadata(actual); loaded.append(row)
                            event(dict(kind="actual_loaded_dll",module=row,base=hex(info.base or 0)))
                            check_loaded_native(row, plan["native_inputs"])
                        finally:
                            if info.file: win.close(info.file)
                    elif debug.code == 2:
                        if debug.data.thread: win.close(debug.data.thread)
                    elif debug.code == 1:
                        code = debug.data.exception.record.code
                        event(dict(kind="exception",code=hex(code),first_chance=debug.data.exception.first_chance))
                        # Continue the loader breakpoint; preserve normal app
                        # exception handling for every other exception.
                        if code != 0x80000003: status = 0x80010001
                    elif debug.code == 5:
                        exited = True; exit_code = debug.data.exit_code
                        event(dict(kind="exit",exit=exit_code))
                    elif debug.code == 9:
                        raise ValueError("Windows debugger RIP event")
                except BaseException:
                    # Never resume a failed image/DLL pin into its entry point.
                    win.kill(child.process,90)
                    if job: win.kill_job(job,90)
                    raise
                finally:
                    win.ok(win.next(debug.pid,debug.tid,status), "ContinueDebugEvent")
            require(exit_code == 0, "Isolated test process failed")
            for row in plan["native_inputs"]:
                matches = [actual for actual in loaded if Path(actual["path"]) == Path(row["path"])]
                require(matches and all(m == row for m in matches), "Required actual native DLL observation differs/missing: "+row["path"])
            # A selected exact lib-test must actually run one test. Zero-test
            # success and ignored/skipped results are not accepted observations.
            stdout.flush(); stderr.flush()
            text = (out/"stdout.log").read_text(encoding="utf-8",errors="replace")
            check_observations(plan["test"], text, (out/"stderr.log").read_text(encoding="utf-8",errors="replace"))
        except BaseException as error:
            failure = str(error); event(dict(kind="failure",reason=failure))
        finally:
            if child.process and not exited:
                win.kill(child.process,90)
                if job: win.kill_job(job,90)
                # Drain/continue owned child debug events briefly so process
                # termination cannot remain suspended at a debugger event.
                deadline = time.monotonic()+2
                while time.monotonic()<deadline:
                    debug=DebugEvent()
                    if win.wait(C.byref(debug),50):
                        if debug.code == 6 and debug.data.loaded.file: win.close(debug.data.loaded.file)
                        elif debug.code == 2 and debug.data.thread: win.close(debug.data.thread)
                        elif debug.code == 3:
                            info = debug.data.created
                            if info.file: win.close(info.file)
                            for handle in (info.process,info.thread):
                                if handle and handle not in (child.process,child.thread): win.close(handle)
                        win.next(debug.pid,debug.tid,0x10002)
                        if debug.code==5: break
            if job:
                usage=ExtendedLimit()
                if win.query_job(job,9,C.byref(usage),C.sizeof(usage),None):
                    peak=usage.peak_job
                    if peak > LIMIT: failure = failure or "Observed peak job commit exceeds cap"
                else: failure = failure or "Peak job commit query unavailable"
                win.close(job)  # kill-on-close is retained on every failure path
            if child.thread: win.close(child.thread)
            if child.process: win.close(child.process)
            try:
                check_source_stability(plan)
                event(dict(kind="reviewed_sources_stable_after_dispatch"))
            except BaseException as error:
                failure = failure or str(error)
                event(dict(kind="source_stability_failure",reason=str(error)))
    return dict(scope="ISOLATED_GUARDED_TEST_NOT_PRODUCTION_ACCEPTANCE",test=plan["test"],pid=child.pid,resumed=resumed,success=failure is None,failure=failure,seconds=time.monotonic()-started,free_commit_before_gib=initial,job_limit_bytes=LIMIT,peak_job_commit_bytes=peak,timeout_seconds=TIMEOUT,actual_loaded_dlls=loaded,stdout=metadata(out/"stdout.log"),stderr=metadata(out/"stderr.log"),journal=metadata(out/"events.jsonl"))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate",type=Path,required=True)
    parser.add_argument("--provenance-sha256",required=True)
    parser.add_argument("--test-exe",type=Path,required=True)
    parser.add_argument("--test-sha256",required=True)
    parser.add_argument("--game-dir",type=Path,required=True)
    parser.add_argument("--test",choices=TESTS,required=True)
    parser.add_argument("--output",type=Path,required=True,help="Fresh child of source build/physics")
    parser.add_argument("--run",action="store_true",help="Explicit root dispatch, after independent plan/source review")
    args=parser.parse_args()
    plan=prepare(args)
    output=args.output.resolve()
    require(output.is_relative_to((ROOT/"build/physics").resolve()) and output != (ROOT/"build/physics").resolve(), "Fixture output must be a fresh build/physics child")
    game=args.game_dir.resolve();require(not output.is_relative_to(game) and not game.is_relative_to(output), "Fixture output overlaps original install")
    output.mkdir(parents=True,exist_ok=False)
    save(output/"plan.json",plan)
    if args.run:
        try: receipt=run(plan,output)
        except BaseException as error:
            receipt=dict(scope="DISPATCH_REFUSED_OR_HOST_FAILURE",success=False,failure=str(error))
        save(output/"result.json",receipt)
        print(json.dumps(receipt,indent=2))
        if not receipt["success"]:raise SystemExit(1)
    else:
        print("Prepared only; no child or original DLL entry: "+str(output/"plan.json"))


if __name__=="__main__":
    main()
