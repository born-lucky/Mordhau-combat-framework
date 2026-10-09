"""Prepare/capture/encode an honest captioned baseline demo; no build or installation.
Source recipes contain input commands only. Images/video/receipts live in ignored build/.
"""
from pathlib import Path
import argparse,ctypes as C,datetime,hashlib,json,os,re,shutil,subprocess,time,uuid

ROOT=Path(__file__).resolve().parents[1]
FPS=30;STEPS=4;DT=1/120
def sha(path):
    h=hashlib.sha256()
    with path.open('rb')as f:
        for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
    return h.hexdigest()
def js(path,value):path.write_text(json.dumps(value,indent=2)+'\n',encoding='utf-8')
def require(ok,message):
    if not ok:raise RuntimeError(message)
def load(path):return json.loads(path.read_text(encoding='utf-8'))
def memory():
    class Status(C.Structure):_fields_=[('length',C.c_uint32),('load',C.c_uint32)]+[(s,C.c_uint64)for s in ['total_physical','available_physical','total_commit','available_commit','total_virtual','available_virtual','extended_virtual']]
    s=Status();s.length=C.sizeof(s);f=C.WinDLL('kernel32',use_last_error=True).GlobalMemoryStatusEx;f.argtypes=[C.POINTER(Status)];f.restype=C.c_int
    require(f(C.byref(s)),'Windows memory status unavailable')
    return{'free_commit_gib':s.available_commit/2**30,'free_physical_gib':s.available_physical/2**30}
