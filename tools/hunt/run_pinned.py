#!/usr/bin/env python3
"""Like run_realworld but each corpus line is `name@version` (pinned)."""
import sys, os, json, shutil, tarfile, subprocess, time, urllib.request, re, csv
from pathlib import Path
ROOT=Path("/Users/hassnain/Desktop/LHS/lhs")
LHS=ROOT/"target/debug/lhs"
TC="nightly-2026-01-10"
UA={"User-Agent":"lhs-eval/0.1 (research)"}
STATS=re.compile(r"LHS_STATS (\{.*\})")
env=dict(os.environ); env["RUSTUP_TOOLCHAIN"]=TC
home=subprocess.run(["rustc",f"+{TC}","--print","sysroot"],capture_output=True,text=True).stdout.strip()
if home: env["DYLD_FALLBACK_LIBRARY_PATH"]=str(Path(home)/"lib")
def run(cmd,cwd=None,to=300): return subprocess.run(cmd,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=to)
def fetch(name,ver,work):
    dst=work/f"{name}-{ver}"
    if dst.exists(): return dst
    url=f"https://static.crates.io/crates/{name}/{name}-{ver}.crate"
    tgz=work/f"{name}-{ver}.crate"
    with urllib.request.urlopen(urllib.request.Request(url,headers=UA),timeout=120) as r,open(tgz,"wb") as f: shutil.copyfileobj(r,f)
    with tarfile.open(tgz) as t:
        try: t.extractall(work,filter="data")
        except TypeError: t.extractall(work)
    return dst
def analyze(name,ver,work,to):
    try: d=fetch(name,ver,work)
    except Exception as e: return {"crate":f"{name}@{ver}","outcome":"fetch_error","detail":str(e)[:120]}
    shutil.rmtree(d/".cargo",ignore_errors=True)
    for f in d.glob("dangerous_spans.csv"): f.unlink()
    try: pre=run(["cargo","build"],cwd=d,to=to)
    except subprocess.TimeoutExpired: return {"crate":f"{name}@{ver}","outcome":"timeout","detail":"dep"}
    if pre.returncode!=0: return {"crate":f"{name}@{ver}","outcome":"build_error","detail":pre.stdout[-160:].replace("\n"," ")}
    run(["cargo","clean","-p",name],cwd=d,to=120)
    (d/".cargo").mkdir(exist_ok=True); (d/".cargo/config.toml").write_text(f'[build]\nrustc-wrapper = "{LHS}"\n')
    t0=time.time()
    try: p=run(["cargo","build"],cwd=d,to=to)
    except subprocess.TimeoutExpired: return {"crate":f"{name}@{ver}","outcome":"timeout","detail":"analysis"}
    out=p.stdout; low=out.lower()
    if "panicked at" in low or "internal compiler error" in low: 
        return {"crate":f"{name}@{ver}","outcome":"crash","detail":out[-200:].replace("\n"," ")}
    fp=d/"dangerous_spans.csv"; findings=0
    if fp.exists(): findings=max(0,sum(1 for _ in fp.open())-1)
    return {"crate":f"{name}@{ver}","outcome":"analyzed","elapsed":round(time.time()-t0,1),"findings":findings,"dir":str(d)}
def main():
    corpus=Path(sys.argv[1]); work=Path(sys.argv[2]); work.mkdir(parents=True,exist_ok=True)
    to=int(sys.argv[3]) if len(sys.argv)>3 else 300
    for ln in corpus.read_text().splitlines():
        ln=ln.strip()
        if not ln or ln.startswith("#"): continue
        name,ver=ln.split("@")
        r=analyze(name,ver,work,to)
        print(f"{r['outcome']:12} {r['crate']:28} findings={r.get('findings','-')} {r.get('detail','')[:70]}",flush=True)
main()
