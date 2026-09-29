import json,urllib.request,sys,io,tarfile
from pathlib import Path
UA={"User-Agent":"lhs-eval/0.1 (research)"}
name=sys.argv[1]; out=Path(sys.argv[2])
d=json.load(urllib.request.urlopen(urllib.request.Request(f"https://crates.io/api/v1/crates/{name}",headers=UA),timeout=40))
ver=d["crate"].get("max_stable_version") or d["crate"]["max_version"]
raw=urllib.request.urlopen(urllib.request.Request(f"https://static.crates.io/crates/{name}/{name}-{ver}.crate",headers=UA),timeout=120).read()
dst=out/f"{name}-{ver}"
with tarfile.open(fileobj=io.BytesIO(raw)) as t:
    try: t.extractall(out,filter="data")
    except TypeError: t.extractall(out)
print(dst)
