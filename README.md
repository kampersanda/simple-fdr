# simple-fdr

FDRの解説記事に対応する、Rustによる複数パターン検索の実験用実装です。
現在は開発計画の段階3まで実装しています。

## 段階0の内容

- バイト列のパターン集合と、パターンID・開始位置・終了位置を持つ一致結果。
- 全パターンを各入力位置で比較する参照実装。
- 固定seedでパターンと入力を生成し、指定位置に一致を埋め込む生成器。
- 生成データと実験設定・結果をJSONに保存するCLI。

開始位置は0始まり、終了位置は一致区間の直後です。重なり、包含関係、同じ内容の別IDを
すべて報告します。IDはパターン配列の添字です。参照実装の結果はID、開始位置の順で返します。
空パターンはエラーとし、空入力と空のパターン集合は扱います。

## 実行方法

Rustのedition 2024に対応したツールチェーンを使用します。

```sh
cargo test --locked
cargo run --locked -- --help
cargo run --locked -- generate --help
cargo run --locked --example reference
cargo run --locked --release -- generate experiments/stage0.json /tmp/fdr-dataset.json
cargo run --locked --release -- run experiments/stage0.json /tmp/fdr-result.json
```

出力先の親ディレクトリは事前に作成してください。既存のファイルには上書きしません。
再実行時は別の出力先を指定するか、不要な出力を削除してください。

ライブラリでは、任意のバイト列を直接検索できます。

```rust
use simple_fdr::{Patterns, naive::Naive};

let patterns = Patterns::new(vec![b"aa".to_vec(), b"a".to_vec()]).unwrap();
let matcher = Naive::new(patterns);
let matches = matcher.find_all(b"aaa");
assert_eq!(matches.len(), 5);
```

`scan`は一致ごとにコールバックを呼び、`find_all`は全一致を配列に格納します。
後続のアルゴリズムの正解確認にこの参照実装を使います。

## 段階1：単一パターンShift-OR

`shift_or::ShiftOr`は、記事と同じく開始状態を明示した説明用モデルです。
ビット0が開始状態、ビットjが先頭jバイトの一致を表し、活性状態を0で表します。
状態は `D = ((D << 1) | M[byte]) & state_mask` で更新し、受理ビットが0なら一致です。
開始状態は常に0で、初期状態ではそれ以外を1にします。

`u64`の1ワードを使うため、対応長は1〜63バイトです。長さ63では開始状態を含めて
64ビットを使います。空パターンと64バイト以上は構築時にエラーになります。
文字マスクは全256バイト分を持ち、状態とともに使用幅の下位ビットだけを表示します。
複数パターンと複数ワードへの拡張は段階2、原著の末尾から数える配置は後続の段階で扱います。

```rust
use simple_fdr::shift_or::ShiftOr;

let matcher = ShiftOr::new(b"acbc").unwrap();
let matches = matcher.find_all(b"aacbc");
assert_eq!((matches[0].start, matches[0].end), (1, 5));

let classes = vec![b"ac".to_vec(), b"c".to_vec(), b"b".to_vec(), b"c".to_vec()];
let matcher = ShiftOr::from_classes(&classes).unwrap();
assert_eq!(matcher.find_all(b"ccbc").len(), 1);
```

文字クラスは位置ごとの許容バイト列で渡します。`[ac]cbc`のような文字列表記を
解析する機能はありません。クラス内の重複は許容し、空クラスのあるパターンは一致しません。
単一パターンなので一致のIDは常に0です。`scan`と`find_all`は重なる一致も報告し、
呼び出しごとに初期状態から検索します。空入力には一致を返しません。

記事の文字マスクと状態列は、次のコマンドで確認できます。

```sh
cargo run --locked --example shift_or
```

`acbc`と入力`aacbc`では `11110 → 11100 → 11100 → 11010 → 10110 → 01110` と
遷移し、区間 `[1, 5)` で一致します。文字クラス`[ac]cbc`では `c`のマスクが
`01000`となり、入力`ccbc`にも一致します。
`initial_state()`、`mask(byte)`、`trace(input)`で同じ情報を取得できます。
トレースは入力1バイトごとの終了位置、入力バイト、文字マスク、更新後の状態、一致の有無を返します。
構築と検索は分離し、トレースの配列確保は通常検索には含めません。
実験用CLIの`run`は既定で段階0の参照実装を使います。

