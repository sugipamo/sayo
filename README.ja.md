# Sayo

[English](README.md) | 日本語

Repository: <https://github.com/sugipamo/sayo>

Sayoは「項の作用」を中心とする言語です。Rust製SayoVMが評価、状態、Native Primitiveを管理し、
標準状態機械は埋め込みSayo Worldとして実行します。実装経路はSayo RelationとSayoVM組み込みNativeです。

Sayoは、項列を状態、Relationを遷移規則、SayoVMを遷移器として扱う離散力学系の言語でもあります。
NormalFormは単なる戻り値ではなく、現在の規則ではそれ以上遷移しない平衡状態です。
Sayoが外界から受け取るのは起動時の初期Term列とWorld snapshotだけで、意味論上の出力は初期評価Groupが
平衡状態へ到達したときに残る順序付きTerm列そのものです。`Print`は導入せず、CLI表示はHost側の観測です。
World snapshotは物理コピーではなくHost契約上の不変値です。Runtimeの存続中にWorldや参照Sayoファイルを
書き換えた場合の動作は保証しません。

Term Template、`.tick` / `.end` / `.any`、および標準Templateとしての`Trap` は仕様として採用済みです。
StateTerm、payload、`%0`、構造的な右項捕捉、`.tick`、`.end`、`.any`のBootstrap実装があります。
`Template { payload, ... }`はユーザーも記述できる正式な状態構築構文であり、複数payloadは`,`で
区切ります。空白は項の作用を表すため、payload間のカンマは必須です。`Template {}`も
0項payloadのStateInstanceとして有効です。同じ式の再評価は既存状態の
復元ではなく、新しいStateInstanceを作ります。Bootstrap段階では標準Worldの中間Templateも構築できます。
ただし、予約Atom `Trap` の意味論的停止とWorldによる再定義拒否は実装済みです。
`.any`については、通常Relationの右辺fallback探索をBootstrap実装済みです。Template専用の
`.tick` / `.end`もSayo `main.sayo`一つを選ぶ実装があります。動的Result（`Right/.any/main.sayo`）は
通常RelationとTemplate制御Relationの両方でSayo実装に限り利用できます。同じ制御スロットに複数候補があれば
曖昧性`Trap`になります。現段階では、実装済みのNativeとSayo Relationを使って項列の評価を行います。

現在の言語仕様、設計資料、IR契約、保留事項は[`docs/`](docs/README.ja.md)にまとめています。

## ビルドと実行

現在は最新のRust stableをサポート対象とし、固定したMSRV（最小対応Rustバージョン）は設けていません。

```sh
cargo build
cargo run -- --world examples/native_arithmetic/world add 5 3
```

結果:

```text
8
```

SayoVMで書かれたRelationの最小例も実行できます。

```sh
sayo check --world examples/identity_relation/world
sayo --world examples/identity_relation/world examples/identity_relation/main.sayo
```

結果:

```text
42
```

Sayoソースファイルを一つの式として実行できます。

```sh
sayo --world examples/native_arithmetic/world examples/native_arithmetic/main.sayo
```

評価結果のSpaceや実装種別を確認する場合は、デバッグ表示を有効にできます。

```sh
sayo --world examples/native_arithmetic/world --show-space --show-kind add 5 3
```

Reducerイベントを調査する場合は、version付きの内部SRI JSONを出力できます。

```sh
sayo --world examples/native_arithmetic/world --trace-format json add 5 3 2> trace.json
```

内部SRI JSON契約の詳細は[`docs/sri-json.ja.md`](docs/sri-json.ja.md)に記録しています。

Visualizer向けの公開形式は、完全な表示フレーム列を持つSayo Visualization IRです。
現在の契約は[`docs/svi-json.ja.md`](docs/svi-json.ja.md)に記録しています。Visualizerは
Reducerイベントの状態復元やSayoソースの構文推測を行いません。

評価結果の**正規形**はstdoutへ、内部SRI JSONはstderrへ出るため、結果をパイプする使い方を壊しません。

ファイル全体を現在のParserへ渡して評価します。現段階ではファイル内の
改行は空白と同じ扱いです。`.sayo` というAtomはSayoソースファイルへの
参照として扱われ、参照元ファイルのディレクトリを基準に内容を評価します。
循環参照はエラーになります。

Worldの構造とSayo実装を事前検証できます。

```sh
sayo check --world examples/native_arithmetic/world
```

`check` はセルフホストを要求しません。現在のブートストラップ規約に従い、
SpaceとRelationの構造およびSayo構文を検証します。
さらに埋め込み標準Worldを合成し、Component shadowと静的payload arityを含めてRuntimeを構築可能か
検証します。成功時の件数表示はユーザーWorldで宣言された内容だけを数えます。

