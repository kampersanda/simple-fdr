# simple-fdr

FDRの解説記事に対応する、Rustによる複数パターン検索の実験用実装です。
現在は開発計画の段階1まで実装しています。

## 段階0の内容

- バイト列のパターン集合と、パターンID・開始位置・終了位置を持つ一致結果。
- 全パターンを各入力位置で比較する参照実装。
- 固定seedでパターンと入力を生成し、指定位置に一致を埋め込む生成器。
- 生成データと実験設定・結果をJSONに保存するCLI。

開始位置は0始まり、終了位置は一致区間の直後です。重なり、包含関係、同じ内容の別IDを
すべて報告します。IDはパターン配列の添字です。結果はID、開始位置の順で返します。
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
原著の末尾から数える配置や、複数ワードへの拡張は後続の段階で扱います。

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
実験用CLIの`run`は段階0の参照実装を引き続き使います。

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

`run`は設定からデータを再生成し、参照実装による検索結果を保存します。
出力には形式・生成器・パッケージのversion、OS・アーキテクチャ、debug assertionsの有無、
設定、入力長、パターン数、構築・検索時間（ナノ秒）、全一致、埋め込んだ一致が含まれます。

現段階の時間は動作確認用の単発サンプルです。検索時間には一致配列の確保と格納を含み、
データ生成、ファイル読み込み、JSON保存は含みません。構築時間はパターン検査と参照実装の
作成にかかる時間です。ウォームアップ・反復計測やCPUの詳細記録は今後追加します。

FDRのフィルタ、SIMD、実ファイルを読み込むCLIは後続の段階で実装します。

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
