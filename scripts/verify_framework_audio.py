"""Exercise real combat playback in the canonical private fork; retain sample provenance and PCM evidence."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import time
import wave

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, default=ROOT / "build/install-gate/debug/mordhau.exe")
    parser.add_argument("--data", type=Path, required=True, help="Owner's prepared private local game data")
    parser.add_argument("--bank", type=Path, default=ROOT / "build/play-session/config/audio")
    parser.add_argument("--output", type=Path, default=ROOT / "build/audio-proof")
    args = parser.parse_args()
    folder = args.output.resolve(); folder.mkdir(parents=True, exist_ok=True)
    config = folder / "config"; config.mkdir(exist_ok=True)
    (config / "Input.ini").write_text('[/Script/Mordhau.MordhauInput]\nActionMappings=(ActionName="Kick",Key=F)\n')
    (config / "GameUserSettings.ini").write_text('[/Script/Mordhau.MordhauGameUserSettings]\nResolutionSizeX=1024\nResolutionSizeY=576\nFullscreenMode=2\nFrameRateLimit=60\nFieldOfView=93\n')
    script = folder / "validation.txt"
    lines = ["load_map TestLevel", "spawn 2", "ui match", "view 1p", "wait 8s", "move 0 1 0", "wait 66", "move 0 0 0", "move 1 0 0 180", "wait 60",
        f"dump_state {folder.as_posix()}/before", "input 0 attack 0 0", "wait 45", "input 1 parry", "wait 100", f"dump_state {folder.as_posix()}/parry"]
    for n in range(5): lines += ["input 0 attack 0 0", "wait 120", f"dump_state {folder.as_posix()}/hit-{n}"]
    lines += ["real_input", "key_down KeyF", "wait 20", f"dump_state {folder.as_posix()}/kick-windup", f"screenshot_nowait {folder.as_posix()}/kick-first-person", "wait 15",
        f"dump_state {folder.as_posix()}/kick-release", f"screenshot_nowait {folder.as_posix()}/kick-release-first-person", "key_up KeyF", "wait 100", f"dump_state {folder.as_posix()}/after", "wait 20", "quit"]
    script.write_text("\n".join(lines) + "\n")
    env = {k:v for k,v in os.environ.items() if not k.startswith(("MH_", "MORDHAU_"))}
    env.update(MORDHAU_LOCAL_DATA=str(args.data.resolve()), MH_CONFIG_DIR=str(config), MORDHAU_GUS_INI=str(config / "GameUserSettings.ini"),
        MH_AUDIO_REPLACEMENTS=str(args.bank.resolve()), MH_AUDIO_CAPTURE=str(folder / "combat.wav"), MH_AUDIO="offline", MH_MEMLOG="0")
    command = [str(args.runtime.resolve()), "--offscreen", "--sim", "mh", "--profile", "Brigand", "--script", str(script), "--frames", "3000", "--dt", "0.016666666666666666", "--fps", "60", "--seed", "739"]
    start = time.time()
    with (folder / "stdout.log").open("w") as out, (folder / "stderr.log").open("w") as err:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=out, stderr=err, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try: result = process.wait(timeout=120)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait(); raise
    assert result == 0, f"Runtime exited {result}"
    paths = [folder / (name + suffix) for name,suffix in [("after", ".json"), ("parry", ".json"), ("kick-windup", ".json"), ("kick-first-person", ".png"), ("combat", ".wav")]]
    for path in paths: assert path.exists() and path.stat().st_mtime >= start - 0.01, f"Missing/stale evidence: {path}"
    after = json.loads((folder / "after.json").read_text())
    counts = after["audio"]["sample_counts"]
    roles = {name.split(":")[1] for name in counts if name.startswith("framework:")}
    assert {"swing", "parry", "flesh", "voice_effort", "voice_death"} <= roles, f"Combat replacement coverage missing: {roles}"
    assert counts.get("original", 0) == 0, f"Original samples remain in this combat test: {counts}"
    assert not after["audio"]["missing"], after["audio"]["missing"]
    kick = json.loads((folder / "kick-windup.json").read_text())
    player = kick["player"]["id"]
    assert any(f["id"] == player and f["state"] == "Attack:BP_KickMotion" for f in kick["sim"]["fighters"]), "Real F key did not start the native kick"
    with wave.open(str(folder / "combat.wav")) as audio:
        assert audio.getnchannels() == 2 and audio.getframerate() == 48000
        raw = audio.readframes(audio.getnframes())
        assert any(raw), "Mixer recording is silent"
        duration = audio.getnframes() / audio.getframerate()
    receipt = {"runtime": str(args.runtime.resolve()), "runtime_sha256": hashlib.sha256(args.runtime.read_bytes()).hexdigest(),
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "working_tree_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT).strip()),
        "script_sha256": hashlib.sha256(script.read_bytes()).hexdigest(), "bank_sha256": hashlib.sha256((args.bank / "bank.json").read_bytes()).hexdigest(),
        "exit": result, "elapsed_seconds": time.time() - start, "capture_seconds": duration, "sample_counts": counts,
        "unmapped_original_plays": counts.get("original", 0), "roles_observed": sorted(roles), "scope": "Private local-data combat audio validation; no full audio parity claim"}
    (folder / "acceptance.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))

if __name__ == "__main__": main()
