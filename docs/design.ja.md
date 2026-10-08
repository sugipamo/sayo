# Sayo 設計メモ

[English](design.md) | 日本語

## 純粋なHost境界

Sayoの純粋核は、Hostが起動時に構築した`InitialTerms × WorldSnapshot`だけを入力とし、初期評価Groupの
残存`NormalForm<Term>`だけを通常結果とする。CLI表示、SRI、診断はHost／VMの観測・
運用層であり、Sayo内の入出力作用ではない。`Trap`と発散も出力Termにはしない。

`WorldSnapshot`は物理コピーを意味しない。HostはRuntimeの存続中にWorldと参照Sayoファイルを書き換えては
ならず、同時変更時の動作は保証しない。これは一般的な実行中プログラム資源の変更と同様に、評価入力への
外部競合として意味論の外に置く。

外部参照の試作は `blob` と `handle` の受け渡しを追加するもので、Relation の副作用は追加しない。
Request は初期 Group 全体の NormalForm を Host が観測するまで通常の Term として残る。
Host は外部で処理し、固定された応答 Term を次の純粋な評価区間へ渡せる。外形は `{}` に Term 全体を
並べて表し、`<>` の内容にはアクセスしない。採用した最小契約と保留中の実行方針は
[外部参照](external-boundary.ja.md) を参照。

## Relation実装のセルフホスト移行

```text
SayoVM
    ├── Native Primitive / Native Space Checker
    └── Sayo Relation
```

四則演算などの基礎RelationはRust Runtime側のNative Registryへ実装する。
これは、すべての基礎RelationをSayo自身で実装しようとすると、最初の計算を
実行するための実装がなくなり、循環的なブートストラップ問題になる可能性が
あるためである。

Runtime内のNative Relationとして実装する方式を採用する。

```text
通常Relation: Worldからmain.sayoを探索して実行
Native Relation: Runtime内部のRust実装を実行
```

Native Relationは、SayoVMが直接実行するPrimitiveなRelation実装とする。
Relationの意味はSpace/ResultSpaceと通常の評価規則で決まり、実装方式はSayoまたはNativeとする。

SpaceとTemplateは概念上分離する。Spaceは外部から見えるTermの分類・接続点、TemplateはTermの構造と
状態遷移規則である。Templateは公開Spaceを提供でき、StateInstanceは公開Spaceを通してRelationへ参加
する。BootstrapではTemplate名と公開Space名を同名にするが、これは公開インターフェースの対応付けであり、
概念上の同一化ではない。

標準PrimitiveはユーザーWorldへ宣言しない。`main.native`は標準Primitiveの宣言
方法として使用せず、World内に置かれていた場合は誤設定としてエラーにする。
Native RegistryはSayoVM内部で固定登録し、ユーザー独自Nativeは追加不可とする。

Native内部Termは`NativeTermId`から型付きの`enum NativeTermState`を参照する。
IDとStoreの境界により、内部表現を公開意味論から分離する。

### Template schema

Templateの`template.sayo`は、payloadを検証する制限付きSayoプログラムとして扱う。検証処理の成功を
schema成功とし、`Trap`、`NoStep`、step limit超過は不通過とする。通常のNative Relation、外部ファイル、
外部入力、副作用を禁止し、schema専用の`CheckSpace`と`Pass`だけをSayoVM内部で提供する。
schemaがないTemplateは静的payload arityとRuntime遷移時の検証で扱う。

### Native Relationの選択

標準Native RelationはSayoVMが管理し、ユーザーWorldから追加できない。
World内に適用可能なSayo／構造的Capture RelationがあればWorld定義を優先し、Native候補は
World Relationがない場合だけ意味論互換fallbackとして使う。Native登録APIはRust内部に限定する。

最初の標準Native Relationとして整数四則演算を実装する。

```text
Add Int : Plus       Plus Int : Int
Sub Int : Minus      Minus Int : Int
Mul Int : Times      Times Int : Int
Div Int : Quotient   Quotient Int : Int
```

Native IntはBootstrap段階では`i64`で、計算結果を通常のInt Termとして登録する。
各左側Relationはcaptured TermIdを持つNative中間Termを生成し、中間Termが次のIntを
受け取って計算する。オーバーフローとゼロ除算はRuntimeエラーとする。

入力を保持して後続の作用を受ける状態は、Sayo Worldでは構造的CaptureとStateInstance、Nativeでは
Native Term Storeによって表現する。Sayo Relationへラムダやクロージャ専用構文は追加しない。
どちらの実装も同じRelation、payload、左端優先Schedulerの意味論に従う。

整数表現・四則演算・比較はNative、Scheduler・Tick・End・Vanish・TrapはRuntime、通常の状態機械は
Sayo Worldへ置く。Native RelationをWorld内で宣言する方式は採用しない。今後追加する個々のPrimitiveを
NativeとSayoのどちらへ置くかは、その意味論を決める際に判断する。

Runtime内部では、SayoとNativeの実行処理を分離する。
Native Executorの仕様は内部実装として扱い、Worldからの
選択方法は公開しない。Native最適化はSayo実装との観測上の同値性を保つ。

Sayo Relationの再帰実行は、Rust関数の再帰ではなくSayoVMが所有する明示的な評価フレームへLoweringする。
各フレームはReducerセル、呼び出しContext、Sayo RelationまたはTemplate制御payloadからの復帰位置、
step数を保持する。これによりFlat項列の幅とRelation深度を分離し、Hostスレッドのスタックサイズに
依存せずBootstrap depth limitで停止できる。

### 状態の管理

StateInstanceとNative Termはソースファイルへ生成せず、プロセス内のTerm Storeで管理する。
Runtime終了後の永続化、GC、有効期限、複数Runtime間の共有は導入しない。

### テストWorld

`examples/`は利用者が手動で試すサンプル、Rustテスト内で作る一時Worldは自動テスト
専用とする。Space判定、Sayo/Native Relation、状態項、構造的Capture、ファイル参照、
曖昧性、World検証、算術エラーを自動テストでカバーし、サンプルWorld
は最小の実行例として保つ。
