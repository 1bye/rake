#!/usr/bin/env python3
"""Opt-in real pinned archive acceptance on macOS arm64. No publication occurs.
One JSON object: rake_root, cli, archive; optional evidence directory.
Requires an absent ~/.rake installation for the first check. Restores fixture source.
"""
from dataclasses import dataclass
from pathlib import Path
import json, os, platform, shutil, signal, subprocess, sys, tempfile, time
@dataclass(frozen=True)
class Input:
    rake_root: str
    cli: str
    archive: str
    evidence: str = ""

def main(v):
    if (platform.system(),platform.machine())!=("Darwin","arm64"):raise SystemExit("Apple Silicon Mac required")
    root=Path(v.rake_root).resolve();cli=Path(v.cli).resolve();archive=Path(v.archive).resolve()
    evidence=Path(v.evidence).resolve() if v.evidence else Path(tempfile.mkdtemp(prefix="rake-live-evidence-"))
    evidence.mkdir(parents=True,exist_ok=True)
    if (Path.home()/".rake").exists():raise SystemExit("clean-install acceptance requires ~/.rake to be absent; preserve it rather than deleting user files")
    from native_test_client import exchange
    source=root/"examples/view/src/lib.rs";original=source.read_text()
    baseline=original.replace('mod inputs;','mod inputs;\n\nfn rake_pinned_probe_helper() -> &\'static str { "Pinned helper initial" }')
    heading='<h1>HTML-like views. Native GPUI.</h1>'
    assert baseline.count(heading)==1
    baseline=baseline.replace(heading,heading+'\n                            <p>{rake_pinned_probe_helper()}</p>')
    source.write_text(baseline)
    env={k:value for k,value in os.environ.items() if not k.startswith(("BLAZE_","RAKE_"))}
    # Private tool links retain Cargo metadata support while excluding every PATH
    # directory containing Blaze. The selected compiler never comes from PATH.
    tools=tempfile.TemporaryDirectory(prefix="rake-live-path-")
    for name in ["cargo","rustc","rustdoc","rustup"]:
        found=shutil.which(name)
        if found:Path(tools.name,name).symlink_to(found)
    directories=[d for d in os.environ["PATH"].split(os.pathsep) if d and not Path(d,"blaze").exists()]
    env.update(PATH=os.pathsep.join([tools.name,*directories]),BLAZE_ARCHIVE=str(archive),CARGO_TARGET_DIR=str(evidence/"target"),CARGO_BUILD_JOBS="3")
    assert shutil.which("blaze",path=env["PATH"]) is None
    env.pop("RAKE_BLAZE",None)
    cmd=[str(cli),"dev","--live","--manifest-path",str(root/"examples/view/Cargo.toml"),"--bin","rake-view","--json","--no-open"]
    child=None;session=None;start=time.monotonic();records=[]
    def wait(fn,timeout=60):
        end=time.monotonic()+timeout;last=None
        while time.monotonic()<end:
            try:
                result=fn()
                if result:return result
            except (OSError,KeyError,RuntimeError,StopIteration) as e:last=e
            time.sleep(.01)
        raise TimeoutError(str(last))
    def status():return json.loads((session/"status.json").read_text())
    def instance():
        pid=status()["runtime_pid"]
        return next(v for p in (session/"control").glob("*.json") if (v:=json.loads(p.read_text()))["pid"]==pid)
    def query(name,**kw):return exchange(instance(),"query",dict(type=name,**kw))
    def texts():return [e["text"] for e in query("inspect_tree")["elements"] if e.get("text")]
    def diagnostics(after=0):return query("diagnostics",after=after,kind=None)["records"]
    try:
        with (evidence/"supervisor.jsonl").open("w") as events,(evidence/"supervisor.stderr").open("w") as err:
            child=subprocess.Popen(cmd,cwd=root,env=env,stdout=subprocess.PIPE,stderr=err,text=True)
            for line in child.stdout:
                events.write(line);events.flush()
                try:event=json.loads(line)
                except ValueError:continue
                if event.get("event")=="session":session=Path(event["session"])
                if event.get("event")=="build_failed":raise RuntimeError(str(event))
                if event.get("event")=="watching":break
            else:raise RuntimeError("supervisor exited before launch")
            startup=time.monotonic()-start;wait(lambda:"Pinned helper initial" in texts())
            tree=query("inspect_tree");button=next(e for e in tree["elements"] if e.get("semantic_id")=="increment");box=button["layout"];window=query("preview")[0]
            point=dict(window=window["window"],native_id=window["native_id"],generation=window["generation"],x=round(box["x"]+box["width"]/2),y=round(box["y"]+box["height"]/2))
            try:exchange(instance(),"command",dict(type="preview_input",point=point,input=dict(type="click")))
            except KeyError:pass
            wait(lambda:"Count: 1" in texts())
            for case,old,new,expected in [("view text","HTML-like views. Native GPUI.","Pinned live text","Pinned live text"),("helper","Pinned helper initial","Pinned helper edited","Pinned helper edited")]:
                wait(lambda:status()["phase"]=="watching")
                before=instance();cursor=max((x["sequence"] for x in diagnostics()),default=0)
                current=source.read_text();assert current.count(old)==1
                temp=source.with_suffix(".live-tmp");temp.write_text(current.replace(old,new));saved=time.time_ns()//1000;os.replace(temp,source)
                applied=None;end=time.monotonic()+120
                while time.monotonic()<end:
                    if applied is None:applied=next((x for x in diagnostics(cursor) if x["kind"]=="live_patch_applied"),None)
                    now=texts()
                    if applied and expected in now:break
                    if status()["phase"]=="build_failed":raise RuntimeError(status()["error"])
                    time.sleep(.01)
                else:raise TimeoutError("no live apply/redraw for "+case)
                row=dict(case=case,save_to_apply_ms=(applied["fields"]["at_unix_us"]-saved)/1000,save_to_redraw_ms=(time.time_ns()//1000-saved)/1000,pid_before=before["pid"],pid_after=instance()["pid"],state_kept="Count: 1" in now,event=applied)
                assert row["pid_before"]==row["pid_after"] and row["state_kept"],row
                records.append(row);print(json.dumps(row),flush=True)
                (evidence/"reloads.json").write_text(json.dumps(records,indent=2)+"\n")
            doctor=subprocess.run([cli,"doctor","--json"],cwd=root,env=env,capture_output=True,text=True)
            (evidence/"doctor.json").write_text(doctor.stdout)
            assert json.loads(doctor.stdout)["live"]["ready"],doctor.stdout
            result=dict(startup_seconds=startup,no_blaze_on_path=True,initial_rake_home_absent=True,command=cmd,reloads=records,evidence=str(evidence))
            (evidence/"result.json").write_text(json.dumps(result,indent=2)+"\n");print(json.dumps(result),flush=True)
    finally:
        source.write_text(original)
        if child:
            child.terminate()
            try:child.wait(timeout=20)
            except subprocess.TimeoutExpired:child.kill();child.wait()
        # Only stop the daemon owned by this versioned Rake installation.
        pin=__import__("tomllib").loads((root/"crates/rake-cli/blaze-pin.toml").read_text());installed=Path.home()/".rake/blaze"/pin["version"]
        if (installed/"blaze").exists():
            subprocess.run([installed/"blaze-engine","cargo","stop"],cwd=root,env=dict(env,BLAZE_HOME=str(installed/"runtime")),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30)
        tools.cleanup()
if __name__=="__main__":
    value=sys.argv[1]
    if not value.startswith("{"):value=Path(value).read_text()
    main(Input(**json.loads(value)))
