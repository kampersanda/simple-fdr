#!/usr/bin/env python3
"""Build: cargo build --locked --release --example batched_fdr
Run: python3 experiments/stage5.py target/release/examples/batched_fdr
"""

import json
import platform
import subprocess
import sys
from pathlib import Path

binary = Path(sys.argv[1]).resolve()
root = Path(__file__).resolve().parent
cpu = (
    subprocess.check_output(
        ["sysctl", "-n", "machdep.cpu.brand_string"], text=True
    ).strip()
    if platform.system() == "Darwin"
    else platform.processor()
)
report = {
    "platform": platform.platform(),
    "cpu": cpu,
    "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "build": "Cargo default release profile; no custom RUSTFLAGS",
    "cases": [
        json.loads(subprocess.check_output([str(binary), str(root / name)], text=True))
        for name in ["stage4.json", "stage5.json"]
    ],
}
print(json.dumps(report, indent=2))
