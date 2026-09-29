import json,urllib.request,sys,re
UA={"User-Agent":"lhs-eval/0.1 (research)"}
def api(u):
    return json.load(urllib.request.urlopen(urllib.request.Request(u,headers=UA),timeout=60))
cand={}
KWS=["unzip","untar","extract","archive","decompress","unpack","zip","tar"]
for kw in KWS:
    for page in (1,2):
        try: data=api(f"https://crates.io/api/v1/crates?keyword={kw}&per_page=100&page={page}&sort=downloads")
        except Exception as e: print(f"# {kw} p{page} {e}",file=sys.stderr); continue
        for c in data.get("crates",[]):
            cand[c["id"]]=max(cand.get(c["id"],0),c.get("downloads",0))
# also q= search for extraction-y terms (name/desc match)
for q in ["unzip","extract archive","untar","decompress archive"]:
    try: data=api(f"https://crates.io/api/v1/crates?q={urllib.parse.quote(q)}&per_page=50&sort=downloads") if False else None
    except Exception: data=None
# keep names that look like extractors (heuristic) OR are archive-format libs
pat=re.compile(r"(unzip|untar|unpack|extract|decompress|archive|zip|tar|cpio|cab|rar|7z|sevenz|cbv|gz|xz|zstd|lz4|brotli|deflate)",re.I)
names=[n for n in cand if pat.search(n)]
names.sort(key=lambda n:-cand[n])
for n in names: print(f"{n}\t{cand[n]}")
