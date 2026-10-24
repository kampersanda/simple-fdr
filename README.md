# simple-fdr

FDRの解説記事に対応する、Rustによる複数パターン検索の実験用実装です。
現在は開発計画の段階8のNEON／SSE2版まで実装しています。SSE2はクロスビルドと生成命令の確認までで、実機では未測定です。

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
`state_width_bits`と`state_words`は複数パターンShift-ORと段階3〜6のフィルタで出力します。
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

## 段階4：原著型の配置とGrouping方針

`fdr_filter::FdrFilter`は、原著§4.1、Figure 8–9、Algorithm 1の配置を
1バイトずつ処理する版（B=1）です。バケットbの実効末尾長は
`ell[b] = min(指定フィルタ長, バケット内の最短パターン長)`です。
元パターンの末尾から同じ距離にある文字をまとめてマスクを作り、候補は元パターン全体で検証します。

説明用モデルとの差を、パターン `abcd`・`ad`・`bc`、ID順の2バケット、指定長8で比較します。

| 項目 | 段階3のGrouping | 段階4のbalanced |
|---|---|---|
| バケット | `[0, 1]`, `[2]` | `[0, 1]`, `[2]` |
| フィルタに使う部分 | `abcd`・`ad`, `bc` | `cd`・`ad`, `bc` |
| 長さ | バケット0で2と4を受理 | バケットごとに共通長2 |
| 状態幅 | 開始状態を含む10ビット | 開始状態を持たない4ビット |
| ビットの意味 | 先頭から何文字一致したか | 現在位置から何文字先で候補が終わるか |

有効バケット数をq、末尾からの距離をd（末尾で0）とすると、ビット位置は `d*q+b` です。
行0が最下位のqビットで、マスクでは許容する文字を0、それ以外を1にします。
`d >= ell[b]` のpaddingは、すべての文字に対して0です。過去の入力が、短い末尾長の候補に
余分な制約を加えるのを防ぎます。使用する行数は `max(ell)`、幅は `q*max(ell)` です。
指定長が全パターンより長い場合、使わない上位行は確保しません。

初期状態は、各バケットの `d < ell[b]-1` のビットを1、それ以外を0にします。
これは入力先頭より前の文字を必要とする候補を防ぎます。1文字ごとに次を実行します。

1. `R |= mask[byte]` で現在文字の制約を加える。
2. 行0の0ビットを `(bucket_id, end)` の候補として報告する。
3. `R >>= q`（上位は0で埋める）で次の文字位置へ進む。

`Vec<u64>`でqが64以上の場合やワード境界も扱います。マスク表の未使用上位ビットは0です。
原著の8バケットでは1行が1バイトですが、この実装の1行はqビットです。
原著の8文字分のマスク合成・120ビットの範囲・途中位置の候補抽出は段階5で扱います。
空集合ではバケット数と幅が0になり、候補を返しません。

Groupingは `grouping::partition` で次の3方針を選べます。バケット数は指定数とパターン数の
小さい方とし、空バケットを作りません。元のIDを保ち、同じ長さのパターンはID順です。

| 方針 | 分割方法 | 構築の計算量（nパターン、qバケット） |
|---|---|---|
| `balanced` | ID順に個数を均等化。余りは先頭から配る | O(n) |
| `length-sorted` | 長さ順に並べてから個数を均等化 | O(n log n) |
| `dynamic-programming`（既定） | 長さ順の連続区間を経験的コストの和で最適化 | O(n log n + q n²) |

DPの区間 `[i, j)` のコストは、原著と同じ `size^alpha / min_length^beta`、
既定値はα=1.05、β=3です。`min_length`には切り詰める前の元パターン長を使います。
先頭jパターンをbバケットに分ける最小コストを `D[b,j]` とし、
`D[b,j] = min_i(D[b-1,i] + cost(i,j))` を非空区間だけで計算します。
`D[0,0]=0`、その他の到達できない状態は無限大です。
この実装では単独パターンの区間も許可し、残りのバケット数だけパターンを残します。
同コストでは最初に調べた小さい分割位置を採用します。

