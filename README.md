# simple-fdr

FDRの解説記事に対応する、Rustによる複数パターン検索の実験用実装です。
Shift-ORから始めて、Grouping・Truncation、原著の末尾基準の配置、複数バイトの一括処理、
Super Character、ハッシュによる検証、NEON／SSE2によるマスク累積までを実装しています。

## 共通の約束

- パターンIDはパターン配列の添字です。同じ内容のパターンも別IDとして扱います。
- 一致は `[start, end)` の半開区間で、開始位置は0始まりです。
- 重なり、包含関係、同じ内容の別IDをすべて報告します。
- 空パターンはエラーです。空入力と空のパターン集合は扱います。
- 各検索器は `scan`（一致ごとにコールバック）と `find_all`（配列に格納）を持ちます。
  報告順は検索器ごとに異なるため、結果を比べるときはソートしてください。

## 実行方法

Rustのedition 2024に対応したツールチェーンを使用します。

```sh
cargo test --locked
cargo run --locked -- --help
cargo run --locked --release -- generate experiments/small.json /tmp/fdr-dataset.json
cargo run --locked --release -- run experiments/small.json /tmp/fdr-result.json
```

出力先の親ディレクトリは事前に作成してください。既存のファイルには上書きしません。

## 検索器

### 参照実装

`naive::Naive`は全パターンを各入力位置で比較します。他の検索器の正解確認に使います。

```rust
use simple_fdr::{Patterns, naive::Naive};

let patterns = Patterns::new(vec![b"aa".to_vec(), b"a".to_vec()]).unwrap();
let matches = Naive::new(patterns).find_all(b"aaa");
assert_eq!(matches.len(), 5);
```

### 単一パターンShift-OR

`shift_or::ShiftOr`は、記事と同じく開始状態を明示した説明用モデルです。
ビット0が開始状態、ビットjが先頭jバイトの一致を表し、活性状態を0で表します。
状態は `D = ((D << 1) | M[byte]) & state_mask` で更新し、受理ビットが0なら一致です。
`u64`の1ワードを使うため、対応長は1〜63バイトです。

```rust
use simple_fdr::shift_or::ShiftOr;

let matcher = ShiftOr::new(b"acbc").unwrap();
assert_eq!(matcher.find_all(b"aacbc")[0].start, 1);

let classes = vec![b"ac".to_vec(), b"c".to_vec(), b"b".to_vec(), b"c".to_vec()];
let matcher = ShiftOr::from_classes(&classes).unwrap();
assert_eq!(matcher.find_all(b"ccbc").len(), 1);
```

文字クラスは位置ごとの許容バイト列で渡します。文字列表記の解析はしません。
`initial_state()`、`mask(byte)`、`trace(input)`で記事のマスクと状態列を確認できます
（`cargo run --example shift_or`）。

### 複数パターンShift-OR

`multiple_shift_or::MultipleShiftOr`は文字位置ごとに全パターンの状態を並べます。
パターン数をq、最長長をLとすると、パターンkの先頭jバイトの状態はビット `j*q+k`、
状態幅は `q*(L+1)` ビットです。短いパターンの受理位置より後ろはダミー状態として
常に1にします。状態は `Vec<u64>` に下位ワードから格納し、ワード数に制限はありません。
記事例 `abcd`・`ad`・`bc` では受理ビットが12・7・8です（`cargo run --example multiple_shift_or`）。

### GroupingとTruncation（説明用モデル）

`filter::Filter`は、複数パターンShift-ORで候補を抽出し、バケット内の全パターンと完全比較します。
Groupingは位置ごとの文字クラスをバケット内で共有し、Truncationは各パターンの末尾だけを
フィルタに使います。長さが異なるパターンには、それぞれの長さに受理位置を設けます。
バケットはID順に個数を均等に分けます。

