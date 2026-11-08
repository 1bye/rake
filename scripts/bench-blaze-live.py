#!/usr/bin/env python3
"""One JSON config; compare pinned and lab Blaze on an identical fixture, locally."""
from dataclasses import dataclass, field
from pathlib import Path
import json, os, re, shutil, statistics, subprocess, sys, threading, time

@dataclass(frozen=True)
class Input:
    rake_root: str
    cli: str
    evidence: str
    fixture: str
    mode: str = "pinned"
    archive: str = ""
    environment: dict[str, str] = field(default_factory=dict)
    trials: int = 6
    clean_install: bool = True
    cold_build: bool = True
    manual_start: bool = False
    editor_pid: int = 0
    windowctl: str = "/private/tmp/blaze-timer-windowctl"

def main(v):
    root=Path(v.rake_root).resolve(); evidence=Path(v.evidence).resolve(); evidence.mkdir(parents=True,exist_ok=True)
    if (evidence/'supervisor.jsonl').exists() or (evidence/'start').exists():
        raise SystemExit('Choose a fresh evidence directory; keep target/store paths in environment to reuse caches')
    if v.clean_install and (Path.home()/'.rake').exists(): raise SystemExit('Preserve ~/.rake before clean-install acceptance')
    assert v.mode in {'pinned','lab'} and v.trials>0
    sys.path.insert(0,str(root/'crates/rake-cli/scripts'))
    from dev import exchange
    source=root/'examples/view/src/lib.rs'; original=source.read_bytes(); baseline=Path(v.fixture).read_text()
    assert 'Live text base' in baseline and 'Helper base' in baseline
    env={k:x for k,x in os.environ.items() if not k.startswith(('BLAZE_','RAKE_'))}
    env.update(CARGO_TARGET_DIR=str(evidence/'target'),CARGO_BUILD_JOBS='3',BLAZE_STATS='1')
    env.update(v.environment)
    if v.mode=='pinned':
        if v.archive:env['BLAZE_ARCHIVE']=str(Path(v.archive).resolve())
        else:env.pop('BLAZE_ARCHIVE',None)
        tools=evidence/'path-bin';tools.mkdir(exist_ok=True)
        for name in ['cargo','rustc','rustdoc','rustup']:
            found=shutil.which(name,path=env['PATH'])
            if found and not (tools/name).exists():(tools/name).symlink_to(found)
        env['PATH']=os.pathsep.join([str(tools),*(d for d in env['PATH'].split(os.pathsep) if d and not Path(d,'blaze').exists())])
        assert shutil.which('blaze',path=env['PATH']) is None
    pin=__import__('tomllib').loads((root/'crates/rake-cli/blaze-pin.toml').read_text())
    installed=Path.home()/'.rake/blaze'/pin['version']; home=installed/'runtime' if v.mode=='pinned' else Path(env['BLAZE_HOME'])
    if len(os.fsencode(home/'workers'/('0'*16)/'sock')) >= 104:
        raise SystemExit('BLAZE_HOME is too long for a macOS worker socket; use a shorter runtime directory')
    compiler=[str(installed/'blaze-engine'),'cargo'] if v.mode=='pinned' else ['blaze']
    cmd=[v.cli,'dev',*(['--live'] if v.mode=='pinned' else ['--compiler','blaze']),'--manifest-path',str(root/'examples/view/Cargo.toml'),'--bin','rake-view','--json','--no-open']
    shared={'session':None,'ready':False,'failure':None}; rows=[]; child=None; start=time.monotonic(); thread=None
    def wait(fn,timeout=60):
        end=time.monotonic()+timeout;last=None
        while time.monotonic()<end:
            try:
                result=fn()
                if result:return result
            except (OSError,KeyError,RuntimeError,StopIteration,ValueError) as e:last=e
            if child and child.poll() is not None:raise RuntimeError('supervisor exited '+str(child.returncode))
            time.sleep(.01)
        raise TimeoutError(str(last))
    def status():return json.loads((shared['session']/'status.json').read_text())
    def instance():
        pid=status()['runtime_pid']
        return next(x for p in (shared['session']/'control').glob('*.json') if (x:=json.loads(p.read_text()))['pid']==pid)
    def query(name,**kw):return exchange(instance(),'query',dict(type=name,**kw))
    def texts():return [e['text'] for e in query('inspect_tree')['elements'] if e.get('text')]
    def diagnostics(after=0):return query('diagnostics',after=after,kind=None)['records']
    def capture():
        with (evidence/'supervisor.jsonl').open('w') as out,(evidence/'event-times.jsonl').open('w') as ts:
            for line in child.stdout:
                out.write(line);out.flush()
                try:event=json.loads(line)
                except ValueError:continue
                ts.write(json.dumps(dict(observed_us=time.time_ns()//1000,event=event))+'\n');ts.flush()
                if event.get('event')=='session':shared['session']=Path(event['session'])
                if event.get('event')=='watching':shared['ready']=True
                if event.get('event')=='build_failed':shared['failure']=event
    def click():
        button=next(e for e in query('inspect_tree')['elements'] if e.get('semantic_id')=='increment');box=button['layout'];w=query('preview')[0]
        point=dict(window=w['window'],native_id=w['native_id'],generation=w['generation'],x=round(box['x']+box['width']/2),y=round(box['y']+box['height']/2))
        try:exchange(instance(),'command',dict(type='preview_input',point=point,input=dict(type='click')))
        except KeyError as e:
            if e.args!=('value',):raise
    def occluded():return all(w.get('occluded') for w in query('preview'))
    def trial(kind,marker,label,allow_restart=False):
        wait(lambda:status()['phase']=='watching')
        if v.editor_pid:subprocess.run([v.windowctl,str(v.editor_pid),'front'],check=True,capture_output=True)
        time.sleep(.3);wait(occluded,15)
        before=instance();preview=query('preview');assert preview and all(w.get('occluded') for w in preview),preview
        cursor=max((r['sequence'] for r in diagnostics()),default=0)
        offsets={p:p.stat().st_size for p in (home/'workers').glob('*/log')}
        s=source.read_text();pattern=r'Live text (?:base|first|warm|trial \d+)' if kind=='text' else r'Helper (?:base|warm|trial \d+)'
        updated,n=re.subn(pattern,marker,s);assert n==1,n
        temp=source.with_suffix('.live-tmp');temp.write_text(updated);saved=time.time_ns()//1000;os.replace(temp,source)
        event=None
        if allow_restart:
            end=time.monotonic()+180
            while time.monotonic()<end:
                event=event or next((r for r in diagnostics(cursor) if r['kind']=='live_patch_applied'),None)
                now=texts()
                if marker in now and (event or instance()['pid']!=before['pid']):break
                if shared['failure']:raise RuntimeError(str(shared['failure']))
                time.sleep(.01)
            else:raise TimeoutError('no apply/redraw for '+label)
        else:
            event=wait(lambda:next((r for r in diagnostics(cursor) if r['kind']=='live_patch_applied'),None),180)
            now=wait(lambda:(visible if marker in (visible:=texts()) else None),30)
        redraw=time.time_ns()//1000;wait(lambda:status()['phase']=='watching')
        selected='';allraw={}
        for p in (home/'workers').glob('*/log'):
            with p.open('rb') as f:f.seek(offsets.get(p,0));raw=f.read().decode(errors='replace')
            if raw:allraw[str(p)]=raw
            if event and re.search(rf"live: edit {event['fields']['sequence']} .*at_us=",raw):selected=raw
        (evidence/(label+'.workers.json')).write_text(json.dumps(allraw,indent=2)+'\n')
        timelines=[{k:int(n) for k,n in re.findall(r'(\S+)=(\d+)',line)} for line in selected.splitlines() if line.startswith('timeline-us:')]
        published=re.search(rf"live: edit {event['fields']['sequence']} .*at_us=(\d+)",selected) if event else None
        row=dict(case=label,kind=kind,saved_us=saved,save_to_apply_ms=(event['fields']['at_unix_us']-saved)/1000 if event else None,save_to_redraw_ms=(redraw-saved)/1000,publication_to_apply_ms=(event['fields']['at_unix_us']-int(published[1]))/1000 if published else None,pid_before=before['pid'],pid_after=instance()['pid'],state_kept='Count: 2' in now,preview_before=preview,preview_after=query('preview'),system_load=os.getloadavg(),no_preview_input=True,event=event,timelines_us=timelines,macro_lines=[line for line in selected.splitlines() if line.startswith('proc macros:')])
        if not allow_restart:
            assert row['state_kept'] and row['pid_before']==row['pid_after'],row
            assert all(w.get('occluded') for w in row['preview_after']),row
        rows.append(row);(evidence/'trials.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps({k:row[k] for k in ['case','save_to_apply_ms','save_to_redraw_ms','pid_before','pid_after','state_kept']}),flush=True)
    try:
        source.write_text(baseline)
        with (evidence/'supervisor.stderr').open('w') as err:
            child=subprocess.Popen(cmd,cwd=root,env=env,stdout=subprocess.PIPE,stderr=err,text=True)
            thread=threading.Thread(target=capture,daemon=True);thread.start()
            wait(lambda:shared['ready'] or shared['failure'],1200)
            if shared['failure']:raise RuntimeError(str(shared['failure']))
            startup=time.monotonic()-start;wait(lambda:'Helper base' in texts())
            click();wait(lambda:'Count: 1' in texts());click();wait(lambda:'Count: 2' in texts())
            if v.manual_start:
                (evidence/'ready.json').write_text(json.dumps(dict(session=str(shared['session']),pid=instance()['pid']))+'\n')
                print('READY: restore the editor in front, then create '+str(evidence/'start'),flush=True)
                wait(lambda:(evidence/'start').exists(),900)
            elif v.editor_pid:subprocess.run([v.windowctl,str(v.editor_pid),'front'],check=True,capture_output=True)
            else:subprocess.run(['/usr/bin/osascript','-e','tell application "ChatGPT" to activate'],check=True,capture_output=True)
            wait(occluded,15)
            trial('text','Live text first','first-after-cold' if v.cold_build else 'first-after-warm-start',True)
            while 'Count: 2' not in texts():
                count=next(t for t in texts() if t.startswith('Count:'))
                assert count in {'Count: 0','Count: 1'},count
                click();wait(lambda:count not in texts())
            trial('text','Live text warm','warmup-text');trial('helper','Helper warm','warmup-helper')
            for kind in ['text','helper']:
                for i in range(201,201+v.trials):trial(kind,('Live text trial ' if kind=='text' else 'Helper trial ')+str(i),f'{kind}-{i}')
            summary={}
            for kind in ['text','helper']:
                rs=[r for r in rows if r['case'].startswith(kind+'-')]
                summary[kind]={k:statistics.median(r[k] for r in rs) for k in ['save_to_apply_ms','save_to_redraw_ms','publication_to_apply_ms'] if all(r[k] is not None for r in rs)}
                times=[r['timelines_us'][-1] for r in rs if r['timelines_us']]
                summary[kind]['timeline_us']={k:statistics.median(t[k] for t in times) for k in times[0]} if times else {}
            result=dict(startup_seconds=startup,command=cmd,mode=v.mode,no_blaze_on_path=v.mode=='pinned',archive_override=env.get('BLAZE_ARCHIVE'),pin_url=pin['url'],initial_rake_home_absent=v.clean_install,first_edit=rows[0],summary=summary,evidence=str(evidence))
            (evidence/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
            if v.mode=='pinned':
                doctor=subprocess.run([v.cli,'doctor','--json'],cwd=root,env=env,capture_output=True,text=True,check=True);(evidence/'doctor.json').write_text(doctor.stdout);assert json.loads(doctor.stdout)['live']['ready']
    finally:
        if child:
            child.terminate()
            try:child.wait(timeout=20)
            except subprocess.TimeoutExpired:child.kill();child.wait()
            if thread:thread.join(timeout=5)
        stop_env=dict(env,BLAZE_HOME=str(home))
        if v.mode=='pinned':stop_env['BLAZE_CACHE']=str(installed/'store')
        try:
            if shutil.which(compiler[0],path=stop_env.get('PATH')):
                subprocess.run([*compiler,'stop'],cwd=root,env=stop_env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30)
        finally:
            source.write_bytes(original)
if __name__=='__main__':
    value=sys.argv[1]
    if not value.startswith('{'):value=Path(value).read_text()
    main(Input(**json.loads(value)))
