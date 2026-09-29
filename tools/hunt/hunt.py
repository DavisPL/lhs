#!/usr/bin/env python3
"""Lean LHS hunt: analyze each crate's OWN code, then delete its build tree
immediately so the disk never accumulates more than one crate at a time.

On a failed build it escalates (build_mode in the output CSV): `default` (routine
zero-config build) -> `update` (bump stale transitive deps) -> `update+trim`
(drop native/C/rar-style features that pull unbuildable deps, keeping the
crate's pure-Rust code) -> `update+minimal` (--no-default-features). This
recovers crates like `decompress`, whose only blocker is an optional feature's
ancient dependency, so LHS can analyze the real code instead of just build_error."""
import sys, os, json, shutil, tarfile, subprocess, time, urllib.request, re, csv
from pathlib import Path
ROOT=Path("/Users/hassnain/Desktop/LHS/lhs"); LHS=ROOT/"target/debug/lhs"
TC="nightly-2026-01-10"; UA={"User-Agent":"lhs-eval/0.1 (research)"}
STATS=re.compile(r"LHS_STATS (\{.*\})")
env=dict(os.environ); env["RUSTUP_TOOLCHAIN"]=TC
home=subprocess.run(["rustc",f"+{TC}","--print","sysroot"],capture_output=True,text=True).stdout.strip()
if home: env["DYLD_FALLBACK_LIBRARY_PATH"]=str(Path(home)/"lib")
def run(cmd,cwd=None,to=300): return subprocess.run(cmd,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=to)
def _index_path(name):
    # crates.io sparse-index layout: 1/2/3-char prefix dirs
    n=name.lower()
    if len(n)==1: return f"1/{n}"
    if len(n)==2: return f"2/{n}"
    if len(n)==3: return f"3/{n[0]}/{n}"
    return f"{n[:2]}/{n[2:4]}/{n}"
def latest(name):
    # Resolve latest version via the sparse INDEX CDN (index.crates.io) which is
    # not rate-limited like the JSON API. Prefer newest non-yanked, non-prerelease;
    # fall back to newest non-yanked, then the JSON API on any failure.
    try:
        u=f"https://index.crates.io/{_index_path(name)}"
        raw=urllib.request.urlopen(urllib.request.Request(u,headers=UA),timeout=40).read().decode()
        vers=[json.loads(l) for l in raw.splitlines() if l.strip()]
        live=[v for v in vers if not v.get("yanked")]
        stable=[v for v in live if "-" not in v["vers"]] or live or vers
        return stable[-1]["vers"]
    except Exception:
        for i in range(4):
            try:
                d=json.load(urllib.request.urlopen(urllib.request.Request(
                    f"https://crates.io/api/v1/crates/{name}",headers=UA),timeout=40))
                return d["crate"].get("max_stable_version") or d["crate"]["max_version"]
            except Exception:
                time.sleep(2*(i+1))
        raise
def fetch(name,ver,work):
    tgz=work/f"{name}-{ver}.crate"; dst=work/f"{name}-{ver}"
    with urllib.request.urlopen(urllib.request.Request(f"https://static.crates.io/crates/{name}/{name}-{ver}.crate",headers=UA),timeout=120) as r,open(tgz,"wb") as f: shutil.copyfileobj(r,f)
    with tarfile.open(tgz) as t:
        try: t.extractall(work,filter="data")
        except TypeError: t.extractall(work)
    tgz.unlink(missing_ok=True)
    return dst
# Feature names that tend to pull UNbuildable native/C or ancient deps (e.g. the
# `unrar`->`num 0.1`->`rustc-serialize` chain that blocks `decompress`). When a
# plain build fails we retry with these dropped so the crate's own (pure-Rust)
# code still compiles and LHS can analyze it.
# Tier-1 deny: feature names that pull native/C or ancient deps (e.g.
# `rar`->`unrar`->`num 0.1`->`rustc-serialize`). Tier-2 additionally drops
# formats whose modern crate needs a *newer* rustc than the pin (e.g. `zip` 8.x)
# or is otherwise heavy. Meta-features (`all`/`full`) re-enable everything, so
# they are always skipped.
DENY1=re.compile(r"(unrar|rar|zstd|zst|xz|lzma|lz4|bzip|bz2|7z|sevenz|native|[-_]sys\b|ffi|bindgen|openssl)",re.I)
DENY2=re.compile(r"(zip|ar\b|brotli|http|reqwest|tokio|async|hyper|wasm|serde_|derive)",re.I)
META={"all","full","everything","complete","default"}
def crate_features(d):
    """Feature names declared in the crate's [features] table (minus meta-features)."""
    try: txt=(d/"Cargo.toml").read_text()
    except Exception: return []
    m=re.search(r"(?ms)^\[features\]\s*(.*?)(^\[|\Z)",txt)
    if not m: return []
    out=[]
    for line in m.group(1).splitlines():
        km=re.match(r'\s*"?([A-Za-z0-9_.-]+)"?\s*=',line)
        if km and km.group(1) not in META: out.append(km.group(1))
    return out
