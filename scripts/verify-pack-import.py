#!/usr/bin/env python3
"""Compare the Tauri import entries on a parent revision and this checkout."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile

root = Path(__file__).resolve().parents[1]
parent = sys.argv[1] if len(sys.argv) > 1 else "origin/audit/36-pack-source"
receipt = root / "target" / "pack-import-proof"
receipt.mkdir(parents=True, exist_ok=True)
revision = subprocess.check_output(["git", "rev-parse", parent], cwd=root, text=True).strip()
source = receipt / revision
source.mkdir(exist_ok=True)
archive = subprocess.check_output(["git", "archive", revision], cwd=root)
with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
    tree.extractall(source, filter="data")
test = Path("crates/mine-utils-core/tests/import_plan_equivalence.rs")
(source / test).write_bytes((root / test).read_bytes())


def run(label, directory, target):
    env = os.environ.copy()
    env.pop("APPIMAGE", None)
    env["CARGO_TARGET_DIR"] = str(target)
    command = ["cargo", "test", "-p", "mine-utils-core", "--test", "import_plan_equivalence", "--", "--nocapture"]
    with (receipt / f"{label}.log").open("w") as log:
        subprocess.run(command, cwd=directory, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    rows = [json.loads(line.removeprefix("SELFPROOF ")) for line in (receipt / f"{label}.log").read_text().splitlines() if line.startswith("SELFPROOF ")]
    (receipt / f"{label}.json").write_text(json.dumps(rows, indent=2) + "\n")
    for row in rows:
        for entry in row["files"].values():
            if isinstance(entry, dict) and "nbt" in entry:
                # NBT compounds have unordered keys; the raw hash stays in the receipt.
                del entry["sha256"]
    assert len(rows) == 52, f"{label}: expected 52 scenarios, got {len(rows)}"
    return rows


before = run("parent", source, receipt / "parent-target")
after = run("pr", root, root / "target")
assert before == after, "Import outcomes or disk/trash contents differ; inspect parent.json and pr.json"
result = f"PASS: 52 parent-versus-PR scenarios match. Parent {revision}. Raw file hashes and parsed NBT are in {receipt}. NBT key order is ignored.\n"
(receipt / "result.txt").write_text(result)
print(result, end="")
