#!/usr/bin/env python3
"""
Systematic RUSTSEC reproduction study (recall, RQ-A).

Instead of a hand-picked advisory list, this walks the ENTIRE RustSec
advisory-db, auto-selects every advisory in LHS's threat model (untrusted input
-> command/code execution, path traversal / arbitrary file write, env/argument
injection), resolves a *vulnerable* version of the crate, builds it with LHS
wired in as the rustc-wrapper, and checks whether LHS flags the advisory's
named affected function (when the advisory names one).

Outcome per advisory:
  detected          - an affected function (or, if none named, some sink) is flagged
  flows_but_not_fn  - LHS found flows, but not at the named affected function
  missed            - built + analyzed, no relevant finding
  build_error       - crate does not build on the toolchain (not counted against LHS)
  no_vuln_version   - could not resolve a vulnerable version from the index
  fetch_error       - download failed

Usage:
  export LIBRARY_PATH=...; export Z3_SYS_Z3_HEADER=...
  python3 tools/hunt/rustsec_scan.py <advisory-db-dir> <work-dir> [--limit N]
"""
import json, os, re, shutil, subprocess, tarfile, urllib.request, urllib.error, argparse, time
from pathlib import Path

ROOT = Path("/Users/hassnain/Desktop/LHS/lhs")
LHS = ROOT / "target" / "debug" / "lhs"
TC = os.environ.get("LHS_TOOLCHAIN", "nightly-2025-01-09")
UA = {"User-Agent": "lhs-eval/0.1 (research)"}

# --- in-scope selection: LHS models untrusted input -> fs/command/env sinks. ---
# RustSec `categories` is a controlled vocabulary; `code-execution` and
# `file-disclosure` are the ones that can be a source->sink data-flow LHS sees.
# (`format-injection` is HTML/XSS/SQL, `denial-of-service`/`memory-*` are
# out of LHS's model, so they are excluded to avoid inflating "missed".)
IN_SCOPE_CATEGORIES = {"code-execution", "file-disclosure"}
# Free-form `keywords` are the strongest signal for the exact bug classes LHS
# targets (path traversal / zip-slip / arbitrary file write / command injection).
IN_SCOPE_KEYWORDS = re.compile(
    r"(path[-_ ]?travers|directory[-_ ]?travers|zip[-_ ]?slip|tar[-_ ]?slip|"
    r"file[-_ ]?overwrite|file[-_ ]?write|arbitrary[-_ ]?file|arbitrary[-_ ]?write|"
    r"symlink|command[-_ ]?inject|code[-_ ]?inject|os[-_ ]?command|shell[-_ ]?inject|"
    r"argument[-_ ]?inject|arg[-_ ]?inject|traversal)", re.I)
# The "core" classes are LHS's sweet spot: traversal/file-write/symlink and
# command/code/argument injection. Recall is reported on this subset separately.
CORE_KEYWORDS = re.compile(
    r"(travers|zip[-_ ]?slip|tar[-_ ]?slip|file[-_ ]?overwrite|arbitrary[-_ ]?file|"
    r"arbitrary[-_ ]?write|symlink|command[-_ ]?inject|code[-_ ]?inject|"
    r"os[-_ ]?command|shell|argument[-_ ]?inject|arg[-_ ]?inject|extract|archive)", re.I)
# Informational advisories (unmaintained / unsound / notice) are not vulns.
SKIP_INFORMATIONAL = True


def is_core(adv):
    if "code-execution" in adv["categories"]:
        return True
    blob = " ".join(adv["keywords"] + [adv["id"] or ""])
    return bool(CORE_KEYWORDS.search(blob))

STATS = re.compile(r"LHS_STATS (\{.*\})")


def env():
    e = dict(os.environ, RUSTUP_TOOLCHAIN=TC)
    sysroot = subprocess.run(["rustc", f"+{TC}", "--print", "sysroot"],
                             capture_output=True, text=True).stdout.strip()
    if sysroot:
        e["DYLD_FALLBACK_LIBRARY_PATH"] = str(Path(sysroot) / "lib")
    return e


ENV = env()


def run(cmd, cwd, timeout=600):
    return subprocess.run(cmd, cwd=cwd, env=ENV, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True, timeout=timeout)


