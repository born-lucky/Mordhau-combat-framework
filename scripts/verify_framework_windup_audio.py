"""Check that withdrawing rejected foley restores native windup audio without adding events."""
from pathlib import Path
import argparse
import hashlib
import json
import math
import os
import subprocess
import time
import wave

ROOT = Path(__file__).resolve().parents[1]


def run(runtime, data, bank, output):
    output.mkdir(parents=True, exist_ok=True)
    config = output / "config"
    config.mkdir(exist_ok=True)
    (config / "Input.ini").write_text('[/Script/Mordhau.MordhauInput]\nActionMappings=(ActionName="Kick",Key=F)\n')
    (config / "GameUserSettings.ini").write_text('[/Script/Mordhau.MordhauGameUserSettings]\nResolutionSizeX=640\nResolutionSizeY=360\nFullscreenMode=2\nFrameRateLimit=60\nFieldOfView=93\n')
    lines = ["load_map TestLevel", "spawn 2", "ui match", "view 1p", "wait 8s"]
    # Stay at spawn separation: no hits, parries, flinches or deaths to confound the windup.
    for name, move in [("swing", 0), ("stab", 2)]:
        lines += [f"dump_state {output.as_posix()}/{name}-before", f"input 0 attack {move} 0", "wait 20",
                  f"dump_state {output.as_posix()}/{name}-windup", "wait 24",
                  f"dump_state {output.as_posix()}/{name}-release", "wait 100",
                  f"dump_state {output.as_posix()}/{name}-after"]
    lines += ["wait 20", "quit"]
    script = output / "validation.txt"
    script.write_text("\n".join(lines) + "\n")
    env = {k: v for k, v in os.environ.items() if not k.startswith(("MH_", "MORDHAU_"))}
    env.update(MORDHAU_LOCAL_DATA=str(data), MH_CONFIG_DIR=str(config), MORDHAU_GUS_INI=str(config / "GameUserSettings.ini"),
               MH_AUDIO_REPLACEMENTS=str(bank), MH_AUDIO="offline", MH_AUDIO_CAPTURE=str(output / "combat.wav"), MH_MEMLOG="0")
    command = [str(runtime), "--offscreen", "--sim", "mh", "--profile", "Brigand", "--script", str(script),
               "--frames", "1600", "--dt", "0.016666666666666666", "--fps", "60", "--seed", "739"]
    started = time.time()
    with (output / "stdout.log").open("w") as out, (output / "stderr.log").open("w") as err:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=out, stderr=err,
                                   creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            result = process.wait(timeout=120)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait(); raise
    assert result == 0, f"Runtime exited {result}"
    states = {}
    for name in ("swing", "stab"):
        for phase in ("before", "windup", "release", "after"):
            path = output / f"{name}-{phase}.json"
            assert path.exists() and path.stat().st_mtime >= started - .01, f"Missing/stale evidence: {path}"
            states[name, phase] = json.loads(path.read_text())
    capture = output / "combat.wav"
    assert capture.exists() and capture.stat().st_mtime >= started - .01, "Missing/stale mixer recording"
    with wave.open(str(capture)) as audio:
        assert audio.getnchannels() == 2 and audio.getframerate() == 48000
        assert any(audio.readframes(audio.getnframes())), "Silent mixer recording"
    return states


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, default=ROOT / "build/install-gate/debug/mordhau.exe")
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--bank", type=Path, default=ROOT / "build/play-session/config/audio")
    parser.add_argument("--output", type=Path, default=ROOT / "build/windup-audio-proof")
    args = parser.parse_args()
    runtime, data, bank, output = (p.resolve() for p in (args.runtime, args.data, args.bank, args.output))
    manifest = json.loads((bank / "bank.json").read_text())
    assert not manifest["roles"], "Rejected bank still has active replacements"
    native_bank = output / "original-only-bank"
    native_bank.mkdir(parents=True, exist_ok=True)
    (native_bank / "bank.json").write_text('{"version":1,"roles":{}}\n')
    runtime_digest = hashlib.sha256(runtime.read_bytes()).hexdigest()
    corrected = run(runtime, data, bank, output / "corrected")
    reference = run(runtime, data, native_bank, output / "original")
    receipt = {"runtime": str(runtime), "runtime_sha256": runtime_digest, "attacks": {},
               "bank_sha256": hashlib.sha256((bank / "bank.json").read_bytes()).hexdigest(),
               "scope": "Native windup PCM routing and swing/stab event timing; remaining replacements are not product-approved"}
    for name in ("swing", "stab"):
        before = corrected[name, "before"]["audio"]["plays"]
        windup = corrected[name, "windup"]["audio"]
        added = windup["plays"] - before
        rows = windup["rows"][:added]
        assert added > 0, f"{name}: native armor cue not exercised"
        assert len(rows) == added, f"{name}: log window too small"
        assert all(r["cue"].endswith("SC_NonSnappyArmorFoley") and r["sample_source"] == "original" for r in rows), rows
        end = corrected[name, "after"]
        original = reference[name, "after"]
        assert not end["audio"]["missing"], end["audio"]["missing"]
        assert all(source == "original" for source in end["audio"]["sample_counts"]), "Rejected bank still plays"
        assert end["audio"]["plays"] == original["audio"]["plays"], "Replacement added/removed sound events"
        a, b = end["audio"]["rows"], original["audio"]["rows"]
        # Asset startup may cost a different number of fixed frames in each run.
        # Compare times relative to attack start, not process start. Pose sampling
        # can differ by a few micrometres; distance/attenuation must still agree.
        anchor_a = min(r["t"] for r in a)
        anchor_b = min(r["t"] for r in b)
        assert len(a) == len(b), "Audio log lengths differ"
        for x, y in zip(a, b):
            for k, v in x.items():
                if k == "sample_source":
                    continue
                if k == "t":
                    assert abs((v - anchor_a) - (y[k] - anchor_b)) < 1e-6, "Cue timing relative to attack changed"
                elif isinstance(v, float):
                    assert math.isclose(v, y[k], rel_tol=1e-5, abs_tol=1e-5), f"Mixer multiplier changed: {k}: {v}, {y[k]}"
                else:
                    assert v == y[k], f"Native cue property changed: {k}"
        assert not any(r["sample_source"].startswith("framework:foley:") for r in a), "Rejected foley still plays"
        releases = [e["t"] for e in end["bridge"]["last_events"] if e.get("kind") == "release" and e.get("who") == "F0"]
        assert releases, f"{name}: attack did not release"
        release_time = releases[-1]
        whooshes = [r for r in a if "/Wooshes/" in r["cue"] and r["t"] >= corrected[name, "before"]["clocks"]["elapsed_world_secs"]]
        assert whooshes and all(r["t"] + .0001 >= release_time for r in whooshes), "Weapon whoosh starts during windup"
        receipt["attacks"][name] = {"windup_rows": rows, "release_time": release_time, "whoosh_times": [r["t"] for r in whooshes], "identical_native_event_timing": True}
    assert hashlib.sha256(runtime.read_bytes()).hexdigest() == runtime_digest, "Executable changed during comparison"
    (output / "routing-evidence.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
