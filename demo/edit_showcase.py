"""Verify actual captured state, then encode captioned original runtime frames.
No music, synthetic poses, optical flow, speed changes, source-data bundling or uploads.
"""
from pathlib import Path
import argparse,hashlib,json,subprocess
from showcase import load,js,require,sha,FPS

def verify(plan_path):
    plan=load(plan_path);rows=[]
    for scene in plan['scenes']:
        case=plan_path.parent/scene['runtime_case'];receipt=load(case/'process.json')if(case/'process.json').is_file()else None
        if receipt is None:rows.append({'id':scene['id'],'status':'NOT_CAPTURED'});continue
        try:
            require(receipt['exit_code']==0 and receipt['runtime_sha256']==plan['runtime_sha256']and receipt['bridge_sha256']==plan['bridge_sha256'],'Actual runtime/native process identity failed')
            states=[]
            for i in range(scene['frames']):
                prefix=Path(scene['prefix']+f'_{i:04}');image=prefix.with_suffix('.png');state=prefix.with_suffix('.json')
                require(image.is_file()and state.is_file(),'Missing actual frame/state '+str(prefix));states.append(load(state))
            require(all(s['sim']['backend']=='mh-sim'and s['mode']=='offscreen'and s['step_per_frame']for s in states),'Non-combat/stub capture')
            for a,b in zip(states,states[1:]):
                require(b['frame']-a['frame']==4,'Output cadence is not exactly4 simulation frames')
                require(abs(b['clocks']['elapsed_real_secs']-a['clocks']['elapsed_real_secs']-1/FPS)<1e-6,'Output clock pacing changed')
            fighter=[s['sim']['debug']['snapshot']['f']['F0']for s in states]
            attack=[f for f in fighter if f['kind']=='Attack'];detail={'observed_weapons':sorted({f['weapon']for f in fighter}),'actual_source_frames':scene['frames']}
            if scene['kind']=='attack':require(bool(attack),'No actual attack');detail['moves']=sorted({f['move']for f in attack});detail['types']=sorted({f['type']for f in attack})
            if scene['id']in['right_strike','left_strike','right_stab','left_stab','alternate_strike','alternate_stab','blade_traces','third_person']:
                require(fighter[-1]['kind']=='Idle','Attack recovery not complete at cut')
            if scene['id'].startswith('alternate_'):require(any(f['alt']for f in fighter),'Alternate grip not actually selected')
            if scene['id']=='blade_traces':require(any(s['tracers']['visible_lines']>0 for s in states),'No actual sampled tracer lines')
            if scene['id']=='parry_geometry':require(any(f['kind']=='Parry'for f in fighter),'No actual parry motion');require(any(s['development']['visualize_block_collider']for s in states),'Debug box gate not active')
            start=states[0]['sim']['debug']['snapshot']['t'];end=states[-1]['sim']['debug']['snapshot']['t']
            events=[e for s in states for e in s['bridge']['last_events']if isinstance(e.get('t'),(int,float))and start<=e['t']<=end]
            if scene['id']=='parry_contact':require(any(e.get('kind')=='parry'for e in events),'No actual parry contact');detail['scene_events']=list({json.dumps(e,sort_keys=True):e for e in events}.values())
            if scene['id']=='first_contact':require(any(e.get('kind')=='hit'and e.get('attacker')=='F0'and e.get('victim')=='F1'for e in events),'No actual melee impact');detail['weapon_blood_hits']=max(s['weapon_blood']['hits']for s in states);detail['scene_hits']=list({json.dumps(e,sort_keys=True):e for e in events if e.get('kind')=='hit'}.values())
            if scene['id']=='death_ragdoll':
                require(any(e.get('kind')=='died'and e.get('who')=='F1'for e in events),'No actual target death in this scene')
                rag=[s['sim']['debug']['snapshot'].get('ragdolls')for s in states];rag=[r for r in rag if r]
                require(any(any(c['bodies']>0 and c['weight']>0 for c in r['corpses'])for r in rag),'No native physical ragdoll bodies')
                require(all(not r['errors']for r in rag),'Native corpse physics diagnostic errors');detail['ragdoll_max_bodies']=max(c['bodies']for r in rag for c in r['corpses'])
            if scene['id']=='wall_stop':require(any(e.get('kind')=='world_hit'for e in events),'No actual world contact');require(any(f['kind']=='Blocked'for f in fighter),'No actual wall-stop state')
            rows.append({'id':scene['id'],'status':'NUMERIC_PASS_VISUAL_REVIEW_PENDING','detail':detail,
                'frame_sha256':{Path(scene['prefix']+f'_{i:04}.png').name:sha(Path(scene['prefix']+f'_{i:04}.png'))for i in [0,scene['frames']//3,scene['frames']*2//3,scene['frames']-1]}})
        except Exception as e:rows.append({'id':scene['id'],'status':'REJECT','reason':str(e)})
    result={'scope':plan['scope'],'plan_sha256':sha(plan_path),'scenes':rows,'visual_acceptance':'PENDING actual source/frame/movie review','music':None}
    js(plan_path.parent/'frame-state-review.json',result);return result
def encode(plan_path,ids=None):
    plan=load(plan_path);review=verify(plan_path);accepted={r['id']for r in review['scenes']if r['status']=='NUMERIC_PASS_VISUAL_REVIEW_PENDING'}
    if ids:require(set(ids)<=accepted,'A requested scene lacks actual state proof');accepted&=set(ids)
    folder=plan_path.parent/'edited';folder.mkdir(exist_ok=True);scenes=[s for s in plan['scenes']if s['id']in accepted];require(scenes,'No captured/proved scenes')
    font='C\\:/Windows/Fonts/segoeui.ttf';clips=[]
    for s in scenes:
        caption=folder/(s['id']+'-caption.txt');caption.write_text(s['caption'],encoding='utf-8');out=folder/(s['id']+'.mp4')
        require(not out.exists(),'Preserve prior encode; use a fresh plan/output directory')
        vf="pad=1280:800:0:0:color=0x111723,drawtext=fontfile='"+font+"':textfile='"+caption.name+"':fontcolor=white:fontsize=25:x=24:y=737,drawtext=fontfile='"+font+"':text='Rust + Bevy  |  version "+plan['source_commit'][:7]+"':fontcolor=0x9aaec4:fontsize=16:x=24:y=774,drawtext=fontfile='"+font+"':text='MORDHAU content - Triternion':fontcolor=0x9aaec4:fontsize=16:x=w-tw-24:y=774"
        args=['ffmpeg','-hide_banner','-loglevel','error','-threads','2','-framerate',str(FPS),'-start_number','0','-i',s['prefix']+'_%04d.png','-frames:v',str(s['frames']),'-vf',vf,'-an','-c:v','libx264','-threads','2','-filter_threads','1','-preset','fast','-crf','18','-pix_fmt','yuv420p','-movflags','+faststart',str(out)]
        subprocess.run(args,cwd=str(folder),check=True);clips.append({'id':s['id'],'path':str(out),'sha256':sha(out),'frames':s['frames'],'caption':s['caption']});print('ENCODED',s['id'],flush=True)
    concat=folder/'clips.txt';concat.write_text('\n'.join("file '"+Path(c['path']).as_posix()+"'"for c in clips)+'\n',encoding='utf-8')
    final=plan_path.parent/'Mordhau-Rust-combat-baseline-demo.mp4';require(not final.exists(),'Final already exists; preserve first output')
    subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-f','concat','-safe','0','-i',str(concat),'-c','copy','-an','-movflags','+faststart',str(final)],check=True)
    probe=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_streams','-show_format','-of','json',str(final)],text=True,encoding='utf-8'))
    subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-threads','2','-i',str(final),'-f','null','-'],check=True)
    receipt={'scope':plan['scope'],'output':str(final),'sha256':sha(final),'source_commit':plan['source_commit'],'runtime_sha256':plan['runtime_sha256'],'native_bridge_sha256':plan['bridge_sha256'],'bridge_identity_scope':'Pinned prelaunch bridge file; prior completed captures did not observe loaded modules',
        'clips':clips,'excluded_scenes':[r for r in review['scenes']if r['id']not in accepted],'probe':probe,'full_decode':'PASS','visual_acceptance':'PENDING independent actual review',
        'music':None,'audio':'No audio stream; offscreen source remained silent','pacing':'Actual30fps captures at1x; no speed adjustment or pose synthesis','new_fork_ui_shown':False}
    js(plan_path.parent/'video-receipt.json',receipt);print(final,flush=True)
def main():
    p=argparse.ArgumentParser();p.add_argument('action',choices=['verify','encode']);p.add_argument('--plan',type=Path,required=True);p.add_argument('--ids',nargs='*');a=p.parse_args()
    if a.action=='verify':print(json.dumps(verify(a.plan),indent=2))
    else:encode(a.plan,a.ids)
if __name__=='__main__':main()
