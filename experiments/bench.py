#!/usr/bin/env python3
"""Build: cargo build --locked --release --example bench
Run: python3 experiments/bench.py target/release/examples/bench
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
        for name in ["bytes-8-32.json", "abcd-2-32.json", "abcdqrst-2-6.json"]
    ],
}
print(json.dumps(report, indent=2))