## 段階2：複数パターンをそのまま並べる

`multiple_shift_or::MultipleShiftOr`は、文字位置ごとに全パターンの状態を並べます。
パターン数をqとすると、パターンID kの先頭jバイトの状態はビット `j*q+k` です。
開始状態のqビットは常に0で、状態全体をqビット左にシフトして文字マスクとORします。
各パターンの受理位置は `pattern_length*q+k` です。短いパターンの受理位置より後ろは
ダミー状態とし、すべての文字マスクで1にして活性化を防ぎます。

最長長をLとすると、状態幅は `q*(L+1)` ビット、使用ワード数はその64分の1の切り上げです。
`Vec<u64>`の添字0に下位64ビットを格納し、上位ワードから更新してワード間の桁上がりを
引き継ぎます。qが64の倍数や64を超える場合も扱い、単一パターン版の63バイト制限はありません。
対応範囲はメモリと`usize`で表現できる状態幅に依存します。空集合は幅0で一致を返しません。

```rust
use simple_fdr::{Patterns, multiple_shift_or::MultipleShiftOr};

let patterns = Patterns::new(vec![b"abcd".to_vec(), b"ad".to_vec(), b"bc".to_vec()]).unwrap();
let matcher = MultipleShiftOr::new(patterns);
assert_eq!(matcher.state_width(), 15);
assert_eq!(matcher.word_count(), 1);
assert_eq!(matcher.accepting_bit(0), 12);
assert_eq!(matcher.accepting_bit(1), 7);
assert_eq!(matcher.accepting_bit(2), 8);
assert_eq!(matcher.find_all(b"abcdadbc").len(), 4);
```

`scan`と`find_all`は終了位置、IDの順で全一致を報告します。参照実装と結果を比較するときは
ソートしてください。`initial_state()`、`mask(byte)`、`trace(input)`で状態を確認できます。
トレースの各ステップは文字マスク、更新後の状態、その位置で終了する一致を持ちます。
空集合のトレースは空です。通常検索は状態の作業配列だけを確保し、トレース用の複製は行いません。

```sh
cargo run --locked --example multiple_shift_or
cargo run --locked --release -- run experiments/stage2.json /tmp/fdr-stage2.json --algorithm multiple-shift-or
cargo run --locked --release -- run experiments/stage2.json /tmp/fdr-stage2-naive.json --algorithm naive
```

記事例の受理ビットは12・7・8です。入力`abcdadbc`ではID 2が `[1, 3)`、ID 0が
`[0, 4)`、ID 1が `[4, 6)`、ID 2が `[6, 8)` に一致します。
`experiments/stage2.json`は長さ16の100パターンを生成するため、状態は1,700ビット、27ワードです。
`pattern_count`と長さの範囲を変えた設定を保存すれば、状態幅と検索コストを比較できます。
結果JSONには`max_pattern_len`、`state_width_bits`、`state_words`、構築・検索時間を保存します。
`state_width_bits`と`state_words`は複数パターンShift-ORと段階3のフィルタで出力します。
CLIの一致結果はどちらの方式もID、開始位置の順にそろえ、ソートは検索時間に含めません。

状態更新のコストは入力1バイトにつき使用ワード数に比例し、受理判定にはq個のビットを調べます。
マスク表のワードデータは `256 * state_words * 8` バイトです（Vec等の管理領域を除く）。
検索時間には状態配列の初期化、受理判定、一致配列の確保・格納が含まれます。
時間は単発の動作確認値で、安定した性能比較のための反復測定は後続の段階で追加します。

## 段階3：GroupingとTruncation

`filter::Filter`は、候補を抽出した後、バケット内の元パターンをすべて列挙して完全比較します。
Groupingは位置ごとの文字クラスを共有し、Truncationは各パターンの末尾をフィルタに使います。
長さが異なるパターンをまとめた場合は、各フィルタ長に受理位置を設けます。
開始状態と複数ワードの更新は段階2と共通です。段階4の共通末尾長・末尾から数える配置は使いません。

バケット数は1以上を指定し、パターン数を上限とします。ID順に連続するパターンをなるべく均等に
分け、余りは先頭のバケットから1個ずつ配ります。3パターン・2バケットなら `[0, 1]` と `[2]` です。
これは比較用の単純な振り分けです。Grouping方針の最適化は段階4で扱います。
空集合ではバケット数・状態幅とも0です。

