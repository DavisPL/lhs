#!/usr/bin/env python3
import csv
import subprocess
from pathlib import Path
import sys
import os

ROOT_DIR = Path(os.getcwd())
EXAMPLES_DIR = ROOT_DIR / "examples" / "crates"
RESULTS_CSV  = EXAMPLES_DIR / "results.csv"
LHS_BIN      = ROOT_DIR / "target" / "debug" / "lhs"
CSV_NAME     = "dangerous_spans.csv"

def run(cmd, cwd=None):
    return subprocess.run(
        cmd, cwd=cwd,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, check=False,
    )

def build_lhs():
    print("[INFO] Building lhs…")
    result = run(["cargo", "build", "--manifest-path", str(ROOT_DIR / "Cargo.toml")])
    if result.returncode != 0:
        print(f"[ERROR] Failed to build lhs: {result.stdout}")
        sys.exit(1)

def crate_hit(crate_dir: Path) -> bool:
    csv_path = crate_dir / CSV_NAME
    if not csv_path.exists():
        return False
    try:
        with csv_path.open() as f:
            return sum(1 for _ in f) > 1
    except Exception:
        return False

def cleanup(crate_dir: Path):
    for p in [crate_dir / CSV_NAME, crate_dir / ".cargo" / "config.toml"]:
        try:
            if p.exists():
                p.unlink()
        except Exception:
            pass
    cargo_dir = crate_dir / ".cargo"
    if cargo_dir.exists() and not any(cargo_dir.iterdir()):
        try:
            cargo_dir.rmdir()
        except Exception:
            pass

def eval_group(group: str, writer: csv.writer, summary: dict):
    group_dir = EXAMPLES_DIR / group
    if not group_dir.is_dir():
        print(f"[WARN] {group_dir} missing, skipping.")
        return

    for crate_dir in sorted(p for p in group_dir.iterdir() if p.is_dir()):
        crate_name = crate_dir.name

        cfg_dir = crate_dir / ".cargo"
        cfg_dir.mkdir(parents=True, exist_ok=True)
        (cfg_dir / "config.toml").write_text(
            f'[build]\nrustc-wrapper = "{LHS_BIN}"\n', encoding="utf-8"
        )

        run(["cargo", "clean"], cwd=crate_dir)
        _ = run(["cargo", "build"], cwd=crate_dir).stdout

        hit = crate_hit(crate_dir)
        result = ("FAIL", "PASS")[hit] if group == "unsafe" else ("PASS", "FAIL")[hit]

        print(f"[INFO] {group}: {crate_name} → {result}")

        writer.writerow([group, crate_name, hit, result])

        summary["total"] += 1
        if result == "PASS":
            summary["passed"] += 1
        else:
            summary["failed"].append((group, crate_name))

        cleanup(crate_dir)

def main():
    # Verify that the LHS binary will exist after build
    if not ROOT_DIR.exists():
        print(f"[ERROR] LHS root directory does not exist: {ROOT_DIR}")
        sys.exit(1)
    
    build_lhs()
    
    # Verify LHS binary was built successfully
    if not LHS_BIN.exists():
        print(f"[ERROR] LHS binary not found at: {LHS_BIN}")
        sys.exit(1)
    
    EXAMPLES_DIR.mkdir(parents=True, exist_ok=True)

    summary = {
        "total": 0,
        "passed": 0,
        "failed": []
    }

    with RESULTS_CSV.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["group", "crate", "hit", "result"])
        eval_group("safe", w, summary)
        eval_group("unsafe", w, summary)

    print(f"\n[SUMMARY] Tests passed: {summary['passed']} / {summary['total']}")
    if summary["failed"]:
        print("[FAILED TESTS]: 😱")
        for group, crate in summary["failed"]:
            print(f"  - {group}/{crate}")
    else:
        print("[ALL TESTS PASSED] 🎉😎")

    print(f"\n[DONE] Wrote {RESULTS_CSV}")

if __name__ == "__main__":
    main()