コストはf64で計算し、全区間に共通の正の係数で正規化してオーバーフローを避けます。
αは有限の正数、βは有限の非負数です。極端な係数では丸めやアンダーフローにより
区間コストの差が失われることがあります。検索結果の正しさには影響しません。
累乗は事前計算し、コスト配列は2行、復元用の分割位置は全行を保存します。
メモリはO(q n)、遷移数は `q*(n-q+1)*(n-q+2)/2` です（q=1とq=nは直接分割）。
長さ順の連続分割とこのコスト関数に対する最適化であり、検索時間の最小化は保証しません。
文字の分布や共通接尾辞、指定フィルタ長による情報損失も、このコストには入りません。

```sh
cargo run --locked --example fdr_filter
cargo run --locked --release -- run experiments/stage4.json /tmp/fdr-stage4.json --algorithm fdr-sequential --bucket-count 8 --filter-length 8 --grouping-strategy dynamic-programming --alpha 1.05 --beta 3
cargo build --locked --release
python3 experiments/stage4.py target/release/simple-fdr > /tmp/fdr-stage4-report.json
```

ライブラリでは `FdrFilter::new(patterns, bucket_count, filter_length, strategy, coefficients)` に
`NonZeroUsize`のバケット数・指定長、`grouping::Strategy`、`grouping::Coefficients`を渡します。
`scan_candidates`、`scan`、`find_all`、`statistics`は段階3と同じ役割です。
一致は終了位置、バケット、バケット内の順に返します。参照比較にはソートしてください。
`initial_state()`と`mask(byte)`で下位ワードから状態とマスクを確認できます。

結果JSONの`fdr_filter`に方針、係数、バケットの元ID一覧、`effective_lengths`、候補数、
完全比較回数、真の一致を含まなかった候補数、`mask_table_bytes`を保存します。
表サイズは `256 * state_words * 8` バイトのマスク本体だけで、Vec管理領域・パターン・
検証データは含みません。`construction_ns`にはDPとマスク構築を含め、検索と統計収集は分離します。

`experiments/stage4.py`は固定seedの3方針を参照実装と比較した後、空入力で
100・1,000・10,000パターンの構築時間を調べます。
## 段階5：複数入力の一括処理

段階4と同じ`FdrFilter`に`.with_batch_size(BatchSize::Eight)`を指定すると、
通常の`u64`演算で複数文字のマスクを合成します。`BatchSize`は`One`・`Two`・`Four`・`Eight`です。
`new`の既定は`One`で、段階4の逐次処理を使います。マスク・バケット・初期状態・
候補の完全比較を共有し、処理長の変更で再構築しません。SIMD命令を明示的に使用する実装ではありません。

qを実際のバケット数、Lを最大実効末尾長、Bを一括処理長とします。
ブロックの実際の文字数がnなら、次の処理を行います。

1. `A = R | M[c0] | (M[c1] << q) | ... | (M[c(n-1)] << ((n-1)*q))`を計算する。
2. 各jについて、Aの行jにある0ビットを終了位置`block_start+j+1`の候補として報告する。
3. `R = A >> (n*q)`で次ブロックへ引き継ぐ（上位を0で埋める）。

ブロック内での候補報告順も、終了位置、バケット、バケット内の順です。
後続文字のマスクは上位にずらすため、既に終了した候補を上書きしません。
共通末尾長2のバケットに`ab`と`acd`を入れた入力`abxx`でも、2文字目の候補を保持します。
前ブロックの制約はRに残るため、境界をまたぐ一致も検出します。
初期状態は段階4と同じで、増やした上位領域には0を置きます。
入力は安全なスライスの`chunks(B)`で読み、端数では存在するn文字だけを処理します。
入力先頭より前や末尾より後の読み取りはありません。

状態幅は`q*(L+B-1)`ビット、確保ワード数はその64ビット単位の切り上げです。
q=L=B=8なら120ビット・2ワードです。マスク表は段階4のままで、
`256 * ceil(q*L/64) * 8`バイトです。この条件では2,048バイトです。
記事の説明用モデルの開始状態を含む128ビットとは、数える対象が異なります。
単に状態幅を広げるだけでなく、マスクの移動方向と各行の候補抽出が必要です。
空のパターン集合は幅・表サイズとも0です。qやLは1ワードに限定しません。
`initial_state()`は逐次版の初期状態、`mask(byte)`は拡張前のマスクを返します。