| CLI方式 | バケット | フィルタに使う部分 |
|---|---|---|
| `grouping` | `--bucket-count`（既定2） | パターン全体 |
| `truncation` | 1パターンにつき1バケット | 各パターンの末尾、最大 `--filter-length` バイト（既定2） |
| `grouping-truncation` | `--bucket-count`（既定2） | 各パターンの末尾、最大 `--filter-length` バイト（既定2） |

適用しない方式のオプションは使いません。フィルタ長も1以上を指定し、短いパターンは全体を使います。
ライブラリでは `Filter::new(patterns, bucket_count, filter_length)` に `NonZeroUsize` の
バケット数と `Option<NonZeroUsize>` の末尾長を渡します。`None`なら切り詰めません。
Truncationのみならパターン数以上のバケット数を渡します。

状態幅は、実際のバケット数と「全バケット中の最大フィルタ長に1を足した値」の積です。
バケットごとの短い列にはダミー状態を置きます。記事の `abcd`、`ad`、`bc` では、
段階2が15ビット、Groupingのみが10ビット、Truncationのみ（末尾2文字）が9ビット、
組合せが6ビットです。Groupingのみは `ab` や `adcd`、Truncationのみは `acd` の
末尾 `cd` を候補として受理しますが、元パターンとの検証によって偽陽性を除外します。

```sh
cargo run --locked --example filter
cargo run --locked --release -- run experiments/stage3.json /tmp/fdr-stage3-naive.json --algorithm naive
cargo run --locked --release -- run experiments/stage3.json /tmp/fdr-stage3-multiple.json --algorithm multiple-shift-or
cargo run --locked --release -- run experiments/stage3.json /tmp/fdr-stage3-grouping.json --algorithm grouping --bucket-count 8
cargo run --locked --release -- run experiments/stage3.json /tmp/fdr-stage3-truncation.json --algorithm truncation --filter-length 4
cargo run --locked --release -- run experiments/stage3.json /tmp/fdr-stage3-combined.json --algorithm grouping-truncation --bucket-count 8 --filter-length 4
```

`scan_candidates`でバケットIDと終了位置を取得できます。同じ終了位置で複数の受理位置が
活性化しても、そのバケットの候補イベントは1回です。`scan`と`find_all`は検証済みの
全一致を終了位置、IDの順に返し、重複内容の別IDも残します。

結果JSONの `filter` に次を保存します。

- `requested_bucket_count`、`filter_length`：その方式で使った構築設定。
- `buckets`：実際のバケットごとのパターンID。
- `accepting_lengths`：バケットごとの受理位置に対応するフィルタ長。
- `candidate_count`：バケットと終了位置の組の数。
- `full_comparisons`：元パターンと入力区間を比較した回数。終了位置より長いパターンは比較せず、数えません。
- `false_positive_candidates`：検証で一致を一つも含まなかった候補数。一致数を候補数から引いて求めません。

`statistics(input)`は別の走査で収集し、CLIの `search_ns` にカウンタ更新の負荷を含めません。
マスク表のワードデータは段階2と同じく `256 * state_words * 8` バイトです。
状態更新は入力1バイトあたり使用ワード数に比例し、受理判定は各バケットの異なるフィルタ長の数、
検証は候補バケット内のパターン数に依存します。短いフィルタや大きいバケットで候補が増えると、
状態幅が縮んでも検証時間が増えることがあります。対応範囲はメモリと `usize` の状態幅に依存します。
時間は引き続き単発サンプルで、性能向上を保証する測定ではありません。

上の `experiments/stage3.json`、バケット数8、フィルタ長4で得た比較です。
全5方式の一致結果は同じ2,884件でした。以下の幅と件数は時間計測に依存しません。

| 方式 | 状態幅（ビット） | 候補イベント | 完全比較 | 一致なしの候補 |
|---|---:|---:|---:|---:|
| 複数パターンShift-OR | 1,700 | — | — | — |
| Grouping | 136 | 515,002 | 6,438,008 | 512,118 |
| Truncation | 500 | 25,777 | 25,776 | 22,893 |
| Grouping + Truncation | 40 | 491,488 | 6,143,094 | 488,604 |

