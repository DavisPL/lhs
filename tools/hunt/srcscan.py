#!/usr/bin/env python3
"""Apply LHS's zip-slip detection criterion to crate SOURCE (no build).
Flags crates where a RAW archive entry name reaches a filesystem write with no
recognized traversal guard — the same criterion used for the paper's B2-B6.
Downloads only source tarballs; deletes them after scanning (disk-lean)."""
import json,urllib.request,sys,re,io,tarfile,shutil
from pathlib import Path
UA={"User-Agent":"lhs-eval/0.1 (research)"}
WORK=Path(sys.argv[2]); WORK.mkdir(parents=True,exist_ok=True)
# raw (untrusted) archive entry-name accessors
RAW=re.compile(r"\.(file_name|filename|name|name_raw|path|path_bytes|entry_name|header)\b|->\s*name")
# writes / sinks
SINK=re.compile(r"(File::create(_new)?|OpenOptions::new|fs::write|std::os::unix::fs::symlink|symlink\(|create_dir)")
# recognized guards (if present near the flow, likely safe)
GUARD=re.compile(r"(enclosed_name|mangled_name|canonicaliz|strip_prefix|components\(\)|starts_with|\.\.|is_absolute|sanitiz|normaliz|assert)")
def latest(name):
    d=json.load(urllib.request.urlopen(urllib.request.Request(f"https://crates.io/api/v1/crates/{name}",headers=UA),timeout=40))
    return d["crate"].get("max_stable_version") or d["crate"]["max_version"]
def src_files(name):
    ver=latest(name)
    url=f"https://static.crates.io/crates/{name}/{name}-{ver}.crate"
    raw=urllib.request.urlopen(urllib.request.Request(url,headers=UA),timeout=120).read()
    out={}
    with tarfile.open(fileobj=io.BytesIO(raw)) as t:
        for m in t.getmembers():
            if m.name.endswith(".rs") and m.isfile():
                try: out[m.name]=t.extractfile(m).read().decode("utf-8","replace")
                except Exception: pass
    return ver,out
def scan(name):
    try: ver,files=src_files(name)
    except Exception as e: return {"crate":name,"status":f"err:{str(e)[:50]}"}
    hits=[]
    for fn,txt in files.items():
        lines=txt.splitlines()
        # find sink lines; check for a raw-name mention within a window and no guard nearby
        for i,l in enumerate(lines):
            if SINK.search(l):
                lo=max(0,i-15); hi=min(len(lines),i+3)
                window="\n".join(lines[lo:hi])
                if RAW.search(window):
                    guarded = bool(GUARD.search(window))
                    hits.append((fn,i+1,l.strip()[:90],guarded))
    if not hits: return {"crate":name,"version":ver,"status":"clean","nfiles":len(files)}
    unguarded=[h for h in hits if not h[3]]
    return {"crate":name,"version":ver,"status":"HIT" if unguarded else "guarded",
            "unguarded":unguarded,"guarded":[h for h in hits if h[3]]}
def main():
    names=[l.split("\t")[0].strip() for l in Path(sys.argv[1]).read_text().splitlines() if l.strip() and not l.startswith("#")]
    results=[]
    for n in names:
        r=scan(n); results.append(r)
        tag=r["status"]
        n_ug=len(r.get("unguarded",[])) if isinstance(r.get("unguarded"),list) else 0
        print(f"{tag:9} {n:26} {('unguarded='+str(n_ug)) if r['status']=='HIT' else r.get('version','')}",flush=True)
    print("\n==== HIT crates (raw entry name -> write, no guard in window) ====")
    for r in results:
        if r["status"]=="HIT":
            print(f"\n### {r['crate']} {r['version']}")
            for fn,ln,code,_ in r["unguarded"][:8]:
                print(f"    {fn}:{ln}: {code}")
    Path(sys.argv[2],"srcscan.json").write_text(json.dumps(results,indent=1))
main()