def build_attempts(d):
    """Escalation ladder; stop at the first `cargo build` that succeeds.
    (label, run_cargo_update_first, extra cargo-build flags)."""
    feats=crate_features(d)
    safe1=[f for f in feats if not DENY1.search(f)]
    safe2=[f for f in safe1 if not DENY2.search(f)]
    yield ("default", False, [])                              # routine build (zero-config)
    yield ("update", True, [])                                # bump stale transitive deps
    if safe1:                                                 # drop native/C/rar features
        yield ("update+trim1", True, ["--no-default-features","--features",",".join(safe1)])
    if safe2 and safe2!=safe1:                                # also drop zip/ar/heavy features
        yield ("update+trim2", True, ["--no-default-features","--features",",".join(safe2)])
    yield ("update+minimal", True, ["--no-default-features"]) # last resort: minimal features
def analyze(name,work,to,findings_dir):
    try: ver=latest(name)
    except Exception as e: return {"crate":name,"outcome":"version_error","detail":str(e)[:80]}
    d=None
    try:
        try: d=fetch(name,ver,work)
        except Exception as e: return {"crate":name,"version":ver,"outcome":"fetch_error","detail":str(e)[:80]}
        # --- find a build configuration that compiles (escalation ladder) ---
        updated=False; build_mode=None; flags=[]; pre=None
        for label,do_update,extra in build_attempts(d):
            if do_update and not updated:
                try: run(["cargo","update"],cwd=d,to=min(to,240))
                except subprocess.TimeoutExpired: pass
                updated=True
            try: pre=run(["cargo","build"]+extra,cwd=d,to=to)
            except subprocess.TimeoutExpired:
                if label=="default": return {"crate":name,"version":ver,"outcome":"timeout","detail":"dep"}
                continue
            if pre.returncode==0: build_mode=label; flags=extra; break
        if build_mode is None:
            return {"crate":name,"version":ver,"outcome":"build_error",
                    "detail":(pre.stdout[-120:].replace("\n"," ") if pre else "")}
        # --- rebuild ONLY the target crate with LHS wired in, same flags ---
        run(["cargo","clean","-p",name],cwd=d,to=120)
        (d/".cargo").mkdir(exist_ok=True); (d/".cargo/config.toml").write_text(f'[build]\nrustc-wrapper = "{LHS}"\n')
        t0=time.time()
        try: p=run(["cargo","build"]+flags,cwd=d,to=to)
        except subprocess.TimeoutExpired: return {"crate":name,"version":ver,"outcome":"timeout","detail":"analysis","build_mode":build_mode}
        out=p.stdout; low=out.lower()
        if "panicked at" in low or "internal compiler error" in low:
            return {"crate":name,"version":ver,"outcome":"crash","detail":out[-160:].replace("\n"," "),"build_mode":build_mode}
        fp=d/"dangerous_spans.csv"; findings=0
        if fp.exists():
            findings=max(0,sum(1 for _ in fp.open())-1)
            shutil.copy(fp, Path(findings_dir)/f"{name}-{ver}.csv")
        return {"crate":name,"version":ver,"outcome":"analyzed","elapsed":round(time.time()-t0,1),
                "findings":findings,"build_mode":build_mode}
    finally:
        if d is not None: shutil.rmtree(d, ignore_errors=True)
        # nuke any stray extracted dir / crate cache for this crate
        for p in work.glob(f"{name}-*"): shutil.rmtree(p, ignore_errors=True) if p.is_dir() else p.unlink(missing_ok=True)
def main():
    corpus=Path(sys.argv[1]); work=Path(sys.argv[2]); to=int(sys.argv[3]) if len(sys.argv)>3 else 300
    work.mkdir(parents=True,exist_ok=True)
    fdir=work/"findings"; fdir.mkdir(exist_ok=True)
    names=[l.split("\t")[0].strip() for l in corpus.read_text().splitlines() if l.strip() and not l.startswith("#")]
    rows=[]
    for i,n in enumerate(names,1):
        r=analyze(n,work,to,fdir); rows.append(r)
        extra=f"findings={r.get('findings','-')} {r.get('elapsed','')}s" if r["outcome"]=="analyzed" else r.get("detail","")[:60]
        print(f"[{i:>3}/{len(names)}] {r['outcome']:12} {n:24} {extra}",flush=True)
    with (work/"results.csv").open("w",newline="") as f:
        w=csv.DictWriter(f,fieldnames=["crate","version","outcome","build_mode","elapsed","findings","detail"],extrasaction="ignore"); w.writeheader(); w.writerows(rows)
    an=[r for r in rows if r["outcome"]=="analyzed"]
    print(f"\n== analyzed {len(an)}/{len(rows)}; with-findings {sum(1 for r in an if r.get('findings'))}; build_error {sum(1 for r in rows if r['outcome']=='build_error')} ==")
main()