| CLI方式 | バケット | フィルタに使う部分 |
|---|---|---|
| `grouping` | `--bucket-count` | パターン全体 |
| `truncation` | 1パターンにつき1バケット | 各パターンの末尾 `--filter-length` バイト |
| `grouping-truncation` | `--bucket-count` | 各パターンの末尾 `--filter-length` バイト |

記事例では、複数パターンShift-ORが15ビット、Groupingのみが10ビット、
Truncationのみ（末尾2文字）が9ビット、組合せが6ビットです（`cargo run --example filter`）。

### FDR

`fdr_filter::FdrFilter`は原著（§4.1、Figure 8–9、Algorithm 1）の配置による実装です。
バケットbの実効末尾長は `ell[b] = min(指定フィルタ長, バケット内の最短パターン長)` で、
元パターンの末尾から同じ距離にある文字をまとめてマスクを作ります。

有効バケット数をq、末尾からの距離をdとすると、ビット位置は `d*q+b` です。
マスクは許容する文字を0、それ以外を1にし、`d >= ell[b]` のpaddingは常に0です。
初期状態は各バケットの `d < ell[b]-1` を1にして、入力先頭より前から始まる候補を防ぎます。
1文字ごとに次を実行します。

1. `R |= mask[byte]` で現在文字の制約を加える。
2. 行0の0ビットを `(bucket_id, end)` の候補として報告する。
3. `R >>= q` で次の文字位置へ進む。

候補はバケット内の元パターン全体と比較して検証します。

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
    NonZeroUsize::new(2).unwrap(),
    NonZeroUsize::new(8).unwrap(),
    Strategy::DynamicProgramming,
    Coefficients::default(),
)
.with_batch_size(BatchSize::Eight)
.with_super_character(SuperCharacterBits::Four)
.with_verification(Verification::Hash);
assert_eq!(filter.find_all(b"abcdadbc").len(), 4);
```

`scan_candidates`で候補、`statistics`で候補数・完全比較回数などの統計を取得できます
（`cargo run --example fdr_filter`）。以下の各オプションは自由に組み合わせられ、
検索結果と候補列は変わりません（Super Characterは候補を減らします）。

#### Grouping方針

`grouping::partition` はバケット数を指定数とパターン数の小さい方とし、空バケットを作りません。

| 方針 | 分割方法 | 構築の計算量（nパターン、qバケット） |
|---|---|---|
| `balanced` | ID順に個数を均等化 | O(n) |
| `length-sorted` | 長さ順に並べてから個数を均等化 | O(n log n) |
| `dynamic-programming`（既定） | 長さ順の連続区間を経験的コストの和で最適化 | O(n log n + q n²) |

DPの区間コストは原著と同じ `size^alpha / min_length^beta`（既定α=1.05、β=3）で、
`min_length`には切り詰める前の元パターン長を使います。このコストに対する最適化であり、
検索時間の最小化は保証しません。

#### 一括処理（`with_batch_size`）

B文字（1・2・4・8）のマスクを `A = R | M[c0] | (M[c1] << q) | ...` で合成し、
Aの各行から候補を取り出してから `R = A >> (B*q)` で次のブロックへ進みます。
状態幅は `q*(L+B-1)` ビットです（Lは最大実効末尾長）。q=L=B=8なら120ビット・2ワードです。

#### Super Character（`with_super_character`）

追加ビット数s（0・4・8）について、`(現在文字 << s) | (次文字の下位sビット)` をキーとする表を
使い、同じパターン内の文字対だけを許可します。パターン末尾は次文字の全組合せを許可し、
入力末尾では通常のマスクを使います。表サイズはq=L=8のとき0・4・8ビットで
2,048・34,816・526,336バイトです。

#### ハッシュ検証（`with_verification`）

`Verification::Hash`は、バケットごとに共通末尾長のバイト列をキーとする
`hashbrown::HashMap`を作り、候補の末尾で引いたパターンだけを全文比較します。
共通末尾が同じ別パターンが残るため、全文比較は引き続き必要です。

#### SIMD（`with_backend`）

`Backend::Neon`（aarch64）と`Backend::Sse2`（x86_64）は、一括処理のマスク累積を
128ビットベクトルで行います。ビット配置、候補抽出、状態シフト、検証は整数版と共通です。
非対応のバックエンドは構築時にエラーになります。SSE2版はクロスビルドと生成命令の確認のみで、
x86_64実機では未検証です。

## CLI

`generate`は設定からデータを生成してJSONに保存し、`run`は同じデータで検索した結果を保存します。
`--algorithm`は `naive`（既定）、`multiple-shift-or`、`grouping`、`truncation`、
`grouping-truncation`、`fdr-sequential`、`fdr-batched` から選びます。
FDRの各オプションは `cargo run -- run --help` を参照してください。

```sh
cargo run --locked --release -- run experiments/abcdqrst-2-6.json /tmp/fdr.json \
  --algorithm fdr-batched --bucket-count 8 --filter-length 8 \
  --batch-size 8 --super-character-bits 4 --verification hash --backend neon
