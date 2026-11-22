# simple-fdr

FDRの解説記事に対応する、Rustによる複数パターン検索の実験用実装です。
Shift-ORから始めて、Grouping・Truncation、複数バイトの一括処理、Super Character、
ハッシュ検証、NEON／SSE2によるマスク累積までを段階的に実装しています。

## 使い方

Rust（edition 2024）が必要です。

```sh
cargo test --locked
cargo run --locked --release -- generate experiments/small.json /tmp/fdr-dataset.json
cargo run --locked --release -- run experiments/small.json /tmp/fdr-result.json
```

`run`は `--algorithm` で検索器を選べます（既定は `naive`）。オプションは `--help` を参照してください。

```sh
cargo run --locked --release -- run experiments/abcdqrst-2-6.json /tmp/fdr.json \
  --algorithm fdr-batched --bucket-count 8 --filter-length 8 \
  --batch-size 8 --super-character-bits 4 --verification hash --backend neon
```

出力先の親ディレクトリは事前に作成してください。既存のファイルは上書きしません。

## 検索器

| モジュール | 内容 | 例 |
|---|---|---|
| `naive` | 全パターンを各位置で比較する参照実装 | |
| `shift_or` | 単一パターンShift-OR（文字クラス対応、1〜63バイト） | `cargo run --example shift_or` |
| `multiple_shift_or` | 複数パターンShift-OR | `cargo run --example multiple_shift_or` |
| `filter` | Grouping・Truncationの説明用モデル | `cargo run --example filter` |
| `fdr_filter` | 原著の配置によるFDR | `cargo run --example fdr_filter` |

一致は `[start, end)` の半開区間で、重なりや同じ内容の別パターンもすべて報告します。
報告順は検索器ごとに異なります。

```rust
use simple_fdr::{
    Patterns,
    fdr_filter::{BatchSize, FdrFilter, SuperCharacterBits, Verification},
    grouping::{Coefficients, Strategy},
};
use std::num::NonZeroUsize;

let patterns = Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap();
let filter = FdrFilter::new(
    patterns,
    NonZeroUsize::new(2).unwrap(), // バケット数
    NonZeroUsize::new(8).unwrap(), // フィルタ長
    Strategy::DynamicProgramming,
    Coefficients::default(),
)
.with_batch_size(BatchSize::Eight)
.with_super_character(SuperCharacterBits::Four)
.with_verification(Verification::Hash);
assert_eq!(filter.find_all(b"abcdadbc").len(), 4);
```

FDRのオプションは自由に組み合わせられます。

- **Grouping方針**：`balanced`、`length-sorted`、`dynamic-programming`（既定、原著のコスト関数）
- **一括処理**（`with_batch_size`）：1・2・4・8文字のマスクをまとめて累積
- **Super Character**（`with_super_character`）：次文字の下位0・4・8ビットを加えて候補を減らす
- **ハッシュ検証**（`with_verification`）：候補の末尾で引いたパターンだけを全文比較
- **SIMD**（`with_backend`）：NEON（aarch64）またはSSE2（x86_64）。SSE2版は実機で未検証です。

## データ

`generate`・`run`の設定はJSONで書きます（例は `experiments/`）。
ランダムなパターンと入力を生成する設定と、テキストファイルから単語を抜き出してパターンにする
設定（`text`フィールド）があります。同じ設定と`Cargo.lock`なら同じデータになります。
`data/sherlock.txt`はProject Gutenbergの『The Adventures of Sherlock Holmes』です。

## 実験

```sh
cargo build --locked --release --example bench
mkdir -p experiments/results
python3 experiments/bench.py target/release/examples/bench > experiments/results/bench.json
uv run experiments/report.py experiments/results/bench.json experiments/results
```

`bench.py`はSherlockの単語を使う設定と、Super Characterの効果を見る`abcdqrst-2-6.json`を計測し、
`report.py`が表（`results.md`）とスループットのグラフ（SVG）を作ります。
Grouping方針の比較は `experiments/partition.py` で行えます。

## 開発

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
ruff check experiments
```

各検索器は参照実装と照合してテストしています。
