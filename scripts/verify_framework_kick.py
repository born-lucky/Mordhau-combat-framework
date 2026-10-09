#!/usr/bin/env python3
"""Exercise the actual F bind against a living target and retain first-person pose/contact evidence."""
from pathlib import Path
import argparse, hashlib, json, os, subprocess, time
ROOT = Path(__file__).resolve().parents[1]
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--runtime',type=Path,default=ROOT/'build/install-gate/debug/mordhau.exe')
    ap.add_argument('--data',type=Path,required=True)
    ap.add_argument('--output',type=Path,default=ROOT/'build/kick-proof')
    ap.add_argument('--move-frames',type=int,default=74)
    ap.add_argument('--view',default='1p',choices=['1p','3p'])
    ap.add_argument('--expect-miss',action='store_true')
    args=ap.parse_args();out=args.output.resolve();out.mkdir(parents=True,exist_ok=True)
    cfg=out/'config';cfg.mkdir(exist_ok=True)
    (cfg/'Input.ini').write_text('[/Script/Mordhau.MordhauInput]\nActionMappings=(ActionName="Kick",Key=F)\n')
    (cfg/'GameUserSettings.ini').write_text('[/Script/Mordhau.MordhauGameUserSettings]\nResolutionSizeX=1280\nResolutionSizeY=720\nFullscreenMode=2\nFrameRateLimit=60\nFieldOfView=93\n')
    lines=['load_map TestLevel','spawn 2','ui match','view '+args.view,'wait 8s','move 0 1 0',f'wait {args.move_frames}','move 0 0 0','move 1 0 0 180','wait 60',f'dump_state {out.as_posix()}/before','real_input','key_down KeyF','wait 1','key_up KeyF']
    last=1
    for frame in [10,20,27,33,40,50,70]:
        lines += [f'wait {frame-last}',f'dump_state {out.as_posix()}/phase-{frame}',f'screenshot_nowait {out.as_posix()}/phase-{frame}']
        last=frame
    lines += ['wait 120',f'dump_state {out.as_posix()}/after','wait 10','quit']
    script=out/'kick.txt';script.write_text('\n'.join(lines)+'\n')
    env={k:v for k,v in os.environ.items() if not k.startswith(('MH_','MORDHAU_'))}
    env.update(MORDHAU_LOCAL_DATA=str(args.data.resolve()),MH_CONFIG_DIR=str(cfg),MORDHAU_GUS_INI=str(cfg/'GameUserSettings.ini'),MH_AUDIO='offline',MH_MEMLOG='0',MH_AUDIO_REPLACEMENTS=str(ROOT/'build/play-session/config/audio'))
    cmd=[str(args.runtime.resolve()),'--offscreen','--sim','mh','--profile','Brigand','--script',str(script),'--frames','2000','--dt','0.016666666666666666','--fps','60','--seed','739']
    start=time.time()
    with (out/'stdout.log').open('w') as stdout,(out/'stderr.log').open('w') as stderr:
        subprocess.run(cmd,cwd=ROOT,env=env,stdout=stdout,stderr=stderr,timeout=150,check=True,creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0)
    before=json.loads((out/'before.json').read_text());after=json.loads((out/'after.json').read_text())
    player=before['player']['id'];target=next(f for f in before['sim']['fighters'] if f['id']!=player and f['health']>0)
    final=next(f for f in after['sim']['fighters'] if f['id']==target['id'])
    frames=[json.loads((out/f'phase-{f}.json').read_text()) for f in [10,20,27,33,40,50,70]]
    assert any(f['id']==player and f['state']=='Attack:BP_KickMotion' for x in frames for f in x['sim']['fighters']), 'F did not start native kick'
    events=[e for x in frames+[after] for e in x['bridge']['last_events']]
    kicks=[e for e in events if e.get('kind')=='hit' and 'Kick' in e.get('weapon_class','')]
    result={'runtime':str(args.runtime.resolve()),'runtime_sha256':hashlib.sha256(args.runtime.read_bytes()).hexdigest(),'view':args.view,'player':player,'target':target['id'],'health_before':target['health'],'health_after':final['health'],'kick_hits':kicks,'foot_probes':[{ 'frame':frame,'bones':[(n,p) for n,p in x['rig'].get('fp_probe',[]) if 'Foot' in n or 'Leg' in n]} for frame,x in zip([10,20,27,33,40,50,70],frames)],'elapsed_seconds':time.time()-start}
    (out/'acceptance.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))
    if args.expect_miss:
        assert not kicks and final['health']==target['health'], 'Out-of-range kick unexpectedly damaged the target'
    else:
        assert kicks and final['health']<target['health'], 'Kick animation started but no kick damage reached the living target'
if __name__=='__main__':main()