# ---- minimal TOML-frontmatter parse (advisory files are ```toml ... ``` + md) --
def parse_advisory(path):
    txt = path.read_text(errors="replace")
    m = re.search(r"```toml\n(.*?)\n```", txt, re.S)
    if not m:
        return None
    body = m.group(1)
    adv = {"id": None, "package": None, "keywords": [], "categories": [],
           "informational": None, "functions": {}, "patched": [], "unaffected": []}

    def grab(key):
        mm = re.search(rf'^{key}\s*=\s*"([^"]*)"', body, re.M)
        return mm.group(1) if mm else None

    def grab_list(key):
        mm = re.search(rf'^{key}\s*=\s*\[(.*?)\]', body, re.S | re.M)
        if not mm:
            return []
        return re.findall(r'"([^"]*)"', mm.group(1))

    adv["id"] = grab("id")
    adv["package"] = grab("package")
    adv["informational"] = grab("informational")
    adv["keywords"] = grab_list("keywords")
    adv["categories"] = grab_list("categories")
    # [affected.functions] block: "fn::path" = ["range", ...]
    fm = re.search(r"\[affected\.functions\](.*?)(\n\[|\Z)", body, re.S)
    if fm:
        for line in fm.group(1).splitlines():
            lm = re.match(r'\s*"([^"]+)"\s*=', line)
            if lm:
                adv["functions"][lm.group(1)] = True
    # [versions] patched / unaffected
    vm = re.search(r"\[versions\](.*?)(\n\[|\Z)", body, re.S)
    if vm:
        adv["patched"] = re.findall(r'"([^"]*)"', re.search(
            r'patched\s*=\s*\[(.*?)\]', vm.group(1), re.S).group(1)) if re.search(
            r'patched\s*=\s*\[', vm.group(1)) else []
        um = re.search(r'unaffected\s*=\s*\[(.*?)\]', vm.group(1), re.S)
        adv["unaffected"] = re.findall(r'"([^"]*)"', um.group(1)) if um else []
    return adv


def in_scope(adv):
    if SKIP_INFORMATIONAL and adv.get("informational"):
        return False
    if set(adv["categories"]) & IN_SCOPE_CATEGORIES:
        return True
    blob = " ".join(adv["keywords"] + [adv["id"] or ""])
    return bool(IN_SCOPE_KEYWORDS.search(blob))


# ---- version handling (lightweight semver) --------------------------------
def parse_ver(v):
    core = v.split("+")[0].split("-")[0]
    parts = (core.split(".") + ["0", "0", "0"])[:3]
    try:
        return tuple(int(p) for p in parts)
    except ValueError:
        return (0, 0, 0)


def index_path(name):
    n = name.lower()
    if len(n) == 1:
        return f"1/{n}"
    if len(n) == 2:
        return f"2/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[:2]}/{n[2:4]}/{n}"


def all_versions(name, _tries=6):
    """Fetch a crate's index line-JSON, retrying on 429/5xx/network with
    exponential backoff so a transient throttle self-heals instead of
    cascading every subsequent crate into `no_vuln_version`."""
    u = f"https://index.crates.io/{index_path(name)}"
    delay = 1.0
    for attempt in range(_tries):
        try:
            raw = urllib.request.urlopen(urllib.request.Request(u, headers=UA), timeout=40).read().decode()
            time.sleep(0.4)  # politeness: space bursts out to avoid re-triggering 429
            return [json.loads(l) for l in raw.splitlines() if l.strip()]
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503, 504) and attempt < _tries - 1:
                ra = e.headers.get("Retry-After")
                time.sleep(float(ra) if (ra and ra.isdigit()) else delay)
                delay = min(delay * 2, 30)
                continue
            raise
        except urllib.error.URLError:
            if attempt < _tries - 1:
                time.sleep(delay)
                delay = min(delay * 2, 30)
                continue
            raise


def range_lower(bound):
    """Extract the numeric bound from a range string like '>= 2.3.0' or '< 1.3.0'."""
    mm = re.search(r"(\d+\.\d+(?:\.\d+)?)", bound)
    return parse_ver(mm.group(1)) if mm else None