```sh
cargo run --locked --release -- run experiments/stage5.json /tmp/fdr-stage5.json --algorithm fdr-batched --bucket-count 8 --filter-length 8 --batch-size 8
cargo build --locked --release --example batched_fdr
python3 experiments/stage5.py target/release/examples/batched_fdr > /tmp/fdr-stage5-report.json
```

CLIの`fdr-batched`は`--batch-size 1|2|4|8`（既定8）を使います。
`fdr-sequential`は常に1です。JSONの`fdr_filter.batch_size`に実際の値を保存し、
`state_width_bits`と`state_words`には一括処理の拡張領域を含めます。
候補数などの統計は検索時間とは別に収集します。

比較用exampleは、参照実装との一致と全B間の候補列の一致を確認した後、
同じ入力・DP分割・フィルタ長8・全パターン比較・結果受け渡し方法で計測します。
Bごとに3回ウォームアップし、9回の検索時間と中央値を保存します。
検索中は各一致を`black_box`に渡して数え、結果配列の確保・ソートは行いません。
状態の確保・初期化、候補抽出、完全比較は計測に含めます。
生成、構築、正解比較、詳細統計、JSON出力は検索時間から除きます。
CLIの`find_all`による単発時間とは受け渡し方法が異なるため、直接比較しません。

比較する入力条件は`stage4.json`（4文字・短いパターンが混在）と
`stage5.json`（全256バイト・長さ8〜32）です。
どちらもseed 41、100パターン、8バケット、指定フィルタ長8です。
実験結果は別途計測し、上記コマンドの出力先に保存します。

## 段階6：Super Character

`FdrFilter`に`.with_super_character(SuperCharacterBits::Four)`を指定すると、
現在文字8ビットと次文字の下位4ビットをキーとする表を構築します。
`SuperCharacterBits`は`Zero`（既定）・`Four`・`Eight`です。
状態配置、初期化、padding、候補抽出、元パターン全体での検証は段階5と共通で、
全1・2・4・8バイト処理に組み合わせられます。

追加ビット数をsとすると、キーは`(current << s) | (next & ((1 << s)-1))`です。
各位置では、同じパターン内の現在文字と次文字の組を許可します。
例えば同じバケットの`ab`・`cd`では、文字単独の共有で許された`ad`・`cb`が、
4ビット追加で候補から除外されます。下位4ビットが同じ次文字は区別できず、
8ビット追加でさらに候補が減る場合があります。文字対の共有と末尾の切り詰めによる
偽陽性は残るため、元パターンによる完全比較は引き続き必要です。

パターン末尾（d=0）は次文字の全組合せを許可します。
記事にある`b$`だけを登録する方式では、パターン`ab`の入力`abx`中の一致を落とすためです。
入力末尾では次文字を読まず、現在文字だけの通常マスクを使って保守的に候補を残します。
一括処理の最終文字も、次ブロックが存在すればその先頭文字を参照し、
ブロック境界を入力末尾として扱いません。端数処理でも入力全体の範囲で先読みします。
`super_character_mask(byte, Some(next))`で文字対のマスク、`None`で末尾用のマスクを確認できます。
`mask(byte)`は追加ビット数にかかわらず通常の文字マスクを返します。

マスク表は1項目あたり`ceil(q*L/64)`個のu64です。
通常表256項目を末尾処理用に残し、sが4・8ならSuper Character表`256 * 2^s`項目を加えます。
`mask_table_bytes()`と結果JSONは両表のワードデータの合計を記録します。
Vecの管理領域、アロケータの負荷、パターン、検証用データは含めません。
q=L=8なら、0・4・8ビット追加でそれぞれ2,048・34,816・526,336バイトです。
一括処理長を変えても表サイズは変わりません。空集合では0バイトです。
各項目にVecを使う素朴な実装なので、実際の使用メモリはこの値より大きくなります。
表の構築は項目数と有効な位置数の積に比例する初期化を含み、追加ビット数とともに重くなります。