この設定は4文字のアルファベットに100パターンを生成します。Groupingすると文字クラスが
大きくなり、多くの入力区間が候補に残るため、幅の削減がそのまま検索の高速化にはつながりません。
Truncationの完全比較が候補より1回少ないのは、入力先頭付近で元パターン長に足りない候補を
長さだけで除外したためです。

## 生成設定

設定例は`experiments/stage0.json`にあります。

| フィールド | 意味 |
|---|---|
| `seed` | 64ビット整数のseed |
| `input_len` | 入力のバイト数 |
| `pattern_count` | パターン数 |
| `min_pattern_len`, `max_pattern_len` | 生成するパターン長の範囲。両端を含む |
| `alphabet` | 使用するバイト値の配列。省略すると0から255 |
| `insertions` | 埋め込むパターンIDと開始位置の配列。省略すると埋め込みなし |

長さの範囲は1以上、アルファベットは空でないものとします。長さとアルファベットの
添字は一様に選びます。同じバイトを複数回登録すれば、出現頻度に重みを付けられます。
パターン内容の重複は許容します。

生成器のversion 2では`rand_chacha`のChaCha8と`rand`の一様サンプリングを使い、
パターン、入力の順で生成した後に一致を埋め込みます。同じversionと設定なら同じ
バイト列になります。再現時は、対応するソースと`Cargo.lock`を使用してください。
旧version 1（SplitMix64）とは生成結果が異なります。
埋め込みは隣接可能ですが、重なり、存在しないID、入力範囲外の指定はエラーになります。

`generate`の出力には、生成器のversion、設定、パターン、入力、埋め込んだ一致を保存します。
バイト列はJSONの整数配列です。`planted_matches`は確実に存在する一致で、
偶然生じた一致は含まないため、全一致の一覧ではありません。

## 実験結果

`run`は設定からデータを再生成し、選択した検索方式による結果を保存します。
`--algorithm`で`naive`（既定値）、`multiple-shift-or`、`grouping`、`truncation`、
`grouping-truncation`を選べます。
出力には形式・生成器・パッケージのversion、OS・アーキテクチャ、debug assertionsの有無、
設定、入力長、パターン数、構築・検索時間（ナノ秒）、全一致、埋め込んだ一致が含まれます。

現段階の時間は動作確認用の単発サンプルです。検索時間には一致配列の確保と格納を含み、
データ生成、ファイル読み込み、JSON保存は含みません。構築時間はパターン検査、最長長の取得、選択した検索方式の
作成にかかる時間です。ウォームアップ・反復計測やCPUの詳細記録は今後追加します。

原著型のフィルタ配置、SIMD、実ファイルを読み込むCLIは後続の段階で実装します。

## 使用するライブラリ

- `clap`：引数とサブコマンドの定義、ヘルプ、バージョン表示。
- `anyhow`：CLIのエラー伝播と、失敗したファイルパスの表示。
- `thiserror`：ライブラリのエラー型の定義。
- `rand`と`rand_chacha`：固定seedの乱数生成と一様サンプリング。
- `serde`と`serde_json`：設定・データ・結果のJSON入出力。
- `tempfile`（テスト用）：一時ディレクトリの作成と自動削除。

## 検証

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

手作りの正解、全256バイト、重なり・重複・包含関係、空入力、固定seedの既知データ、
埋め込みの境界条件、JSON保存とCLIの終了コードを確認します。

段階1では記事のマスク・状態列、長さ1〜63、全256バイト、空入力、重なる一致、
固定seedの文字列と文字クラスを検証します。文字列は参照実装、文字クラスは
各入力窓と許容バイト集合の直接比較を正解に使います。

段階2では記事例の状態列とダミー状態、1,700ビットの幅、パターン数63・64・65や127・128・129、
長さ63・64・65を含むワード境界、全256バイト、重複・包含・重なり、空集合を検証します。
固定seedで長さの異なるパターンを生成し、一致を埋め込んだ入力で参照実装と差分比較します。
CLIの結果と状態幅の保存も確認します。

段階3では記事の偽陽性の除去、異なる受理位置の候補イベントの重複排除、完全比較回数、
全256バイト、長さ63・64・65、バケット数63・64・65や127・128・129のワード境界を検証します。
固定seedによる混在長・埋め込み一致の参照比較と、全3方式のCLI設定・統計の保存も確認します。
