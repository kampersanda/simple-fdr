#!/usr/bin/env python3
"""Run stage 4 correctness/partition comparison and construction scaling probes.

Build with cargo build --release --locked, then run:
python3 experiments/stage4.py target/release/simple-fdr > /tmp/stage4-report.json
Each time is one CLI sample, not a throughput benchmark.
"""
import json
import platform
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
root = Path(__file__).resolve().parent
strategies = ["balanced", "length-sorted", "dynamic-programming"]


def run(config, output, algorithm, strategy="balanced"):
    subprocess.run([
        str(binary), "run", str(config), str(output), "--algorithm", algorithm,
        "--bucket-count", "8", "--filter-length", "8",
        "--grouping-strategy", strategy,
    ], check=True)
    return json.loads(output.read_text())


with tempfile.TemporaryDirectory(prefix="simple-fdr-stage4-") as directory:
    temp = Path(directory)
    reference = run(root / "stage4.json", temp / "reference.json", "naive")
    comparison = []
    for strategy in strategies:
        result = run(root / "stage4.json", temp / f"{strategy}.json", "paper-sequential", strategy)
        assert result["matches"] == reference["matches"]
        comparison.append({key: result[key] for key in [
            "algorithm", "construction_ns", "search_ns", "match_count",
            "state_width_bits", "state_words", "paper_filter",
        ]})
    scaling = []
    for count in [100, 1000, 10000]:
        config = temp / f"config-{count}.json"
        config.write_text(json.dumps({
            "seed": 41, "input_len": 0, "pattern_count": count,
            "min_pattern_len": 2, "max_pattern_len": 32,
        }))
        for strategy in strategies:
            result = run(config, temp / f"{count}-{strategy}.json", "paper-sequential", strategy)
            scaling.append({"pattern_count": count, "strategy": strategy,
                            "construction_ns": result["construction_ns"]})
    report = {
        "platform": platform.platform(),
        "architecture": platform.machine(),
        "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "build": "release (default Cargo profile)",
        "timing": "single CLI sample, no warmup; statistics collected separately",
        "config": reference["config"],
        "bucket_count": 8, "filter_length": 8, "alpha": 1.05, "beta": 3,
        "comparison": comparison, "construction_scaling": scaling,
    }
    print(json.dumps(report, indent=2))
