#!/usr/bin/env python3
"""
Build a crates.io corpus for the LHS real-world run, systematically by keyword.

Queries the crates.io keyword API for each in-scope keyword (filesystem /
archive / extraction / process), takes the top-N crates by all-time downloads
per keyword, de-dups, and writes `<name>\\t<downloads>\\t<keywords-hit>` lines
sorted by downloads. Deterministic given the API state.

Usage:
  python3 tools/hunt/gen_corpus.py --per-keyword 40 --cap 220 --out corpus_kw.tsv
"""
import argparse, json, time, urllib.request, urllib.parse
from pathlib import Path

UA = {"User-Agent": "lhs-eval/0.1 (research; contact via crates.io)"}

# In-scope keywords: LHS's threat model is untrusted input -> fs/command/env.
# Weighted toward archive/extraction (where the zip-slip class lives) plus
# filesystem/path and a few process/command keywords (RQ3's build-tool findings).
KEYWORDS = [
    "archive", "unzip", "untar", "extract", "decompress", "unpack",
    "tar", "zip", "compression", "cpio", "sevenz", "7z", "rar",
    "filesystem", "fs", "path", "directory", "extraction",
    "command", "process", "subprocess",
]
# Free-text (`q=`) queries matched against crate NAME + description, not just
# author tags. This catches name-based hits the keyword tags miss (e.g. the
# `unzip` crate has NO keywords but its name contains "zip"). Text-query results
# are kept in full up to --text-per-query (they are NOT subject to the global
# keyword --cap), so a deep name-match like `unzip` (rank ~161 for q=zip) is
# retained.
TEXT_QUERIES = ["zip"]


def api(url, tries=4):
    for i in range(tries):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60) as r:
                return json.load(r)
        except Exception as e:  # noqa: BLE001
            if i == tries - 1:
                print(f"# giving up on {url}: {e}")
                return {}
            time.sleep(2 * (i + 1))
    return {}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per-keyword", type=int, default=40)
    ap.add_argument("--cap", type=int, default=220)
    ap.add_argument("--text-per-query", type=int, default=200)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    downloads = {}
    hits = {}

    def pull(param_name, term, limit, tag):
        """Fetch top `limit` crates by downloads for a keyword= or q= query."""
        got = 0
        for page in range(1, limit // 100 + 2):
            if got >= limit:
                break
            url = (f"https://crates.io/api/v1/crates?{param_name}={urllib.parse.quote(term)}"
                   f"&per_page=100&page={page}&sort=downloads")
            data = api(url)
            crates = data.get("crates", [])
            if not crates:
                break
            for c in crates:
                if got >= limit:
                    break
                name, dl = c["id"], c.get("downloads", 0)
                downloads[name] = max(downloads.get(name, 0), dl)
                hits.setdefault(name, set()).add(tag)
                got += 1
            time.sleep(1.0)  # be polite to crates.io
        return got

    # (1) keyword-tag search
    for kw in KEYWORDS:
        n = pull("keyword", kw, args.per_keyword, kw)
        print(f"# keyword {kw}: took {n}", flush=True)

    # keyword results are globally capped by downloads (as before)...
    kw_names = set(sorted(downloads, key=lambda n: -downloads[n])[:args.cap])

    # (2) free-text search — kept IN FULL up to --text-per-query, bypassing the cap
    text_names = set()
    for q in TEXT_QUERIES:
        before = set(downloads)
        n = pull("q", q, args.text_per_query, f"q:{q}")
        text_names |= {nm for nm in downloads if f"q:{q}" in hits[nm]}
        print(f"# text q={q}: took {n}", flush=True)

    names = sorted(kw_names | text_names, key=lambda n: -downloads[n])
    with args.out.open("w") as f:
        f.write("# name\tdownloads\tsources\n")
        for n in names:
            f.write(f"{n}\t{downloads[n]}\t{','.join(sorted(hits[n]))}\n")
    print(f"[DONE] {len(names)} unique crates "
          f"({len(kw_names)} keyword + {len(text_names)} text, deduped) -> {args.out}")


if __name__ == "__main__":
    main()
