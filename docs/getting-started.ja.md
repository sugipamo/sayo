# Sayoを始める

[English](getting-started.md) | 日本語

このガイドでは、SayoVMのビルド、プログラムの実行、最小Worldの構成、ブラウザPlaygroundのローカル利用、
テスト方法を説明します。厳密な意味論は[Language Specification](language-spec.ja.md)を参照してください。

## 必要な環境

Sayoは最新のstable Rust toolchainを対象とし、固定MSRVは定めていません。

## ビルドと実行

SayoVMをビルドし、算術Example Worldで式を評価します。

```sh
cargo build
cargo run -- --world examples/native_arithmetic/world add 5 3
```

```text
8
```

Identity exampleを検証して実行します。

```sh
cargo run -- check --world examples/identity_relation/world
cargo run -- --world examples/identity_relation/world examples/identity_relation/main.sayo
```

```text
42
```

GitHubリポジトリをpublicにした後は、`sayo`コマンドを直接インストールできます。

```sh
cargo install --git https://github.com/sugipamo/sayo
```

Cargo package名は`sayo-lang`、インストールされるコマンド名は`sayo`です。Sayo 0.1.0はcrates.ioへ公開しません。

## ソースファイル

`.sayo`ファイル全体を一つの式として評価できます。

```sh
sayo --world examples/native_arithmetic/world examples/native_arithmetic/main.sayo
```

現在、改行は空白と同じです。`.sayo`で終わるAtomは、それを記述したファイルからの相対パスで別の
ソースファイルを参照します。循環参照はエラーです。

## 診断とトレース

結果TermのSpaceと実装種別を表示します。

```sh
sayo --world examples/native_arithmetic/world --show-space --show-kind add 5 3
```

NormalFormをstdoutへ保ったまま、version付きの内部Sayo Reduction IR（SRI）をstderrへ出力します。

```sh
sayo --world examples/native_arithmetic/world --trace-format json add 5 3 2> trace.json
```

内部形式は[SRI JSON](sri-json.ja.md)、Visualizer向け公開frame契約は[SVI JSON](svi-json.ja.md)にあります。

## World検証

実行前にWorldを検証します。

```sh
sayo check --world examples/native_arithmetic/world
```

`check`はWorld構造、Sayo構文、Component shadow、静的payload arity、埋め込み標準Worldとの合成を検証します。
セルフホストは要求しません。表示件数はユーザーWorldが所有する宣言だけを数えます。

## 最小World構造

```text
world/
├── <Space>/
│   └── space.sayo
├── <Left>/<Right>/<Result>/
│   └── main.sayo
└── <Template>/template.sayo
```

- `space.sayo`はSayo Space Checkerです。
- `main.sayo`はSayoVMが評価するRelation本体です。
- `template.sayo`は任意の制限付きpayload schemaです。
- 空のResultディレクトリは構造的Captureを定義します。
- `.tick`、`.end`、`.any`は予約Template control slotです。

Native Space CheckerとPrimitive RelationはSayoVMに、標準状態機械は埋め込み標準Worldにあります。そのため、
空のユーザーWorld overlayも有効です。ユーザーComponentは同名の標準Component全体をshadowし、隠された
Component内の個別ファイルへはfallbackしません。

TickとEndのcontrol本体では、`%0`、`%1`以降が現在の不変payloadを参照します。トップレベルの各式が次の
payload slotを一つ定義し、複合作用には`%0 (%0 %1)`のようなGroupを使います。

Result anchor、未参照payloadの放出、Group展開、Scheduler順序、NormalForm規則は
[Language Specification](language-spec.ja.md)を参照してください。Host境界とNative実行の理由は
[Design Notes](design.ja.md)にあります。

## 標準操作

埋め込み標準WorldとNative fallbackは、算術、比較、複製、開いた変換器、Collectorを提供します。

```sh
sayo --world examples/native_arithmetic/world duplicate 5 6
# 5 5 6 6 duplicate

sayo --world examples/native_arithmetic/world for '(add 9)' 1 2 3
# 10 11 12 Each{Plus{9}}

sayo --world examples/native_arithmetic/world filter odd 1 2 3 4
# 1 3 Sieve{odd}

sayo --world examples/native_arithmetic/world fold add 0 1 2 3
# 6

sayo --world examples/native_arithmetic/world reduce add 1 2 3
# 6

sayo --world examples/native_arithmetic/world compare 5 9
# Less
```

Bootstrap `Int`は`i64`で、overflowとゼロ除算はTrapします。比較はBooleanではなく`Ordering` Spaceの
`Less`、`Equal`、`Greater`を生成します。

## BrowserとWebAssembly

PlaygroundはWeb Worker内でWasm SayoVMを実行します。各実行開始時にユーザーWorldをsnapshotし、version付き
単一JSON project documentを受け取ります。意味を持つ空ディレクトリはfileと分けて列挙します。

Wasmビルドには`wasm32-unknown-unknown` targetと、リポジトリが使う版の`wasm-bindgen-cli`が必要です。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
./scripts/build-wasm.sh
```

生成bindingはgit管理外の`playground/pkg/`へ置かれます。`playground/index.html`をHTTPで配信すると、ソース実行、
SVI frameのアニメーション、メモリ上のユーザーWorld編集、projectのimport／export、実行中止を利用できます。

共有URLのfragmentにはgzip圧縮した入力Worldだけを入れ、結果とSVIは含めません。8,000文字を超えるURLは
拒否し、JSON import／exportを使います。

公開先にはCloudflare Pagesを予定しています。再現可能なビルドと後日の公開手順は
[Playground Deployment](deployment.ja.md)にあります。

## テスト

Rustの検査を実行します。

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Chromiumでブラウザテストを実行します。

```sh
npm ci
npm run test:browser
```

ブラウザテストはWasmをビルドし、SayoVM、Web Worker境界、project共有、Trap、Visualizer再生を検証します。
Rustの自動テストは一時Worldを作り、手動Exampleは`examples/`にあります。
