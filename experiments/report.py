#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["matplotlib>=3.8"]
# ///
"""Turn the output of bench.py into Markdown tables and SVG charts.

Run: uv run experiments/report.py BENCH.json OUTPUT_DIR
Writes OUTPUT_DIR/results.md and one techniques-<case>.svg per case.
"""

import json
import sys
from pathlib import Path

import matplotlib

matplotlib.use("svg")
import matplotlib.pyplot as plt

report = json.loads(Path(sys.argv[1]).read_text())
output = Path(sys.argv[2])
output.mkdir(parents=True, exist_ok=True)


def mib_per_second(case, entry):
    return case["input_len"] * 1e9 / entry["median_search_ns"] / 2**20


def table(header, rows):
    lines = ["| " + " | ".join(header) + " |"]
    lines.append(
        "|" + "|".join("---" if i == 0 else "---:" for i in range(len(header))) + "|"
    )
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return lines


def stat(entry, key):
    statistics = entry.get("statistics")
    return f"{statistics[key]:,}" if statistics else "-"


lines = [
    "# 実験結果",
    "",
    f"- 環境：{report['platform']}、{report['cpu']}",
    f"- Rust：{report['rust']}（{report['build']}）",
    "- 時間：3回のウォームアップ後、9回計測した中央値",
    "",
]
for case in report["cases"]:
    name = case["name"].removesuffix(".json")
    lines += [
        f"## {name}",
        "",
        (
            f"入力 {case['input_len']:,} バイト、パターン {case['pattern_count']:,} 個"
            f"（{case['min_pattern_len']}〜{case['max_pattern_len']} バイト）、"
            f"一致 {case['match_count']:,} 件。"
            f"バケット数 {case['bucket_count']}、指定フィルタ長 {case['filter_length']}。"
        ),
        "",
        "### 技法ごとの比較",
        "",
    ]
    rows = []
    for entry in case["techniques"]:
        rows.append(
            [
                entry["name"],
                f"{mib_per_second(case, entry):,.1f}",
                f"{entry['median_search_ns'] / 1e6:,.2f}",
                f"{entry['state_width_bits']:,}"
                if "state_width_bits" in entry
                else "-",
                stat(entry, "candidate_count"),
                stat(entry, "full_comparisons"),
                f"{entry['mask_table_bytes']:,}"
                if "mask_table_bytes" in entry
                else "-",
            ]
        )
    lines += table(
        ["方式", "MiB/s", "検索ms", "状態幅", "候補数", "完全比較", "マスク表B"], rows
    )
    lines += ["", "### FDRのオプションの組合せ", ""]
    rows = [
        [
            entry["super_character_bits"],
            entry["batch_size"],
            entry["verification"],
            entry["backend"],
            f"{mib_per_second(case, entry):,.1f}",
            stat(entry, "candidate_count"),
            stat(entry, "full_comparisons"),
        ]
        for entry in case["comparison"]
    ]
    lines += table(
        ["追加ビット", "一括長", "検証", "実装", "MiB/s", "候補数", "完全比較"], rows
    )
    lines.append("")

    names = [entry["name"] for entry in case["techniques"]]
    speeds = [mib_per_second(case, entry) for entry in case["techniques"]]
    figure, axis = plt.subplots(figsize=(7, 0.35 * len(names) + 1))
    axis.barh(names[::-1], speeds[::-1], color="#4c72b0")
    axis.set_xscale("log")
    axis.set_xlabel("throughput (MiB/s, log scale)")
    axis.set_title(name)
    figure.tight_layout()
    figure.savefig(output / f"techniques-{name}.svg")
    plt.close(figure)
    lines += [f"![{name}](techniques-{name}.svg)", ""]

(output / "results.md").write_text("\n".join(lines))