def loaded_modules(pid):
    """Observe only our launched child's loaded DLL paths; never write process memory."""
    from ctypes import wintypes as W
    k=C.WinDLL('kernel32',use_last_error=True);a=C.WinDLL('psapi',use_last_error=True)
    k.OpenProcess.argtypes=[W.DWORD,W.BOOL,W.DWORD];k.OpenProcess.restype=W.HANDLE
    k.CloseHandle.argtypes=[W.HANDLE];k.CloseHandle.restype=W.BOOL
    a.EnumProcessModulesEx.argtypes=[W.HANDLE,C.POINTER(W.HMODULE),W.DWORD,C.POINTER(W.DWORD),W.DWORD];a.EnumProcessModulesEx.restype=W.BOOL
    a.GetModuleFileNameExW.argtypes=[W.HANDLE,W.HMODULE,W.LPWSTR,W.DWORD];a.GetModuleFileNameExW.restype=W.DWORD
    h=k.OpenProcess(0x410,False,pid)
    if not h:return {'error':'OpenProcess read-only failed','winerror':C.get_last_error()}
    try:
        mods=(W.HMODULE*2048)();needed=W.DWORD()
        if not a.EnumProcessModulesEx(h,mods,C.sizeof(mods),C.byref(needed),3):return {'error':'EnumProcessModulesEx failed','winerror':C.get_last_error()}
        paths=[]
        for m in mods[:min(needed.value//C.sizeof(W.HMODULE),2048)]:
            buf=C.create_unicode_buffer(32768)
            if a.GetModuleFileNameExW(h,m,buf,len(buf)):
                path=Path(buf.value)
                if path.name.lower()=='mh_physx.dll'or path.name.lower().startswith('physx'):
                    paths.append({'path':str(path),'sha256':sha(path)})
        return {'pid':pid,'observed_utc':datetime.datetime.now(datetime.UTC).isoformat(),'physics_modules':paths}
    finally:k.CloseHandle(h)

def expand(recipe,folder,case):
    lines=[];scenes=[]
    def capture(kind,args):
        parts=args.split();sid=parts[0];seconds=float(parts[1]);events={}
        if kind=='attack':label=' '.join(parts[4:]);events[6]=['input 0 attack '+parts[2]+' '+parts[3]]
        elif kind=='switch':label=' '.join(parts[2:]);events[6]=['input 0 mode']
        elif kind=='parry':label=' '.join(parts[2:]);events[6]=['input 0 parry']
        elif kind=='duelparry':label=' '.join(parts[2:]);events[6]=['input 1 attack 0 0'];events[30]=['input 0 parry']
        else:label=' '.join(parts[2:])
        n=round(seconds*FPS);prefix=(folder/'frames'/sid).as_posix();(folder/'frames').mkdir(exist_ok=True)
        implicit=lambda cs:sum(1 for c in cs if c.startswith('input '))
        for i in range(n):
            lines.extend(events.get(i,[]))
            name=prefix+f'_{i:04}'
            lines.extend(['dump_state '+name,'screenshot_nowait '+name])
            if i<n-1:lines.append('wait '+str(STEPS-implicit(events.get(i+1,[]))))
        lines.extend(['wait 4','dump_state '+(folder/(sid+'_after')).as_posix()])
        scenes.append({'id':sid,'kind':kind,'caption':label,'frames':n,'seconds':n/FPS,'prefix':prefix,
            'events_output_frames':events,'runtime_case':case,'source_frame_mode':'Actual 120Hz simulation; captured every4 frames; no interpolation/retiming'})
    for raw in recipe.splitlines():
        m=re.match(r'# @([a-z]+) (.+)',raw)
        if m:capture(m[1],m[2])
        else:lines.append(raw)
    return '\n'.join(lines)+'\n',scenes
def preflight(runtime,data_root,bridge,version_manifest,wall_oracle=None):
    runtime=runtime.resolve(strict=True);data_root=data_root.resolve(strict=True);bridge=bridge.resolve(strict=True);version_manifest=version_manifest.resolve(strict=True)
    require(runtime.is_file()and bridge.is_file()and data_root.is_dir(),'Explicit runtime/bridge files and data-root directory required')
    v=load(version_manifest);require(v.get('schema_version')==1,'Version manifest schema_version must be1')
    for key,n in [('source_commit',40),('runtime_sha256',64),('bridge_sha256',64)]:
        require(isinstance(v.get(key),str)and re.fullmatch('[0-9a-fA-F]{'+str(n)+'}',v[key])is not None,'Manifest '+key+' must be a complete hexadecimal identity')
    require(sha(runtime)==v['runtime_sha256'].lower(),'Runtime differs from explicit version manifest')
    require(sha(bridge)==v['bridge_sha256'].lower(),'Bridge differs from explicit version manifest')
    expected=(data_root/'state/physics/mh_physx.dll').resolve(strict=True)
    require(bridge==expected,'This runtime loads data-root/state/physics/mh_physx.dll; --bridge must identify that exact file')
    wall=None
    if wall_oracle:
        wall=wall_oracle.resolve(strict=True);require(wall.is_file(),'Explicit wall oracle is not a file')
        require(any(line.startswith('dump_state wall_strike_before')for line in wall.read_text(encoding='utf-8').splitlines()),'Wall oracle lacks the reviewed strike-start marker')
    return {'schema_version':1,'data_root':str(data_root),'runtime':str(runtime),'bridge':str(bridge),
        'runtime_sha256':v['runtime_sha256'].lower(),'bridge_sha256':v['bridge_sha256'].lower(),'source_commit':v['source_commit'].lower(),
        'version_manifest':str(version_manifest),'version_manifest_sha256':sha(version_manifest),'version_manifest_contents':v,
        'wall_oracle':str(wall)if wall else None,'scope':v.get('scope','Explicit versioned Rust + Bevy combat runtime; setup UI not represented')}

def prepare(identity):
    folder=ROOT/'build/showcase'/datetime.datetime.now(datetime.UTC).strftime('%Y%m%dT%H%M%SZ');folder.mkdir(parents=True,exist_ok=False)
    cases=[];scenes=[]
    for name,profile in [('combat-baseline','Knight'),('melee-contact','Brigand')]:
        case=folder/name;case.mkdir();source=ROOT/'demo'/('capture-'+name+'.txt');recipe=source.read_text(encoding='utf-8')
        script,clips=expand(recipe,case,name);path=case/('showcase-'+name+'-'+uuid.uuid4().hex[:8]+'.txt');path.write_text(script,encoding='utf-8');scenes+=clips
        cases.append({'id':name,'profile':profile,'script':str(path),'recipe':str(source),'recipe_sha256':sha(source)})
    # Placement remains an explicitly supplied local oracle, never a private-path fallback.
    if identity['wall_oracle']:
        wall_source=Path(identity['wall_oracle']);source=wall_source.read_text(encoding='utf-8').splitlines();marker=next(i for i,s in enumerate(source)if s.startswith('dump_state wall_strike_before'))
        wall_recipe='\n'.join(source[:marker])+'\n# @attack wall_stop 4 0 0 Environment collision and hit-stop\nwait 20\nquit\n'
        case=folder/'wall';case.mkdir();script,clips=expand(wall_recipe,case,'wall');path=case/('showcase-wall-'+uuid.uuid4().hex[:8]+'.txt');path.write_text(script,encoding='utf-8');scenes+=clips
        cases.append({'id':'wall','profile':'Knight','script':str(path),'external_oracle':str(wall_source),'external_oracle_sha256':sha(wall_source)})
    plan={**identity,'cases':cases,'scenes':scenes,'fps':FPS,'dt':DT,'minimum_free_commit_gib':5,'music':None,'footage_in_git':False,'original_data':'Read outside source repository'}
    js(folder/'plan.json',plan);print(folder/'plan.json',flush=True);return folder/'plan.json'

def record(plan_path,case_id):
    plan=load(plan_path);folder=plan_path.parent;case=next(c for c in plan['cases']if c['id']==case_id);case_dir=folder/case_id
    require(not(case_dir/'process.json').exists(),'Preserve previous capture; prepare a new output folder')
    runtime=Path(plan['runtime']);bridge=Path(plan['bridge']);require('data_root'in plan and 'version_manifest_contents'in plan,'Legacy recorded plan is preserved evidence; fresh capture requires explicit portable prepare')
    require(sha(runtime)==plan['runtime_sha256']and sha(bridge)==plan['bridge_sha256'],'Explicit runtime/native bridge drift')
    data_root=Path(plan['data_root']);require(bridge.resolve()==(data_root/'state/physics/mh_physx.dll').resolve(),'Bridge is not the actual runtime-selected data-root file')
    before=memory();require(before['free_commit_gib']>=5,'Free commit is below root-approved5GiB launch guard: '+str(before))
    settings=case_dir/'isolated-config';settings.mkdir(exist_ok=True);timings=case_dir/'stock-timings.json';js(timings,{'schema_version':1})
    env=os.environ.copy();cleared={k:env.pop(k)for k in list(env)if k.startswith('MH_')or k in ['MORDHAU_EXTRACT','MORDHAU_INPUT_INI']}
    env.update({'MORDHAU_REPO':str(data_root),'MH_CONFIG_DIR':str(settings),'MORDHAU_GUS_INI':str(settings/'GameUserSettings.ini'),
        'MORDHAU_TIMINGS_FILE':str(timings),'MH_AUDIO':'offline'})
    args=[str(runtime),'--offscreen','--sim','mh','--profile',case['profile'],'--bots','0','--fps','120','--dt',str(DT),'--step','frame','--seed','0','--frames','14000','--script',case['script']]
    receipt={'scope':plan['scope'],'command':args,'runtime_sha256':sha(runtime),'bridge_sha256':sha(bridge),'bridge_identity_scope':'Prelaunch on-disk SHA-256 pin; not a loaded-module observation','cleared_inherited_overrides':cleared,'game_install_env':env.get('MORDHAU_DIR'),'source_commit':plan['source_commit'],'memory_before':before,'started_utc':datetime.datetime.now(datetime.UTC).isoformat(),'environment_overrides':{k:env[k]for k in ['MORDHAU_REPO','MH_CONFIG_DIR','MORDHAU_GUS_INI','MORDHAU_TIMINGS_FILE','MH_AUDIO']}}
    with(case_dir/'stdout.log').open('x',encoding='utf-8')as stdout,(case_dir/'stderr.log').open('x',encoding='utf-8')as stderr:
        proc=subprocess.Popen(args,cwd=str(data_root),env=env,stdout=stdout,stderr=stderr,creationflags=subprocess.CREATE_NO_WINDOW)
        receipt['pid']=proc.pid;js(case_dir/'process-running.json',receipt);print('CAPTURE PID',proc.pid,case_id,before,flush=True)
        start=time.monotonic();next_report=start+20
        while proc.poll()is None:
            time.sleep(1)
            if not receipt.get('loaded_physics_modules'):
                observation=loaded_modules(proc.pid)
                if observation.get('physics_modules'):
                    receipt['loaded_physics_modules']=observation;js(case_dir/'loaded-modules.json',observation)
            if time.monotonic()>=next_report:
                print('CAPTURE RUNNING',case_id,'seconds',round(time.monotonic()-start),'pngs',len(list((case_dir/'frames').glob('*.png'))),memory(),flush=True);next_report+=20
            if time.monotonic()-start>600:
                proc.terminate();proc.wait();receipt['timeout']=True;break
        receipt['exit_code']=proc.returncode
    receipt['memory_after']=memory();receipt['ended_utc']=datetime.datetime.now(datetime.UTC).isoformat()
    evidence=list((data_root/'state/runtime_evidence').glob('*/'+ '*-'+Path(case['script']).stem));require(len(evidence)==1,'Capture summary directory identity ambiguous')
    receipt['runtime_evidence']=str(evidence[0]);receipt['summary_sha256']=sha(evidence[0]/'summary.json')if(evidence[0]/'summary.json').exists()else None
    receipt['script_sha256']=sha(Path(case['script']));receipt['isolated_timings_sha256']=sha(timings);js(case_dir/'process.json',receipt)
    require(proc.returncode==0,'Actual runtime child failed: '+str(proc.returncode));require(load(evidence[0]/'summary.json')['ok'],'Runtime script summary reports failure')
    print('CAPTURE DONE',case_dir/'process.json',flush=True)
def main():
    p=argparse.ArgumentParser();sub=p.add_subparsers(dest='action',required=True)
    for action in ['preflight','prepare']:
        q=sub.add_parser(action)
        for name in ['runtime','data-root','bridge','version-manifest']:q.add_argument('--'+name,type=Path,required=True)
        q.add_argument('--wall-oracle',type=Path)
    q=sub.add_parser('record');q.add_argument('--plan',type=Path,required=True);q.add_argument('--case',required=True)
    a=p.parse_args()
    if a.action=='record':record(a.plan,a.case)
    else:
        identity=preflight(a.runtime,a.data_root,a.bridge,a.version_manifest,a.wall_oracle)
        if a.action=='preflight':print(json.dumps(identity,indent=2))
        else:prepare(identity)
if __name__=='__main__':main()