```sh
cargo run --locked --release -- run experiments/stage6.json /tmp/fdr-stage6.json --algorithm fdr-batched --bucket-count 8 --filter-length 8 --batch-size 8 --super-character-bits 4
cargo build --locked --release --example super_character
python3 experiments/stage6.py target/release/examples/super_character > /tmp/simple-fdr-stage6-report.json
```

CLIの`--super-character-bits 0|4|8`は`fdr-sequential`と`fdr-batched`に適用し、
実際の値を`fdr_filter.super_character_bits`に保存します。
比較用exampleは3種類の追加ビット数と4種類の一括処理長を組み合わせ、
最終一致を参照実装と、同じ追加ビット数の候補列を全処理長間で照合します。
段階5と同じ結果受け渡し方法で3回ウォームアップ・9回反復し、
全時間サンプル、中央値、表サイズ、構築時間、候補数、完全比較回数をJSONに保存します。
統計収集は検索計測の外で行います。スクリプトはCPU・OS・Rustバージョンも記録します。

2026-10-24にApple M5、macOS 27.0.1 arm64、rustc 1.99.0、Cargo既定releaseで測定しました。
全条件は100パターン、8バケット、指定フィルタ長8、DPの既定係数を使います。
以下の候補数・表サイズは全一括処理長で同じです。速度はMiB/sです。

| 設定（入力分布・パターン長） | 追加ビット | 表サイズ（バイト） | 候補数 | 逐次 | 8バイト処理 |
|---|---:|---:|---:|---:|---:|
| stage5（全256バイト・8〜32） | 0 | 2,048 | 2 | 227.80 | 188.05 |
| stage5 | 4 | 34,816 | 2 | 201.16 | 162.91 |
| stage5 | 8 | 526,336 | 2 | 146.86 | 125.71 |
| stage4（a〜d・2〜32） | 0 | 2,048 | 207,342 | 6.44 | 5.47 |
| stage4 | 4 | 34,816 | 103,969 | 7.56 | 7.59 |
| stage4 | 8 | 526,336 | 103,969 | 7.53 | 7.54 |
| stage6（a〜d・q〜t・2〜6） | 0 | 2,048 | 229,797 | 5.77 | 6.06 |
| stage6 | 4 | 34,816 | 57,358 | 21.83 | 22.39 |
| stage6 | 8 | 526,336 | 30,528 | 39.17 | 38.26 |

stage5はもともと候補が2個だけで、表の拡大と先読みの負荷に対して検証を減らす効果がありません。
stage4は次文字の下位4ビットだけで文字を区別でき、8ビットに増やしても候補は減りません。
stage6は下位4ビットが同じ文字（a/qなど）を含むため、8ビットでさらに絞れます。
stage6の完全比較回数は0・4・8ビットで3,648,825・657,184・283,480回でした。
構築時間は単発値ですが、stage6の逐次版では約0.06・0.15・1.61msでした。
候補削減の効果と表の構築・参照コストの両方があり、追加ビット数や一括処理長を増やしても
常に速くなるとは限りません。速度値はこの小規模入力と実測環境での結果です。

## 段階7：ハッシュによる検証の絞り込み

`FdrFilter`に`.with_verification(Verification::Hash)`を指定すると、バケットごとに
共通末尾長のハッシュ表を構築します。既定は`Verification::Exhaustive`で、従来どおり
バケット内の全パターンを比較します。逐次処理、全一括処理長、Super Characterと併用できます。

バケットbの共通末尾長をLとし、各パターンの末尾Lバイトをバイト列のまま
`hashbrown::HashMap`のキーにして、同じ末尾を持つ元IDを登録順に並べます。
候補の終了位置がendなら、入力の`input[end-L..end]`を借用して問い合わせます。
事前にダイジェストを計算せず、hashbrownの既定のfoldhashでハッシュします。
パターン長が混在していても、問い合わせ長はこのLに固定します。
初期状態によってendがL未満の候補は出ません。
キーがなければ全文比較をせず、キーがあれば対応する全IDの元パターンを全文比較します。
入力先頭より前に始まる長いパターンは、従来どおり比較対象から外します。
ハッシュ衝突はHashMapが末尾バイト列を比較して区別します。
共通末尾が同じ別パターンや同一内容の別IDは同じリストに残るため、
元パターンの全文比較が必要です。一致の報告順とフィルタの候補列は変わりません。