```

結果JSONには、バージョン、OS・アーキテクチャ、設定、構築・検索時間（ナノ秒）、全一致、
埋め込んだ一致、各方式の構築設定と統計（候補数、完全比較回数、偽陽性の候補数、表サイズなど）を
保存します。CLIの時間は単発の動作確認値です。

### 生成設定

| フィールド | 意味 |
|---|---|
| `seed` | 64ビット整数のseed |
| `input_len` | 入力のバイト数 |
| `pattern_count` | パターン数 |
| `min_pattern_len`, `max_pattern_len` | パターン長の範囲。両端を含む |
| `alphabet` | 使用するバイト値の配列。省略すると0から255 |
| `insertions` | 埋め込むパターンIDと開始位置の配列。省略すると埋め込みなし |

生成器はChaCha8と`rand`の一様サンプリングを使い、パターン、入力の順で生成してから
一致を埋め込みます。同じ設定と`Cargo.lock`なら同じバイト列になります。
埋め込みの重なり、存在しないID、入力範囲外の指定はエラーです。
`planted_matches`は確実に存在する一致で、全一致の一覧ではありません。

### 実テキストからの単語抽出

`text` を持つ設定は、テキストファイル全体を入力とし、そこから抜き出した単語をパターンにします。

| フィールド | 意味 |
|---|---|
| `seed` | 64ビット整数のseed |
| `text` | テキストファイル。相対パスは設定ファイルのディレクトリを基準にする |
| `pattern_count` | パターン数 |
| `min_pattern_len`, `max_pattern_len` | 単語長の範囲。両端を含む |

単語はASCII英字の最長の連続で、大文字・小文字を区別します。長さが範囲内の異なり語を
辞書順に並べ、ChaCha8と`rand::seq::index::sample`で重複なく選びます。
パターンIDは選んだ順です。単語は他の単語の一部にも一致するため（`the`と`there`など）、
一致は参照実装で数えます。`planted_matches`は空です。

`data/sherlock.txt`（Project Gutenberg、『The Adventures of Sherlock Holmes』、594,915バイト、
CRLF改行のASCII）は、異なり語が8,788語です。

## 実験

```sh
cargo build --locked --release --example bench
mkdir -p experiments/results
python3 experiments/bench.py target/release/examples/bench > experiments/results/bench.json
uv run experiments/report.py experiments/results/bench.json experiments/results
cargo build --locked --release
python3 experiments/partition.py target/release/simple-fdr > experiments/results/partition.json
```

`bench`は設定ごとに二種類の計測をします。いずれもバケット数8、指定フィルタ長8です。

- `techniques`：各技法を個別に評価します。参照実装、複数パターンShift-OR、説明用モデルの
  Grouping・Truncation・組合せ、FDRの各Grouping方針、DP分割の逐次FDRから一つだけ
  オプションを変えたもの（追加4ビット、ハッシュ検証、一括8、一括8＋SIMD）、全部の組合せです。
- `comparison`：追加ビット数・一括処理長・検証方法・利用可能なバックエンドの全組合せです。

いずれも参照実装との一致を、FDRは候補列の一致も確認してから、3回のウォームアップ後に9回計測し
中央値を求めます。各一致は`black_box`に渡して数えるだけで、構築・統計収集・正解照合は
計測に含めません。`report.py`は`bench.py`の出力から表（`results.md`）と技法ごとの
スループットのグラフ（SVG）を作ります。同じテキスト・単語長の範囲でパターン数だけが異なる設定は
まとめて、パターン数に対するスループットと候補数の表とグラフも作ります。
設定ごとの表には計測したすべての構成を載せますが、グラフとパターン数の表は、解説記事で説明する
手法を順に加えた6構成に絞ります（参照実装、複数パターンShift-OR、Grouping・Truncationを使うFDR、
＋一括8バイト、＋Super Character 4ビット、＋SIMD）。記事で比較していないGrouping方針と検証方法は、
それぞれ動的計画法とハッシュ検証に固定します。依存するmatplotlibはスクリプト内のメタデータで指定しており、
`uv run`が用意します。`experiments/results/`は計測環境に依存するため、リポジトリに含めません。
`partition.py`は`sherlock-1000-2-16.json`でのGrouping方針ごとの結果と、ランダムに生成した
パターン集合（100・1,000・10,000個）での構築時間を調べます。

設定は `experiments/` にあります。`bench.py`はSherlockの四つの設定と、Super Characterの効果を
見るための人工的な`abcdqrst-2-6.json`を計測します。`sherlock-{10,100,1000}-4-16.json`は
単語長の範囲とseedが同じで、パターン数だけが異なります。8バケット、指定フィルタ長8、DP分割での候補数は
次のとおりです。

| 設定 | 入力・パターン | 追加0ビット | 4ビット | 8ビット |
|---|---|---:|---:|---:|
| `sherlock-10-4-16.json` | Sherlockの全文・10語・4〜10バイト | 130 | 130 | 130 |
| `sherlock-100-4-16.json` | Sherlockの全文・100語・4〜13バイト | 15,867 | 1,631 | 1,607 |
| `sherlock-1000-4-16.json` | Sherlockの全文・1,000語・4〜15バイト | 356,084 | 57,644 | 43,923 |
| `sherlock-1000-2-16.json` | Sherlockの全文・1,000語・2〜15バイト | 436,517 | 99,639 | 79,164 |
| `abcdqrst-2-6.json` | a〜d・q〜tのランダム列・100個・2〜6バイト | 229,797 | 57,358 | 30,528 |

単語長は4〜16または2〜16と指定していますが、表には選ばれた単語の実際の範囲を示しています。
一致数は上から130件、780件、10,154件、35,141件です。

`abcdqrst-2-6.json`は下位4ビットが同じ文字（aとqなど）を含むため、8ビットでさらに候補が減ります。
候補が減っても表の構築・参照コストが増えるため、追加ビット数や一括処理長を増やしても
常に速くなるとは限りません。

## 検証

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
ruff check experiments
```

各検索器を参照実装と照合し、FDRの候補列はビット演算を使わない独立した計算と比較します。
手作りの例、全256バイト、重なり・重複・包含関係、空入力・空集合、ワード境界やブロック境界、
固定seedのランダム入力、全オプションの組合せ、CLIの保存内容とエラー処理を確認します。

## 使用するライブラリ

- `hashbrown`：ハッシュ検証の表。
- `clap`：CLIの引数定義。
- `anyhow`、`thiserror`：エラー処理。
- `rand`、`rand_chacha`：固定seedの乱数生成と単語の抽出。
- `serde`、`serde_json`：JSON入出力。
- `tempfile`（テスト用）：一時ディレクトリ。