def pick_vulnerable(name, adv):
    """Highest published non-yanked, non-prerelease version that is vulnerable:
    below the lowest `patched` lower-bound, and >= any `unaffected` '< X' bound."""
    try:
        vers = all_versions(name)
    except Exception:
        return None
    # NOTE: include *yanked* versions — the vulnerable release is frequently
    # yanked precisely because of the advisory, and it is still downloadable and
    # buildable, so excluding yanked would drop exactly the version we need.
    cand = [v["vers"] for v in vers if "-" not in v["vers"]]
    if not cand:
        cand = [v["vers"] for v in vers]
    if not cand:
        return None
    # patched lower bounds -> vulnerable is strictly below the smallest of them
    patched_lows = [range_lower(p) for p in adv["patched"] if ">=" in p or ">" in p]
    patched_lows = [p for p in patched_lows if p]
    ceil = min(patched_lows) if patched_lows else None
    # unaffected '< X' means everything below X is NOT vulnerable
    floor = None
    for u in adv["unaffected"]:
        if "<" in u:
            lo = range_lower(u)
            if lo and (floor is None or lo > floor):
                floor = lo
    vuln = []
    for v in cand:
        pv = parse_ver(v)
        if ceil and pv >= ceil:
            continue
        if floor and pv < floor:
            continue
        vuln.append(v)
    if not vuln:
        # no patch info usable -> fall back to the latest version (still may be vuln)
        vuln = cand if not adv["patched"] else []
    if not vuln:
        return None
    vuln.sort(key=parse_ver)
    return vuln[-1]


def fetch(name, ver, work):
    d = work / f"{name}-{ver}"
    if (d / "Cargo.toml").exists():
        return d
    url = f"https://static.crates.io/crates/{name}/{name}-{ver}.crate"
    tgz = work / f"{name}-{ver}.crate"
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120) as r, open(tgz, "wb") as f:
        shutil.copyfileobj(r, f)
    with tarfile.open(tgz) as t:
        try:
            t.extractall(work, filter="data")
        except TypeError:
            t.extractall(work)
    tgz.unlink(missing_ok=True)
    return d