ハッシュ表の構築は検索前に行い、`construction_ns`に含めます。
`with_verification`を再度呼ぶと表を作り直し、`Exhaustive`なら表の内容を消します。
ハッシュ値自体は保存せず、Rustの版をまたいで同じ値になることも前提にしません。

```sh
cargo run --locked --release -- run experiments/stage6.json /tmp/fdr-stage7.json --algorithm fdr-batched --bucket-count 8 --filter-length 8 --batch-size 8 --super-character-bits 4 --verification hash
cargo build --locked --release --example hash_verification
python3 experiments/stage7.py target/release/examples/hash_verification > /tmp/simple-fdr-stage7-report.json
```

CLIの`--verification exhaustive|hash`はFDRの2方式に適用します。
結果JSONの`fdr_filter`に`verification`と`verification_table_bytes`を保存し、
共通統計に`hash_lookups`を追加しました。問い合わせはハッシュ方式の候補ごとに1回で、
全比較方式および説明用モデルでは0です。`full_comparisons`は元パターン全体の比較回数で、
問い合わせを数えません。候補数と真の一致を含まなかった候補数は両方式で同じです。
検証表サイズは、異なる末尾バイト列の長さと全IDのusizeの論理データ量の合計です。
HashMapの空き容量・管理領域、Vecのヘッダ・余剰容量、アロケータ、元パターンは含めません。
これは全比較方式のバケット一覧に追加する索引のサイズで、検証データ全体の使用量ではありません。
空集合と全比較方式では0です。

比較exampleは3設定について追加0・4・8ビット、逐次・8バイト処理、全比較・ハッシュの
36条件で参照実装との一致を確認します。同じフィルタ設定では候補列も照合します。
各条件は3回ウォームアップ、9回の検索時間の中央値を使い、全サンプルを保存します。
結果の受け渡しは段階5・6と同じ`black_box`とカウントで、配列の確保・ソートは行いません。
検索時間には状態の確保・初期化、候補抽出、ハッシュ問い合わせ、全文比較を含め、
構築・詳細統計・正解比較・JSON出力は除きます。

## 段階8：SIMD

`FdrFilter::with_backend(Backend::Neon|Sse2)`でマスク累積を切り替えます。
既定の`Backend::Integer`と同じ末尾基準のビット配置、マスク表、初期状態、
一括処理長、候補抽出、状態の右シフト、検証処理を使います。
NEONはaarch64、SSE2はx86_64で利用でき、両命令セットは各対象の基本機能です。
非対応のバックエンドは構築時にエラーになります。

128ビットのベクトルで、64ビットの2レーンを同時に左シフトしてORします。
直前のワードからの繰り上がりは別のベクトルを右シフトして合成します。
任意のバケット数と複数ベクトルにまたがる状態を扱い、奇数個目の末尾ワードは
整数演算で処理します。状態が1ワード以下の場合も整数演算で処理するため、
SIMDバックエンドの選択だけではSIMD命令の実行は保証されません。
候補抽出と次ブロックへの状態シフトは整数演算です。
整数版の一括処理も同じマスク累積関数を使う構成にそろえました。

```sh
# arm64ではneon、x86_64ではsse2を指定
cargo run --locked --release -- run experiments/stage5.json /tmp/fdr-stage8.json --algorithm fdr-batched --backend neon --bucket-count 8 --filter-length 8 --batch-size 8 --verification hash
cargo build --locked --release --example simd
python3 experiments/stage8.py target/release/examples/simd > /tmp/simple-fdr-stage8-report.json

# 生成コードの確認（x86_64は対応ターゲットの事前インストールが必要）
cargo rustc --locked --release --lib -- --emit=asm
rg 'ushl\.2d|orr\.16b' target/release/deps/simple_fdr-*.s
cargo rustc --locked --release --lib --target x86_64-apple-darwin -- --emit=asm
rg 'psllq|psrlq|por' target/x86_64-apple-darwin/release/deps/simple_fdr-*.s
```

