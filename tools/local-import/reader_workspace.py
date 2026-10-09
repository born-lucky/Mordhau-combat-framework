"""Prepare an isolated source reader workspace under the user's external cache.

This stage currently permits only the settings writer, which does not consume any
unreviewed reconstructed constructors. It is not a full matrix importer.
"""
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path

SUPPORTED_KINDS=('settings',)

def prepare(game,cache,native):
    game,exe,pdb,pe=native.verify_install(game)
    game=Path(game).resolve();cache=Path(cache).resolve()
    root=Path(__file__).resolve().parents[2]
    if cache==game or cache.is_relative_to(game):raise ValueError('Reader output must be outside original installation')
    if cache==root or cache.is_relative_to(root) and cache.relative_to(root).parts[:1] not in [('build',),('cache',),('state',)]:
        raise ValueError('Generated records cannot be written into published source')
    source=Path(__file__).with_name('reader-source')
    files=sorted(source.rglob('*.gd'))
    if not files or not (source/'tools/gen_spec_src.gd').is_file():raise ValueError('Packaged source readers unavailable')
    data=[]
    for path in files:
        resolved=path.resolve(strict=True)
        if not resolved.is_relative_to(source.resolve()):raise ValueError('Source reader escaped packaged directory')
        raw=path.read_bytes();relative=path.relative_to(source)
        data.append((relative,raw))
    # Reader scripts initialize their named constants when loaded, even for a
    # writer that never constructs a combat object. Build those rows from this
    # verified PE into this isolated stage, without borrowing another cache.
    spec=importlib.util.spec_from_file_location('reader_source_constants',Path(__file__).with_name('source_constants.py'))
    constants=importlib.util.module_from_spec(spec);spec.loader.exec_module(constants)
    header=b'va\trdata_off\twidth\traw\tf32\tf64\ti32\ti64\tfuncs\n'
    raw_constants,recipes=constants.table(pe,header,source)
    cache.mkdir(parents=True,exist_ok=True)
    stage=cache/'stages'/('records-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    project=stage/'project';project.mkdir(parents=True)
    data_root=stage/'extract';(data_root/'native').mkdir(parents=True)
    with (data_root/'native/rdata.tsv').open('xb') as f:f.write(raw_constants)
    manifest=[]
    for relative,raw in data:
        destination=project/relative;destination.parent.mkdir(parents=True,exist_ok=True)
        with destination.open('xb') as f:f.write(raw)
        manifest.append({'source':str(relative).replace('\\','/'),'sha256':hashlib.sha256(raw).hexdigest()})
    project_text='\n'.join([
        'config_version=5','[application]','config/name="Mordhau local source reader"',
        '[rendering]','renderer/rendering_method="gl_compatibility"',
        '[mordhau]','data_backend="pak"','data_path='+json.dumps(str(data_root).replace('\\','/')),
        'game_dir='+json.dumps(str(game).replace('\\','/')),''])
    with (project/'project.godot').open('x',encoding='utf-8') as f:f.write(project_text)
    result={'stage':'source-reader-workspace','project':str(project),'output':str(project/'data_gen/spec_src'),
            'exe_sha1':native.EXE_SHA1,'pdb_sha1':native.PDB_SHA1,'source_files':manifest,
            'supported_kinds':list(SUPPORTED_KINDS),'complete':False,'runtime_ready':False,
            'source_constant_count':len(recipes),'source_constant_sha256':hashlib.sha256(raw_constants).hexdigest(),
            'limits':'Only settings allowed until fresh constructor consumer fields and other writer inputs are accepted.'}
    with (stage/'workspace.json').open('x',encoding='utf-8') as f:json.dump(result,f,indent=2)
    return result

def command(godot,workspace,kind):
    if kind not in SUPPORTED_KINDS:raise ValueError('Writer kind needs original-input and constructor review: '+kind)
    tool=Path(godot).resolve(strict=True)
    if not tool.is_file():raise ValueError('Godot executable unavailable')
    project=Path(workspace['project']).resolve(strict=True)
    return [str(tool),'--headless','--path',str(project),'--editor','--script','res://tools/gen_spec_src.gd','--','--only='+kind]
