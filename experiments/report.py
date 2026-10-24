#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["matplotlib>=3.8"]
# ///
"""Turn the output of bench.py into Markdown tables and SVG charts.

Run: uv run experiments/report.py BENCH.json OUTPUT_DIR
Writes OUTPUT_DIR/results.md, one techniques-<case>.svg per case, and one
scaling-<text>-<min>-<max>.svg per series of word datasets differing only in
pattern count. The tables list every measured configuration; the charts and the
scaling table keep only the techniques explained in the article (see ARTICLE).
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


# The article's techniques, each added to the previous row. Grouping uses the
# DP partition and verification uses hashing in every FDR row, since the article
# does not compare those choices. SIMD is whichever backend was measured.
ARTICLE = [
    ("naive", None),
    ("multiple Shift-OR", None),
    ("FDR (grouping + truncation)", (0, 1, False)),
    ("+ batch of 8 bytes", (0, 8, False)),
    ("+ super character (4 bits)", (4, 8, False)),
    ("+ SIMD", (4, 8, True)),
]


def article(case):
    """Return (label, entry) pairs for the ARTICLE rows."""
    techniques = {entry["name"]: entry for entry in case["techniques"]}
    rows = [
        (ARTICLE[0][0], techniques["naive"]),
        (ARTICLE[1][0], techniques["multiple-shift-or"]),
    ]
    for label, (bits, batch, simd) in ARTICLE[2:]:
        (entry,) = [
            entry
            for entry in case["comparison"]
            if entry["super_character_bits"] == bits
            and entry["batch_size"] == batch
            and entry["verification"] == "hash"
            and (entry["backend"] != "integer") == simd
        ]
        rows.append((label, entry))
    return rows


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

    names = [label for label, _ in article(case)]
    speeds = [mib_per_second(case, entry) for _, entry in article(case)]
    figure, axis = plt.subplots(figsize=(7, 0.35 * len(names) + 1))
    axis.barh(names[::-1], speeds[::-1], color="#4c72b0")
    axis.set_xscale("log")
    axis.set_xlabel("throughput (MiB/s, log scale)")
    axis.set_title(name)
    figure.tight_layout()
    figure.savefig(output / f"techniques-{name}.svg")
    plt.close(figure)
    lines += [f"![{name}](techniques-{name}.svg)", ""]

# Word datasets that differ only in pattern count form one scaling series.
series = {}
for case in report["cases"]:
    config = case["config"]
    if "text" in config:
        key = (config["text"], config["min_pattern_len"], config["max_pattern_len"])
        series.setdefault(key, []).append(case)
for (text, low, high), cases in series.items():
    if len(cases) < 2:
        continue
    cases.sort(key=lambda case: case["pattern_count"])
    name = f"{Path(text).stem}-{low}-{high}"
    counts = [case["pattern_count"] for case in cases]
    lines += [
        f"## パターン数による変化（{name}）",
        "",
        (
            f"`{text}` から長さ{low}〜{high}の単語を選んだ設定。値はMiB/s（括弧内は候補数）。"
            "記事の手法を上から順に加えたもので、FDRは動的計画法による分割とハッシュ検証に固定。"
        ),
        "",
    ]
    rows = []
    for i, (label, _) in enumerate(ARTICLE):
        row = [label]
        for case in cases:
            other = article(case)[i][1]
            cell = f"{mib_per_second(case, other):,.1f}"
            if "statistics" in other:
                cell += f"（{other['statistics']['candidate_count']:,}）"
            row.append(cell)
        rows.append(row)
    lines += table(["方式"] + [f"{count:,}個" for count in counts], rows)
    lines += [
        "",
        "| 一致数 | " + " | ".join(f"{c['match_count']:,}" for c in cases) + " |",
        "|---|" + "---:|" * len(cases),
        "",
    ]

    figure, axis = plt.subplots(figsize=(7, 4.5))
    for i, (label, _) in enumerate(ARTICLE):
        speeds = [mib_per_second(case, article(case)[i][1]) for case in cases]
        axis.plot(counts, speeds, marker="o", label=label)
    axis.set_xscale("log")
    axis.set_yscale("log")
    axis.set_xlabel("pattern count (log scale)")
    axis.set_ylabel("throughput (MiB/s, log scale)")
    axis.set_title(name)
    axis.legend()
    figure.tight_layout()
    figure.savefig(output / f"scaling-{name}.svg")
    plt.close(figure)
    lines += [f"![{name}](scaling-{name}.svg)", ""]

(output / "results.md").write_text("\n".join(lines))