CLIはFDRの両方式で`--backend integer|neon|sse2`を受け付け、結果JSONの
`fdr_filter.backend`に選択値を保存します。逐次方式では一括処理長は1のままです。
比較exampleは同じハッシュ検証と結果の受け渡し方法を使い、追加0・4・8ビット、
処理長1・8、整数版と利用可能なSIMD版について候補列と参照実装の一致を確認します。
3回ウォームアップ後の9回の中央値と全サンプル、構築時間、表サイズ、詳細統計を保存します。
構築、正解照合、統計収集、出力は検索時間から除外します。

Apple M5のaarch64でNEON版を実行し、生成コードの`ushl.2d`と`orr.16b`を確認しました。
x86_64-apple-darwin向けSSE2版はクロスビルドと`psllq`・`psrlq`・`por`の確認までです。
x86_64の実機での正しさと速度は未確認のため、対象CPUでの検証は残っています。
SIMDが整数版を上回らない条件もあり、速度向上を一般化しません。

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
`grouping-truncation`、`fdr-sequential`、`fdr-batched`を選べます。
出力には形式・生成器・パッケージのversion、OS・アーキテクチャ、debug assertionsの有無、
設定、入力長、パターン数、構築・検索時間（ナノ秒）、全一致、埋め込んだ一致が含まれます。

現段階の時間は動作確認用の単発サンプルです。検索時間には一致配列の確保と格納を含み、
データ生成、ファイル読み込み、JSON保存は含みません。構築時間はパターン検査、最長長の取得、選択した検索方式の
作成にかかる時間です。CLI単体にはウォームアップ・反復計測がありません。段階5〜8の比較用exampleでは反復とCPUの記録を行います。

実ファイルを読み込むCLIは後続の段階で実装します。

## 使用するライブラリ

- `hashbrown`：共通末尾バイト列から検証対象IDを引くハッシュ表。
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

段階4では初期状態とpadding、混在長・全256バイト・ワード境界を検証します。
候補集合は各入力窓と末尾の文字クラスを直接比較した結果、一致集合は参照実装と照合します。
DPは小規模な全連続分割の列挙と比較し、元ID・安定した長さ順・再現性、
全3方針のCLI設定・実効末尾長・統計保存と不正係数の拒否も確認します。

段階5では全1・2・4・8バイト処理で、候補列・最終一致・統計を逐次版と比較します。
全ブロック内位置、長さ0〜33の端数入力、境界をまたぐ長い一致、短い共通末尾長、
全256バイト、空集合、混在長、重複・包含・重なり、64ビットの境界と全ワードシフト、
固定seedのランダム入力、120ビットの幅、CLIの処理長・不正値も確認します。

段階6では、文字対を直接照合する独立した候補計算と、参照実装による最終一致を確認します。
追加0・4・8ビットと全一括処理長で、`ab`・`cd`の偽候補、4ビットの衝突、
`abx`内と入力末尾の`ab`、全ブロック位置・端数・境界をまたぐ先読み、
全256バイト、混在長・切り詰め、重複・包含・重なり、空集合・空入力、ワード境界を検証します。
固定seedのランダム入力、全3分割方針、CLIの追加ビット数・表サイズの保存と不正値も確認します。

段階7では全比較方式・ハッシュ方式・参照実装の一致を、候補列と統計も含めて照合します。
同じ末尾を持つ異なるパターンを全文比較で区別し、
重複IDの真の一致を残すことを確認します。共通末尾の共有、キーなし、混在長、入力より長い
パターン、重複・包含・重なり、全256バイト、空集合・空入力、ワード・ブロック境界、
全追加ビット数・一括処理長・分割方針、固定seedの差分検証とCLI保存を確認します。

段階8では整数版と利用可能なSIMD版について、全一括処理長・追加ビット数・検証方法で
候補列・一致集合・統計を照合します。全256バイト、空入力・空集合、重複・包含・重なり、
全ブロック内位置と端数、64／128ビット境界、任意幅の状態、固定seedの差分検証を含みます。
マスク累積は独立したビット単位の正解計算とも比較し、入力と出力の幅0〜10ワード、
0〜520ビットのシフトを確認します。CLIのバックエンド保存と非対応指定の拒否も検証します。