def analyze(adv, work, timeout):
    name, ver_hint = adv["package"], None
    ver = pick_vulnerable(name, adv)
    base = dict(advisory=adv["id"], crate=name, categories=adv["categories"],
                keywords=adv["keywords"], core=is_core(adv),
                fns=list(adv["functions"].keys()))
    if not ver:
        return dict(base, ver=None, result="no_vuln_version")
    base["ver"] = ver
    try:
        d = fetch(name, ver, work)
    except Exception as e:
        return dict(base, result="fetch_error", detail=str(e)[:120])
    if not (d / "Cargo.toml").exists():
        return dict(base, result="fetch_error", detail="no manifest")

    def cleanup():
        shutil.rmtree(d, ignore_errors=True)

    # 1) build deps + crate without the wrapper; retry once with minimal-versions.
    try:
        pre = run(["cargo", "build"], d, timeout)
        if pre.returncode != 0:
            run(["cargo", "generate-lockfile", "-Z", "minimal-versions"], d, 180)
            pre = run(["cargo", "build"], d, timeout)
    except subprocess.TimeoutExpired:
        cleanup()
        return dict(base, result="build_error", detail="timeout(dep build)")
    if pre.returncode != 0:
        tail = pre.stdout[-160:].replace("\n", " ")
        cleanup()
        return dict(base, result="build_error", detail=tail)

    # 2) drop the target crate's artifacts and rebuild it with LHS wired in.
    run(["cargo", "clean", "-p", name], d, 120)
    (d / ".cargo").mkdir(exist_ok=True)
    (d / ".cargo" / "config.toml").write_text(f'[build]\nrustc-wrapper = "{LHS}"\n')
    for f in d.glob("dangerous_spans.csv"):
        f.unlink()
    try:
        out = run(["cargo", "build"], d, timeout).stdout
    except subprocess.TimeoutExpired:
        cleanup()
        return dict(base, result="build_error", detail="timeout(analysis)")
    low = out.lower()
    if "panicked at" in low or "internal compiler error" in low:
        cleanup()
        return dict(base, result="crash", detail=out[-200:].replace("\n", " "))

    csv = d / "dangerous_spans.csv"
    findings = csv.read_text().splitlines()[1:] if csv.exists() else []
    # also capture the WARNING public-API sink lines from stdout
    warn_fns = re.findall(r"public API taint sink: `([^`]+)`", out)
    # LHS emits `LHS_FINDING_FN <def_path>` for each analyzed function that
    # produced a finding — that is the containing function an advisory names as
    # affected (the CSV's column 0 is the taint *source*, not the function).
    finding_fns = re.findall(r"^LHS_FINDING_FN (.+)$", out, re.M)
    all_flagged = [l.split(",")[0] for l in findings] + warn_fns + finding_fns
    stats = {}
    for m in STATS.finditer(out):
        try:
            stats = json.loads(m.group(1))
        except json.JSONDecodeError:
            pass
    cleanup()

    named = list(adv["functions"].keys())
    if named:
        # match if any affected function's last path segment appears in a flagged name
        def seg(fp):
            return fp.split("::")[-1].split("<")[0]
        hit = [fp for fp in named if any(seg(fp) and seg(fp) in fl for fl in all_flagged)]
        result = "detected" if hit else ("flows_but_not_fn" if all_flagged else "missed")
        return dict(base, result=result, findings=len(findings),
                    matched=hit, n_flagged=len(all_flagged), stats=stats)
    else:
        result = "detected" if all_flagged else "missed"
        return dict(base, result=result, findings=len(findings),
                    n_flagged=len(all_flagged), stats=stats)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("db", type=Path, help="path to advisory-db checkout")
    ap.add_argument("work", type=Path, help="scratch work dir")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--timeout", type=int, default=420)
    ap.add_argument("--out", type=Path,
                    default=ROOT / "examples/eval/results/rustsec/rustsec_results_2025-01-09.json")
    args = ap.parse_args()
    if not LHS.exists():
        raise SystemExit(f"[FATAL] build LHS first: {LHS} missing")
    args.work.mkdir(parents=True, exist_ok=True)

    advs = []
    for p in sorted((args.db / "crates").rglob("*.md")):
        adv = parse_advisory(p)
        if adv and adv["package"] and in_scope(adv):
            advs.append(adv)
    # de-dup by (package,id)
    seen, uniq = set(), []
    for a in advs:
        k = (a["package"], a["id"])
        if k not in seen:
            seen.add(k)
            uniq.append(a)
    advs = uniq
    if args.limit:
        advs = advs[:args.limit]
    print(f"[INFO] {len(advs)} in-scope advisories, toolchain {TC}", flush=True)

    rows = []
    t_start = time.time()
    for i, adv in enumerate(advs, 1):
        try:
            r = analyze(adv, args.work, args.timeout)
        except Exception as e:  # noqa: BLE001
            r = dict(advisory=adv["id"], crate=adv["package"], result="harness_error",
                     detail=str(e)[:120])
        rows.append(r)
        extra = r.get("matched") or r.get("detail", "")
        print(f"[{i:>3}/{len(advs)}] {r['result']:16} {r['advisory']:20} "
              f"{r['crate']:24} {str(extra)[:60]}", flush=True)
        args.out.write_text(json.dumps(rows, indent=2))  # checkpoint each step

    # ---- summary ----
    by = {}
    for r in rows:
        by[r["result"]] = by.get(r["result"], 0) + 1
    buildable = sum(by.get(k, 0) for k in ("detected", "flows_but_not_fn", "missed"))
    print("\n==============  SYSTEMATIC RUSTSEC SUMMARY  ==============")
    print(f"  in-scope advisories attempted : {len(rows)}")
    for k in sorted(by):
        print(f"    {k:18}: {by[k]}")
    if buildable:
        print(f"  detected / buildable          : {by.get('detected',0)}/{buildable}")
    core = [r for r in rows if r.get("core")]
    core_build = [r for r in core if r["result"] in ("detected", "flows_but_not_fn", "missed")]
    core_det = [r for r in core if r["result"] == "detected"]
    print(f"  CORE (traversal/fs-write/cmd/code-exec): {len(core)} advisories, "
          f"detected {len(core_det)}/{len(core_build)} buildable")
    print(f"  wall-clock: {round(time.time()-t_start)}s")
    print(f"[DONE] wrote {args.out}")
    print("=========================================================")


if __name__ == "__main__":
    main()