GitHubリポジトリからPATHへ直接インストールする場合:

```sh
cargo install --git https://github.com/sugipamo/sayo
sayo --world examples/native_arithmetic/world add 5 3
```

Cargo package名は`sayo-lang`、インストールされるコマンド名は`sayo`です。Sayo 0.1.0はcrates.ioへ
公開しません。Rust組み込みAPIが発展途中である間は、PlaygroundとGitHubリポジトリを主要な配布経路とします。

## Browser / WebAssembly

ブラウザ版はユーザーWorldを実行開始時に`MemoryWorld`へ固定し、埋め込み標準Worldと分離したまま
SayoVM内で評価します。Wasm境界はversion付き単一JSONドキュメントです。空Relationディレクトリは
`files`から復元できないため、`directories`へ明示します。

```json
{
  "version": 1,
  "entry": "main.sayo",
  "directories": ["Add/Int/Plus"],
  "files": {
    "main.sayo": "add 5 3"
  },
  "options": {
    "visualization": true,
    "relation_depth": 256,
    "max_steps": 100000,
    "max_trace_events": 100000
  }
}
```

ビルドには`wasm32-unknown-unknown`ターゲットと、Cargo依存と同じ版の`wasm-bindgen-cli`が必要です。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
./scripts/build-wasm.sh
```

生成物は`playground/pkg/`へ置かれます。`playground/sayo-worker.js`はSayoVMをWeb Worker内で初期化し、JSON文字列を
受け取って、NormalFormと完全なSVIフレーム列を含むJSON文字列を返します。意味論の再生と表示フレームの
構築はRust側で完了し、Visualizerは各フレームと明示された変化を描画します。

公開先にはCloudflare Pagesを使用する予定です。再現可能なPages用ビルドはリポジトリへ含めますが、
プロジェクト作成と初回デプロイは後日の公開作業として残します。正確なビルド設定は
[`docs/deployment.ja.md`](docs/deployment.ja.md)を参照してください。

## World構造

```text
world/
├── <Space>/
│   └── space.sayo              # Sayo Space判定器
├── <Left>/<Right>/<Result>/
│   └── main.sayo               # Relation実装（SayoVM）
└── <Template>/template.sayo    # 任意のpayload schema
```

標準PrimitiveのSpace判定とRelationは
SayoVM内のNative Registryが提供し、標準状態機械は埋め込みSayo Worldが提供します。したがって、
ユーザーWorldは空のoverlayでも有効です。

`main.sayo` はSayoVM内で評価します。通常の二項Relationを空のResultディレクトリで定義すると、
Runtimeが右項を新しい状態のpayload末尾へ捕捉します。`.tick`と`.end`の`main.sayo`では、現在payloadを
`%0`、`%1`のような0始まりの位置参照で利用できます。
`.tick`または`.end`の固定Resultディレクトリを空にすると、payload 0項のResult StateInstanceへ
遷移します。未参照の現在payloadは通常どおり、その状態の左へ放出されます。
`.tick`は外部入力を消費しない状態の自己変化であり、遷移先Templateの定義に応じてpayload個数を
増加・維持・減少できます。各Templateのpayload個数はWorldから静的に導出されます。
Relation評価は現在、Bootstrap保護として意味論的な呼び出し深度256を上限としています。
呼び出しはSayoVMの明示的な評価フレームで管理され、Rustのコールスタックへは依存しません。
Flat項列の項数はRelation深度に含まれません。
Rust APIでは`Runtime::new_with_limits`へ`RuntimeLimits`を渡すことで、深度と遷移数を
Runtimeごとに変更できます。`Runtime::new`はBootstrap既定値を使います。

`main.sayo` Relationは生成ソースを返しません。本体を正規化し、その項列を呼び出し元へ展開します。
評価結果は一項に限らず、これ以上
遷移できない順序付きのTerm列（NormalForm）です。例えば作用できない `5 3` はエラーではなく
そのまま `5 3` という正規形になります。括弧は明示的なGroupであり、内部が正規化されてから
外側の列へ展開されます。
空のNormalFormも正当な結果です。`Trap`、発散、SRIトレース、CLIの描画文字列は
残存Term列には含まれず、言語の出力とは区別されます。

Sayo Space判定器は`<Space>/space.sayo`へ置き、候補ソースをStringとして`%0`で受け取ります。例えば
`equal-to "hello" %0`はAtom `hello`だけをそのSpaceへ所属させます。結果が通常の一項ならMatch、予約結果
`Miss`なら非所属、`Trap`なら判定異常です。空または複数項の結果も実装エラーになります。checkerでは
`%0`以外と外部Sayoファイルは利用できません。`Miss`は通常プログラムの値ではありません。

Host境界やNative Relationの設計理由は[`docs/design.ja.md`](docs/design.ja.md)、現在残っている実装候補は
[`docs/roadmap.ja.md`](docs/roadmap.ja.md)に記録しています。

標準PrimitiveはユーザーWorldへ宣言せず、SayoVMが提供します。World内の
`main.native`は現在サポートしておらず、`sayo check`でエラーになります。
そのため`check`のRelation数はユーザーWorldで宣言されたRelationだけを数え、
SayoVM標準Native Relationは別枠として表示します。

標準WorldはユーザーWorldと別ルートでSayoVMバイナリへ埋め込まれ、Runtime起動時に独立した
`MemoryWorld`として構築されます。探索順はユーザーWorld、標準World、標準Nativeです。ユーザーWorldに
同名Componentがある場合、標準側の判定器・schema・Relation・`Tick`・`End`を含むComponent全体を
shadowし、標準側の一部分へはフォールバックしません。現在の
埋め込み標準Worldには`For`、`Filter`、`Fold`、`Reduce`、`Duplicate`のCapture／Tick／End Relationを収録しています。同名Native Relationは
標準World定義が利用できない場合のfallbackです。

Template制御Relationの`main.sayo`では、トップレベルの各項が独立した次payloadスロットになります。
複合式を一つのスロットとして評価する場合は、`%0 (%0 %1)`の第2項のように括弧で囲みます。

現在の標準Native Relationとして、整数四則演算を提供しています。

```text
Add Int : Plus       Plus Int : Int
Sub Int : Minus      Minus Int : Int
Mul Int : Times      Times Int : Int
Div Int : Quotient   Quotient Int : Int
Compare Int : CompareInt   CompareInt Int : Ordering
Compare String : CompareString   CompareString String : Ordering
Duplicate Int : Duplicate
For * : Each            Each Int : Each
Filter * : Sieve        Sieve * : Sieve
Odd Int : Keep | Drop
Fold * : FoldInit       FoldInit * : FoldAccum
FoldAccum * : FoldAccum
Reduce * : ReduceInit   ReduceInit * : ReduceAccum
ReduceAccum * : ReduceAccum
```

算術のInt表現はBootstrap段階では`i64`です。オーバーフローとゼロ除算は
Runtimeエラーになります。

`Compare 5 9`、`Compare 5 5`、`Compare 9 5`は、それぞれ`Ordering` SpaceのNative Term
`Less`、`Equal`、`Greater`へ遷移します。これはBooleanではなく、整数比較用の暫定Primitive protocolです。

`Duplicate 5` は標準Worldの状態機械により
`EmitTwice{5}` → `EmitPair{5, 5}` → `5 EmitOnce{5}` → `5 5 duplicate` と遷移します。
中間表示はRuntime内部状態であり、ユーザーが書くAtomではありません。標準Primitive Atomは、ユーザー／標準Worldの
Space判定器で所属が決まらない場合に限り、Native fallbackによって所属Spaceを得ます。
`For`は、右の変換項を各後続Termへ一回ずつ作用させます。
`Filter`のpredicateは、互いに異なる標準Space `Keep` または `Drop` に属する一項を返します。
Sayo RelationではResultSpaceを`Keep`または`Drop`として宣言し、対応するAtom `keep`または`drop`を返します。
Native `Odd` は入力値に応じてどちらかのSpaceを動的に返します。専用Booleanや`if`は導入しません。
標準WorldのFilterは判定結果を`SieveChoose{predicate, input}`の右へ置き、通常Relationによって
`Keep`なら値を左へ放出し、`Drop`なら値を破棄します。それ以外のSpaceは`Trap`です。

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

## テスト

```sh
cargo test
```

ブラウザPlaygroundはWasm版SayoVMをWeb Worker内で実行します。`playground/index.html`をHTTPで配信すると、
ソース実行、SVIフレーム列の表示とアニメーション、ユーザーWorld編集、実行中断を利用できます。
共有URLには入力Worldだけをgzip圧縮した単一JSONとしてフラグメントへ格納し、実行結果とSVIは含めません。
8,000文字を超える共有URLは生成せず、Playground文書のJSON入出力を利用します。

```sh
npm run test:browser
```

このコマンドはWasmをビルドし、実際のヘッドレスブラウザでPlaygroundとWeb Workerの境界を検証します。

`examples/`には手動実行用のWorldを置き、自動テストでは機能ごとに一時Worldを生成します。
Worldに適用可能なRelationがある場合は、空ディレクトリによる構造的Captureを含めて
World定義を優先し、Native RelationはWorld候補がない場合だけfallbackします。
