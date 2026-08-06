# Sayo Language Specification

[English](language-spec.md) | 日本語

この文書は、現在までに決定したSayoの言語仕様とRuntime方針を一つにまとめたものです。

各項目の状態は次のとおりです。

- **実装済み**: 現在の`SayoVM`で動作する。
- **保留**: 意図的に決めていない。

## 1. 概要

Sayoは、項（Term）の作用を中心にした言語である。

Sayoは、項列を状態、RelationとRuntime内部状態を遷移規則、SayoVMを遷移器として扱う
**離散力学系の言語**でもある。プログラムの意味はRelation群だけでは完結せず、
左端優先で遷移後に先頭から再走査するSchedulerを含む。

| 力学系としての役割 | Sayoでの対応 |
|---|---|
| 状態 | 順序付きTerm列とGroup制御フレーム |
| 遷移規則 | RelationおよびRuntime内部状態の遷移 |
| 時間発展則 | 左端優先Scheduler |
| 境界条件 | Group末尾でのみ試される内部`End` |
| 観測 | NormalForm、CLI表示、SRIトレース |

NormalFormは「一つの値」ではなく、`Tick`、通常の二項遷移、Group末尾で適用可能な`End`遷移の
いずれも残っていない平衡状態である。
無限遷移は言語エラーではなく、収束しない軌道である。初期SayoVMの資源上限は、その軌道を
Bootstrap実装が打ち切るための運用上の制限に過ぎない。

```text
M N
```

は「項`M`へ項`N`を作用させる」ことを表す。

Sayoには、表面上の関数・メソッド・Callableという概念を導入しない。高階的な振る舞いは、
次の作用を受けられる項をRelationが生成することで表す。

```text
Add 5
```

は、5を保持し、後から別の項を受け取れる項を生成しうる。

Sayoの基本原則は次のとおりである。

1. Everything is a Term
2. Semantic data is String
3. Runtime owns state
4. Everything evolves by transitions

ここでいうStringは、Sayoプログラムが観測・受け渡しするデータを指す。`TermId`、Native状態、
Group、`End`はSayoVMが遷移を管理する内部制御情報であり、ユーザーの意味論上の値ではない。

## 2. Term

### 2.1 基本構文とString Term — 実装済み

```text
Term ::= Atom
       | StringLiteral
       | Term Term
       | (Term)

StringLiteral ::= `"` UTF-8文字列 `"`
```

作用は左結合である。

```text
a b c
```

は次と同じである。

```text
(a b) c
```

空白は項の区切りである。現在、改行は空白と同じ扱いであり、独自の意味は持たない。

引用符付き文字列は単一の不変Termであり、`String` Spaceに属する。UTF-8を使用し、`\"`、`\\`、
`\n`、`\r`、`\t`をエスケープとして認める。未知のエスケープと、文字列内の直接改行は構文エラーとする。
引用符のないAtomや、内部的なソース文字列は言語上の`String`値ではない。

`.sayo`で終わるAtomはSayoソースファイルへの参照として扱う。相対パスは参照元の
Sayoファイルから解決する。循環参照はRuntimeエラーである。

### 2.2 Relation入力 — Bootstrap実装済み

通常の二項Relationでは、右項を参照する`input`または`%input`をSayo構文として提供しない。作用可能な
右項を見つけると、Runtimeが現在状態のpayload末尾へ右項を一項追加し、Relationパスが示すResult
Templateの新しいStateInstanceへ遷移させる。この自動捕捉遷移にSayo実装ファイルは必要ない。

```text
State{p0, ..., pn} x
→ Result{p0, ..., pn, x}
```

source-backedな左項はpayload 0項として扱う。捕捉後の計算はResult Templateの`Tick` Relationで表現する。

### 2.3 状態付きTerm — Bootstrap実装済み

状態付きTermの標準表記は、Template名と0個以上のSayo項からなるpayloadの組で表す。

```text
<Template>{<Term> (, <Term>)*}
```

`Template{}`はAtom `Template`とは異なる、payload 0項のStateInstanceである。

例:

```sayo
Plus{5}
For{(Add 9)}
```

`{...}`の中では`,`がpayload Termの区切りであり、空白は通常どおり項の作用を表す。複数payloadの
カンマは必須であり、`Pair{left right}`は不正な構文となる。作用を一つのpayloadへ格納する場合は
`For{(Add 9)}`のようにGroupで囲む。

payloadは一つのStateInstanceが所有する順序付きTerm列である。payloadに状態付きTermが
含まれていても、それは外側のTemplateから見た一つのpayload Termであり、複数の独立した状態機械を
一つの`{}`へ格納する意味は持たない。payloadの個数・各位置の意味・必要なSpaceはTemplate自身が解釈する。
RuntimeはpayloadをTerm ID列として保存し、Template固有のフィールド名や型名を意味論として持たない。

これは通常の作用である次の表記と区別される。

```sayo
Plus 5
```

`Plus(5)`は既存文法では`Plus 5`と同じであるため、状態付きTermの表記には使わない。

状態付きTermはRuntimeが自動捕捉Relationおよび内部遷移の結果として生成できるほか、ユーザーも
`Template { payload... }`を正式な状態構築構文として記述できる。payloadは0項でもよく、
`Template {}`は新しい0項StateInstanceを生成する。この構文は既存のState IDを復元する
シリアライズではなく、評価のたびに新しいStateInstanceを作る。

## 3. Space

SpaceはTermが属する集合である。Spaceは静的型ではなく、RuntimeのSpace判定器または既知のNative規則に
よって動的に判定される。Termは一つ以上のSpaceに属しうる。

```text
Int
Add
Plus
```

SpaceはWorld直下のディレクトリで表す。各Spaceには所属判定器を置ける。

SpaceとTemplateは概念上分離する。Spaceは外部から見えるTermの分類・接続点、TemplateはTermの構造と
状態遷移規則を表す。Templateは0個以上の公開Spaceを提供でき、StateInstanceはTemplateの公開Spaceを
通してRelationへ参加する。公開Spaceを持たないTemplateは内部状態としてのみ存在できる。
Bootstrap Runtimeでは既存のWorld構造と互換にするため、標準的にTemplate名と公開Space名を同じにする。
この同名は「TemplateがそのSpaceを実装している」ことを表し、TemplateとSpaceが同一概念であることを意味しない。
payload位置で使う既存のWorld Spaceと、将来のTemplate局所Spaceは、Templateの検証規則から参照できる。

```text
world/
    Hello/
        space.sayo
```

判定器は候補Termのソース文字列をSayoVMから受け取る。

標準PrimitiveのSpace判定器はSayoVM内部のNative Registryが提供する。現在は`Int`のi64判定と、
`add`、`fold`など標準Atomの完全一致判定を持つ。
ユーザーWorldでは`<Space>/space.sayo`にSayo Space Checkerを定義できる。候補Termの生のソース文字列は
`String` Termとしてpayload `%0`へ渡される。checkerの正規形がちょうど一項なら、その結果が`Miss`で
ない限り所属（Match）とする。`Miss`はchecker専用の非所属結果であり、通常のTermとして残存できない。
`Trap`は判定器の異常として評価全体を停止する。空または複数項の正規形もchecker実装エラーである。

checkerは純粋で閉じた分類規則として実行するため、参照できるpayloadは`%0`だけとし、外部`.sayo`
ファイルと外部入力を禁止する。checker内のAtomは別のユーザーSpace Checkerで
再分類せず、埋め込み標準WorldとNative層だけで解決する。これによりchecker間の再帰を生じさせない。
Worldで所属が見つからなかった場合にNative Space判定へfallbackする。複数SpaceがMatchした場合は
曖昧性エラーとする。

## 4. Relation

RelationはSpace間の作用を定義する。

```text
LeftSpace RightSpace : ResultSpace
```

例:

```text
Add Int : Plus
Plus Int : Int
```

意味は次のとおりである。

> LeftSpaceに属する項へRightSpaceに属する項を作用させると、
> ResultSpaceに属する項が得られる。

Relationは、結果として次の作用を受けるTermを返せる。これは関数・Callableを導入せずに
部分適用や高階的な振る舞いを表す方法である。

```text
Add 5 : Plus
Plus{5} 3 : Int
```

## 5. WorldとRelation Storage

### 5.1 World構造 — 実装済み

```text
world/
├── <Space>/
│   └── space.sayo
├── <LeftSpace>/<RightSpace>/<ResultSpace>/
│   └── （自動捕捉Relationでは実装ファイルなし）
└── <LeftSpace>/<RightSpace>/<ResultSpace>/main.sayo
```

次の構造は、`Add Int : Plus`を表す。

```text
world/
    Add/
        Int/
            Plus/
                main.sayo
```

現在のRuntimeでWorldに宣言するRelationでは、Space、RightSpace、ResultSpaceはすべてWorld内のSpaceで
なければならない。
SayoVMが提供する非公開Native Relationはこの制約の外側にあり、`Each`、`Sieve`などの
Runtime内部SpaceをResultSpaceとして持てる。

RelationのLeft、Right、ResultはすべてWorld内の**Term Template名**を
指す。Spaceはmaterialize可能なTemplate、内部状態はRuntimeが生成したcapture-backed Template instanceで
ある。Relation結果は宣言されたResult Templateを右端anchorとして返さなければならない。複数項置換でも
右端anchorによって検証する。Result位置の予約名`.any`は例外であり、右端anchorを任意のTemplateにできる
動的Result Relationを表す。固定Result Templateと`.any` Resultは同じLeft / Rightの組に共存できない。
この排他規則はWorld検証とRuntime探索の両方で強制する。通常Relationでは、具体的なRightSpaceが
見つからない場合に`Left/.any/Result`をfallbackとして探索する。`Left/Right/.any/main.sayo`の
動的ResultはSayo実装に限り利用でき、Template制御Relationでも同じ`.any`の意味を用いる。

#### Relation候補の解決 — 実装済み

候補は次の優先順位で選ぶ。

1. 具体的なRightと固定Result
2. 具体的なRightと動的Result`.any`
3. `.any` Rightと固定Result
4. `.any` Rightと動的Result`.any`
5. Native wildcard

同じ優先度に複数候補がある場合は`Trap`とする。選択した候補が`NoStep`を返しても、低優先度の
候補へフォールバックしない。この候補順位は、後述のComponent単位shadowによって選ばれた一つの
World Component内で適用する。

### 5.2 Relation実装 — 実装済み

Relation実装には次の種類がある。

| 種類 | 配置 | 実行者 |
|---|---|---|
| Sayo | `main.sayo` | SayoVM |
| Native | World外 | SayoVM内部のRust |

通常の二項Relationでは、Resultディレクトリの存在自体が自動捕捉遷移の定義である。将来、宣言用ファイルを
追加できる余地は残すが、初期仕様では空ディレクトリを正式なRelationとして扱う。
`main.native`をWorld内に置くことはできない。標準Native RelationはSayoVMが管理する。
Native Space判定器も同じRegistryで管理し、ユーザーWorldでの宣言を要求しない。

## 6. 実行境界

Relation実装はSayoVM内部で評価する`main.sayo`、空ディレクトリによる構造的Capture、または
SayoVM組み込みNativeである。外部プロセスprotocolは存在しない。Relationの適用不可はRelation候補が
存在しない場合の`NoStep`としてSchedulerが扱い、実装プロセスのexit statusには依存しない。

## 7. Runtime-owned Terms

状態付きTermとNative Termはプロセス内のTerm Storeで管理し、ファイルへ保存しない。
Native状態の寿命は**そのRuntimeの寿命と同じ**である。個別のGC・有効期限・複数Runtime間の
共有は行わない。

### 7.1 状態のpayload — Bootstrap実装済み

状態付きTermは、Templateが保持するpayloadと、後続の右項を区別する。

```text
Plus{5} 3

payload: [5]
input:   3
```

整数算術のNative Termと汎用StateInstanceは、捕捉値をRuntimeメモリ内の`TermId`として保持する。
汎用StateInstanceは次の形式でpayloadを保持する。

```text
StateInstance {
  state_id: StateId
  template: TemplateId
  payload: Vec<TermId>
}
```

Template内の位置参照は、`%0`、`%1`、`%2`のような0始まりの記法を採用する。これは
Template内部のpayload参照であり、通常のユーザー項における変数や関数ではない。
`Add{9}[Plus]`のようなSpace・型名の表示は、Runtimeの意味論とは分離し、Visualizerや
デバッグ表示層が必要に応じて付加する。Runtimeがpayloadの上位型名を要求することはない。

Sayo RelationへCapture構文、ラムダ構文、任意のソース生成機能は現時点では導入しない。
状態付きTermを一般に生成する仕組みはRuntime側の責務とする。

## 8. 評価

### 8.0 Host Boundary — 実装済み

Sayoの意味論が外界と接触する面は二つだけである。

1. Hostが起動時に、初期Term列と論理的に不変なWorld snapshotを初期評価Groupへ与える。
2. 遷移不能になった初期評価Groupに残存する、順序付きTerm列をHostが結果として観測する。

Sayo内部から値を送信する`Print`や、評価途中で外部入力を追加取得する操作は導入しない。結果は文字列ではなく
`NormalForm<Term>`であり、0項・1項・複数項の全てを許す。CLIの空白区切り表示、Rust APIでの構造取得、
将来のVisualizer表示は、同じ残存Term列に対するHost側の異なる観測方法である。

`Trap`は出力Termではなく、NormalFormへ到達しなかった評価結果である。発散には結果がなく、初期VMの
bootstrap limit到達は発散を保護する診断である。SRIは実行観測であり、Sayoの出力ではない。
概念上の評価は次で表す。

```text
evaluate : InitialTerms × WorldSnapshot
        → NormalForm
        | Trap
        | Divergence
```

同じ初期Term列と同じWorld snapshotからは、同じ残存Term列または同じTrapへ到達する。

初期SayoVMはWorldを物理的に複製せず、Runtimeの存続中にWorldディレクトリと参照Sayoファイルが変更されない
ことをHost契約として要求する。実行と同時にこれらを書き換えた場合の動作は保証しない。この変更はSayo内の
状態遷移ではなく、Hostが評価中の入力値を外部から破壊的に変更する競合として扱う。

### 8.1 左端優先の状態遷移 — 基盤実装済み

RuntimeはParserが作る構文木を受け取り、Term列を左端から走査する。隣接する二項に
遷移が可能なら、その**最初の一つだけ**を遷移させ、必ず列の先頭から走査をやり直す。
どこにも遷移がなければ、その列はNormalFormである。NormalFormは一項に限らない。

```text
add 5 3
```

は次の順に評価される。

```text
add 5
    ↓
Plus{5} に相当するNative Term
    ↓
そのTerm 3
    ↓
8
```

`NoStep`（適用可能なRelationがない）はエラーではない。右へ走査を
進める。曖昧なRelation、Space不整合、実装失敗は`Trap`として停止する。

Relationの再帰深度には256、状態遷移数には100,000の既定Bootstrap上限がある。上限到達は
`bootstrap limit`診断であり、言語上「自己遷移が不正」であることを意味しない。可視上
`A -> A`となる自己遷移も正当な遷移であり、無制限実行を許す将来のRuntimeでは発散しうる。

### 8.2 Groupと項列の展開 — 基盤実装済み

`(...)`は単なる構文木の結合優先順位ではなく、内部制御用のGroupである。Groupは内部を
NormalFormまで評価してから、そのTerm列を親の列へ展開する。

```text
1. Groupがあれば先に内部を正規化して展開する。
2. 現在位置のTermへ右項なしの`Tick`遷移を試みる。
3. `Tick`が`NoStep`なら、現在位置の隣接二項に作用を試みる。
4. 遷移すれば、対象を一つ以上のTermへ置換して先頭へ戻る。
5. `NoStep`なら右へ進む。
6. 最後まで`NoStep`なら、Group末尾で最後のTermへ`End`を試みる。
7. `End`も`NoStep`なら、全列をNormalFormとして返す。
```

Relation実行の置換結果は一項に限らない。ただし現在のRelation契約では、展開した
置換列の右端Term（anchor）が宣言されたResultSpaceに属さなければならない。

```text
Sum [右側で生成中のSequence]
    ↓ 右側を評価
Sum Sequence{6, 7, 8}
    ↓ 左を再試行
21
```

内部状態はRuntimeが構造的Captureと標準Worldの制御Relationから生成する。最小の参照実装は次のように、
右を消費した状態が内部の値を複製し、二値を保持する別状態を介して左へ一つずつ吐く。

```text
Duplicate 5
→ EmitTwice{5}
→ EmitPair{5, 5}
→ 5 EmitOnce{5}
→ 5 5 duplicate
```

`EmitTwice`、`EmitPair`、`EmitOnce`はユーザーが書くAtomやSpaceではない。表示はRuntimeの`display`であり、
実行用の`source`とは区別される。

### 8.3 Relation Outcome — 実装済み

Reducerの内部Outcomeは次の三つである。

| Outcome | 意味 | Reducerの動き |
|---|---|---|
| `Success` | 一つ以上のTermへの状態遷移 | 先頭へ戻る |
| `NoStep` | 現在の隣接対に作用できない | 右へ進む |
| `Trap` | 実装または意味論の失敗 | 即座に停止 |

同じ表示の状態への`Success`も意味論上は許される。初期VMだけがBootstrap step limitで
止める。`Pending`は存在しない。

`Trap`の例には、除算ゼロ、整数オーバーフロー、不正な状態遷移、循環参照、深度超過が含まれる。

### 8.4 Scheduler Contract — 実装済み

Schedulerは、項列上の次の遷移を決定するSayoの意味論そのものである。初期SayoVMは次を守る。

1. 各評価Groupは左端から走査する。
2. Groupは外側の項列へ展開する前に、内側をNormalFormまで評価する。
3. 現在セルでは、二項作用より先に`Tick`を試す。
4. `Tick`が`NoStep`なら、現在セルと右隣セルに二項作用を試す。
5. いずれかの遷移が成功したら、置換された項列全体の先頭から走査をやり直す。
6. 全ての`Tick`と二項作用が`NoStep`なら、最後のTermだけに`End`を試す。
7. `End`も`NoStep`なら、その項列がNormalFormである。

Sayo Relationの意味論的な再帰は許される。SayoVMはRelation呼び出し、Template制御payloadの評価、
および呼び出し元への復帰位置を明示的な評価フレームとしてheap上で管理し、Rustのコールスタックへ
Relation深度を写像しない。Flat項列の項数はRelation深度ではなく、長いFlat列だけを理由に
`relation_depth`制限へ到達しない。Relationが生成した状態Termは増加後の評価深度を保持し、後続の
`Tick`／`End`から開始するRelation評価へその深度を引き継ぐ。

従って、`A Tick → A`が発散する`A B`では、`A`は`B`を受け取らない。これは右項を受ける前に
内部発展を続ける状態として扱う。公平な並列評価や、右側を先行させる評価戦略は導入しない。

最適化実装は許すが、左端優先と再走査による因果関係を変えてはならない。すなわち、最適化は
正当な遷移列を短縮できても、本来より右側の遷移を先行させたり、左へ放出されるTermの順序を
変えたりしてはならない。SRIは最適化後の実行軌道を記録しうる。

Relationは再実行されうる。Sayoは純粋言語であり、全てのRelationは、同じ入力に対して同じ結果を返し、
外部状態を変更してはならない。Native最適化と将来の並列化もこの契約を前提とする。step limitは
初期SayoVMのBootstrap保護であり、Sayoの意味論上の停止規則ではない。

### 8.5 Tick Relation — Bootstrap実装済み

右項なしRelationを、単体Termへの内部イベント`Tick`として導入する。

```text
Left : Result
```

これは、実際の右Termを消費せずに行う遷移である。

```text
State Tick → Result
```

`Tick`はユーザーが書くAtomやSpaceではない。Runtimeが項列を左から走査する際、各Termに対して
通常の二項作用より先に試す内部イベントである。`Tick`の成功は、一項または複数項による現在セルの
置換であり、Schedulerは列の先頭から再走査する。

`Tick`の本質は、外部の右項を取り込まずに行う状態の自己変化である。遷移前後のpayload個数に
単調性の制約は置かず、増加・維持・減少をすべて認める。例えば`One{%0}`から`Two{%0, %0}`への
遷移も正当である。個数は各Templateについて静的に一意に決まり、`Tick`実装のトップレベルの
payloadスロット数が遷移先Templateの個数と一致しなければWorld検証エラーになる。
遷移先payloadが0項の場合は`main.sayo`を置かず、空の固定Resultディレクトリで表す。

`End`とは区別する。`Tick`は現在位置の任意のTermに試されるが、`End`は通常遷移が尽きたGroup末尾の
最後のTermにだけ試される境界イベントである。

現在の`EachApply → EachEmit`、`EachEmit → 値 Each`、`FoldApply → FoldNext`などの状態遷移は、
この`Tick` Relationとして標準Worldに実装されている。RuntimeはTemplateごとの`.tick/Result/main.sayo`を
読み込み、Sayo実装を一つ実行する。同じWorldの同じ制御スロットに複数Result候補があれば曖昧性`Trap`とする。
ユーザーWorldに同名Componentがあれば標準WorldのComponent全体をshadowする。

状態付きTermはRelationから生成でき、ユーザーも宣言済みTemplateの状態構築構文を直接書ける。
Runtimeは各状態個体へ固有の
State IDを割り当て、状態が捕捉する値は表示文字列ではなくTerm IDとして保持する。同じ表示・同じ捕捉値を
持つ状態も別個体である。状態のGC、永続化、生成ソースへのシリアライズは初期SayoVMでは導入しない。
状態付きTermの個体はRuntimeメモリだけで管理する。ソース中の同じ状態構築式を再評価しても、既存個体の
State IDを参照せず、新しい個体を作る。

状態構築で指定できるのはユーザーWorldまたは標準Worldに存在するTemplateだけである。payload項数は
Templateごとに静的に一意に導出し、`template.sayo`があればschema検証を通さなければならない。
異なるTemplate間の`Tick`ではpayload個数が変化してよい。`Trap`と`Vanish`は
Runtime予約状態なので直接構築できない。Bootstrap段階ではモジュール可視性を導入していないため、
`SieveApply`など標準Worldの内部段階Templateも構築可能である。この内部名への依存は標準ライブラリの
実装変更で壊れ得る。ただし、その標準機能のentry ComponentがユーザーWorldによってshadowされ、
内部段階が到達不能になった場合、その内部Templateは有効Worldから除外され構築できない。

状態遷移は空項列を結果にできる。予約内部状態`Vanish{payload...}`は次のTickで状態とpayloadを空項列へ
置換する。空Groupは構文として認めないが、評価途中および最終結果の空NormalFormは有効である。

ユーザーがTick Relationを書ける段階では、World上の保存形式として次を採用する。

```text
world/
  Left/
    .tick/
      Result/
        main.sayo
```

`.tick`は通常のSpace名ではなく予約されたRelation右辺スロットである。Runtimeは
`main.sayo`を一つ選び、Input遷移より先に実行する。Templateの`.end`についても、
`main.sayo`を一つ選び、Group末尾で実行する。同じ優先度の複数候補は曖昧性`Trap`である。

固定Resultのディレクトリに`main.sayo`がない場合は、Result Templateのpayload 0項StateInstanceへ
遷移する。例えば`Before/.tick/After/`は`After{}`を生成する。現在payloadはどれも次payloadから
参照されないため、元の順序で`After{}`の左へ放出される。この空形式は`Trap`および動的Result
`.any`には使用できない。`Vanish`の空形式は予約済みの消失遷移として従来どおり扱う。

### 8.6 Term Template / 自動捕捉protocol — Bootstrap実装済み

汎用状態付きTermは、Runtime所有のStateInstanceとして扱う。

Templateは状態Termの構造と、右項の自動捕捉、`Tick`、`End`の遷移規則を定義する。Templateは公開Spaceを提供
でき、StateInstanceはその公開Spaceを通じてRelationへ参加する。StateInstanceはTemplateをpayload付きで
具体化したRuntime上の個体であり、TemplateそのものやSpace判定器とは別の存在である。

```text
StateInstance {
  state_id: StateId
  template: TemplateId
  payload: Vec<TermId>
}
```

`state_id`はRuntimeが発行する個体IDである。`template`はTermの形と遷移規則を指す。
`payload`は表示文字列やコピーした値ではなく、順序付きのTerm ID列である。payloadの個数と各位置の
意味はTemplateが解釈する。同じTemplate・同じpayloadを持っていても、異なる`state_id`を持つものは
別個体とする。

構造的な二項捕捉だけは、遷移元よりpayloadがちょうど一つ多い別Templateへ遷移しなければならない。
一方、`Tick`と`End`は外部入力を伴わない自己変化なので、遷移先payloadの個数を自由に増加・維持・
減少できる。各Templateの個数は、payload 0項のroot、捕捉遷移、および制御RelationのSayo本体から
静的に一意に導出する。不一致、到達不能、および正の捕捉cycleはWorld検証エラーである。

#### payload検証 — Bootstrap実装済み

初期実装ではschemaの検証Primitiveを`CheckSpace`に限定する。複合条件やユーザー定義検証Primitive
（将来の完全なschema実行を含む）は、必要性と安全な実行制限が確認できるまで導入しない。
schema検証成功時の`Pass{}`は、現段階ではschema評価専用の一時状態であり、通常のユーザー値ではない。
将来、成功値を一般Primitiveとして公開する見込みが立った場合は、公開値としての残存・作用・終了規則を
確定したうえで、この内部状態を拡張または置換する。

Templateに`template.sayo` schemaがある場合は、payloadを検証Contextとして制限付きSayo評価を行う。
schemaは検証処理が成功した場合に通過し、`Trap`、`NoStep`、または制限超過の場合は不通過とする。
schema実行では通常のNative Relation、外部ファイルアクセス、外部入力、副作用を禁止し、schema専用の
`CheckSpace`と`Pass`だけをSayoVM内部で提供する。検証には専用のstep limitを適用する。

schemaがない場合はRuntimeが既知のBootstrap規則を検証し、未知のTemplateは遷移時に検証する。payload式は
一項へ評価されなければならず、payload不足・過剰、Space不一致、範囲外の`%n`参照は`Trap`とする。

schema内では、ワールド直下に定義されたSpace名（例: `Int`）をSpace名リテラルとして扱う。
これは値としてSpace判定器へ渡すのではなく、`CheckSpace{%0, Int}` のように検証対象の所属先を指定するための参照である。
この解釈はschema評価中に限り、通常の評価で値を生成するAtomとは区別される。
検証Primitiveはschema側で状態Termとして構成する（例: `CheckSpace{%0, Int}`）。そのTickが検証成功時に
内部`Pass{}`へ遷移し、失敗時はTrapする。

Templateは右項の自動捕捉、`Tick`、`End`ごとの遷移規則を持つ。状態遷移は`NoStep`、`Trap`、または
空を含む項列置換を返す。初期モデルでは一つのTemplateにphaseタグを持たせるのではなく、`Each`、`EachApply`、
`EachEmit`のようにphaseごとにTemplateを分ける。

通常の二項RelationはSayo実装を実行せず、現在payloadの末尾に右項を追加した新しいStateInstanceを生成する。
遷移先はRelationパスのResult Templateだけで決まり、Relationファイル内でResult名を再宣言しない。
Runtimeは新しいState IDを割り当て、Templateのpayload数とschemaを検証する。Result Templateのpayload数は、
現在payload数よりちょうど一つ多くなければならず、不一致はWorld検証エラーとする。

StateInstanceは不変である。Templateの遷移規則はpayloadの位置参照（`%0`、`%1`、`%2`）で値を
参照できるが、既存の
StateInstanceを破壊的に更新できない。状態が変化する遷移は、必ず新しいState IDを持つStateInstanceを
生成し、古い状態を含む現在セルを項列置換で置き換える。State IDはRuntimeの個体管理用であり、
Relation実装から参照・比較・更新できる言語上の値ではない。

遷移規則のスコープは、`Tick`と`End`ではpayload参照だけを提供する。右項の取り込みはRuntimeが行うため、
Sayo Relationへ入力束縛を提供しない。`End`はTerm値として公開しない。

`Tick`と`End`のSayo実装は、Relationパスが示す遷移先状態のpayloadだけを生成する。現在payloadのうち、
次payloadの式から参照されなかったTermは、元の順序を保って遷移先状態の左へ放出する。式の内部で
参照された`%n`も使用済みとみなし、暗黙に破棄されるpayloadは存在しない。Sayoファイル参照を介して
参照先から使われた`%n`も同じ使用済み集合へ含める。

制御Relationの`main.sayo`では、トップレベルの各項を独立した次payloadスロットとして評価する。
スロット同士を通常の作用として評価しない。各スロットは正確に一項へ収束しなければならず、複合した
作用を一つのスロットに入れる場合は括弧で囲む。

```text
%0 (%0 %1)
```

これは第0スロットへ現在payloadの`%0`を保持し、第1スロットへ`%0 %1`の評価結果を入れる。
通常のSayo Relationとユーザー入力では、従来どおり全体を一つの左結合項列として評価する。

SayoVMは任意個のpayloadを持てる予約内部Template `Vanish`を持つ。`Vanish{payload...}`は次の`Tick`で
空項列へ置換され、状態の殻と保持するpayloadをまとめて消失させる。右項を消費せず、通常のSpaceや
Relationへ参加しない。

消滅させたいpayloadは、固定Resultが`Vanish`である遷移の`main.sayo`から明示的に参照する。参照された
payloadは`Vanish`へ移るため左へ放出されず、次の`Tick`で状態とともに消える。参照しなかったpayloadは
通常規則どおり左へ放出される。空のResultディレクトリだけを置いた場合は、全payloadを放出して
`Vanish{}`へ遷移し、状態の殻だけが消える。

```text
DropOne{discarded, kept}
→ kept Vanish{discarded}
→ kept
```

```text
SumEmit{6}
→ 6 Vanish{}
→ 6
```

`Vanish`だけはTemplateごとの固定payload項数規則の例外である。これは異なる個数の値を同じ消失状態へ
集められるようにするためである。

Native状態機械は、このprotocolと同じ意味論を短縮して実行する最適化実装である。
`Template { payload... }`は正式な状態構築構文であり、新しいStateInstanceをRuntimeメモリ内に作る。

状態遷移Relationで用いる参照構文は`%n`だけである。

```text
%0
%1
```

`%n`は現在の不変StateInstanceのpayloadを位置で読み取る射影構文であり、メソッド呼び出しや
一般的なオブジェクト属性アクセスを導入しない。
位置番号は0始まりで、`%0`がpayloadの先頭を表す。負数、範囲外の番号、整数以外の添字は`Trap`とする。
World検証では、通常Relation、`Tick`、`End`、schema、およびそれらから参照されるSayoファイル内の
`%n`を、左Templateから静的導出したpayload個数に照らして検査する。schemaから外部Sayoファイルを
参照することは、制限付き検証Contextの規則によりWorld検証エラーとする。
構造的な二項Capture RelationにはSayo実装がなく、Runtimeが右項を一つ追加する。Sayo実装を持つ通常の
二項Relationと、捕捉後の`Tick`／`End`は、左項がStateInstanceであれば現在payloadを参照できる。
payload 0項のsource-backedな左項から`%n`を参照する定義は静的検証で拒否する。

World直下の各ディレクトリはTerm Templateを表し、通常RelationのLeft / Right / Resultと同じ名前空間を使う。

```text
world/
  Each/
    template.sayo
    .any/
      EachApply/
        main.sayo

  EachApply/
    template.sayo
    .tick/
      EachEmit/
        main.sayo
```

`template.sayo`はRelation実装やfield宣言ではなく、payloadを検証する制限付きSayoプログラムである。
payload個数はWorldの構造的Captureと制御Relationから静的に導出する。二項作用は
`<Template>/<RightSpace>/<Result>/main.sayo`、Tick遷移は`<Template>/.tick/<Result>/main.sayo`、
End遷移は`<Template>/.end/<Result>/main.sayo`で定義する。`.any`は任意の右Termに適用できる予約
RightSpaceであり、すべてのTemplate Relationに使える。具体的なRightSpaceと`.any`が
ともに適用可能なら、具体的なRightSpaceだけを優先し、具体Relationが`NoStep`を返しても`.any`へ
フォールバックしない。どちらの候補集合内でも複数Relationがあれば、通常どおり曖昧Relationとして
`Trap`とする。Result位置の`.any`は、実行結果の右端anchorを任意Templateにできる動的Result slotである。
動的Result Relationも一項以上の項列を返さなければならないが、固定Result Templateとのanchor照合は
行わない。`.tick`、`.end`、`.any`はいずれも通常のTemplate名でなく、World構造における予約制御slotである。

SayoVMは非公開の標準`Trap`を提供する。Sayo Relationの評価結果にAtom `Trap`が現れた時、Runtimeは
その列をNormalFormとして返さず直ちにSemantic Trap
として停止し、SRIへTrap eventを記録する。`Trap` Template自体の停止意味論はSayoVMが所有し、Worldは
上書き・再定義できない。`Trap`はfieldを持たない。

`space.sayo`を持つComponentは、ソース文字列を分類してTermをmaterializeできるSpaceでもある。
`template.sayo`と`space.sayo`は同じComponentに共存できる。Space判定器を持たない内部Templateは、構造的な
捕捉Relation、Template制御Relationの遷移先、または明示的な`Template { payload... }`で生成される。
ユーザーが同名Atomだけからmaterializeすることはできない。

#### 8.6.1 For Templateの標準World — 実装済み

次は`For`を構造的捕捉とTickだけで書く参照Worldである。空のResultディレクトリが右項をpayload末尾へ
捕捉するため、捕捉Relationには`main.sayo`を置かない。

```text
world/
  For/
    .any/
      Each/

  Each/
    .any/
      EachApply/

  EachApply/
    .tick/
      EachEmit/
        main.sayo

  EachEmit/
    .tick/
      Each/
        main.sayo
```

```text
# EachApply/.tick/EachEmit/main.sayo
%0 (%0 %1)

# EachEmit/.tick/Each/main.sayo
%0
```

これが表すRelationは次である。

```text
For Any : Each
Each Any : EachApply
EachApply : EachEmit
EachEmit : Each
```

`EachApply`のpayloadは`[transform, input]`であり、Tickは`[transform, transformed]`を次の`EachEmit`へ
渡す。次のTickは`%0`だけを新しい`Each`へ渡すため、未参照の`%1`が左へ放出される。各Templateには
`.end`を置かないため、`For`は開いた変換器として継続Termを残す。

```text
For (Add 9) 1
→ Each{Plus{9}} 1
→ EachApply{Plus{9}, 1}
→ EachEmit{Plus{9}, 10}
→ 10 Each{Plus{9}}
```

この例では、右項捕捉によるpayload増加、Tickによる新状態payloadの生成、未参照payloadの左放出を確認できる。

#### 8.6.2 Fold Templateの標準World — 実装済み

`Fold`は`For`の参照Worldに加え、accumulatorの更新と`End`によるCollectorの終了を検証する。

```text
world/
  Fold/
    space.sayo
    .any/
      FoldInit/

  FoldInit/
    .any/
      FoldAccum/
    .end/
      Trap/
        main.sayo

  FoldAccum/
    .any/
      FoldApply/
    .end/
      FoldClose/
        main.sayo

  FoldApply/
    .tick/
      FoldNext/
        main.sayo

  FoldNext/
    .tick/
      FoldAccum/
        main.sayo

  FoldClose/
    .tick/
      Vanish/
        main.sayo
```

```text
# FoldInit/.end/Trap/main.sayo
Trap

# FoldApply/.tick/FoldNext/main.sayo
%0 (%0 %1 %2)

# FoldNext/.tick/FoldAccum/main.sayo
%0 %1

# FoldAccum/.end/FoldClose/main.sayo
%0 %1

# FoldClose/.tick/Vanish/main.sayo
%0
```

`FoldClose`は`[transform, accumulator]`を保持する。最後のTickは`transform`だけを`Vanish`へ移すため、
未参照のaccumulatorが左へ放出され、その次のTickでtransformとVanish状態が消える。

```text
Fold Add 0 1
→ FoldInit{Add} 0 1
→ FoldAccum{Add, 0} 1
→ FoldApply{Add, 0, 1}
→ FoldNext{Add, 1}
→ FoldAccum{Add, 1}
→ FoldClose{Add, 1}       ; End
→ 1 Vanish{Add}           ; Tick
→ 1                        ; Tick
```

`FoldInit`は`.end/Trap`により、初期値を受け取る前に終了するとTrapする。`FoldAccum`だけが
`.end/FoldClose`で正常終了できる。accumulatorはSpaceを限定せず、そのまま左へ放出される。

### 8.7 Reduction eventとSayo Visualization IR（SVI）— 実装済み

SayoVMはReducerの進行を構造化された内部SRIイベントとして記録する。SRIは診断とRust SVI Builderの
入力に限り、Visualizerへ直接公開しない。イベントではRuntime内のTerm、項列Cell、評価Group、
Template制御処理をそれぞれ一意なIDで識別する。イベントには少なくとも次がある。

```text
group            評価グループの開始
materialize      遅延セルをTermへ評価
attempt          隣接する二項への作用を試行
not_applicable   `NoStep`として次の隣接対へ進んだ
rewrite          二セルを一つ以上の新しいセルへ置換
state            一セルのRuntime内部状態を一つ以上のセルへ遷移
end              Group末尾の内部`End`を受理して状態遷移
yield            グループのNormalFormを構成するTerm
trap             評価失敗
```

CLIでは診断用の内部SRI JSONを`--trace-format json`で出力できる。

```sh
sayo --trace-format json --world examples/native_arithmetic/world add 5 3
```

SRIはstderr、NormalFormの表示はstdoutへ出力する。通常評価はトレースを収集しない。
Rust APIでは`Runtime::evaluate_with_trace`と`Runtime::evaluate_file_with_trace`が、成功・失敗の
結果と`SayoTrace`をまとめて返す。

JSONは`format: "sayo-reduction-ir"`、整数`version: 1`、順序付き`events`を持つ単一ドキュメントとする。
イベントのGroup／Cell／Term IDは装飾なしの整数であり、挿入セルは`{ "cell", "term" }`として表す。
成功時もTrap時もstderr全体が一つの妥当なJSONであり、Trap時のプロセス終了statusは失敗となる。
version 1の内部契約は`sri-json.md`を正本とする。

Web APIはRust側でSRIイベントを完全なSVIフレーム列へ変換する。各Frameは単独でScene全体を持ち、
構造化Term、Focus、直前Frameからの`preserve`／`replace`／`rewrite`／`copy`／`remove`を明示する。
表示列が直前Frameから変化しないSRIイベントはSVI Frameへ追加しない。したがってSRIはReducerの
全探索過程を保持し、SVIは表示可能な状態変化だけを保持する。Trap Frameはこの省略対象外とする。
Visualizerはソースを解析せず、状態を再生せず、この確定済みフレーム列の表示だけを担う。
公開契約の詳細は`svi-json.md`を正本とする。

### 8.8 評価グループ終端 — 実装済み

`;`は導入しない。

Collectorのように複数の右項を受け続けるTermの入力終了は、新しい構文ではなく既存の
評価グループ終端で表す。

- CLI入力全体の終端
- Sayoファイル全体の終端
- `(...)`内部の終端

RuntimeはGroup末尾で、最後のRuntime状態へ内部的な`End`を試せる。`End`はユーザーが書くTermや
Spaceではない。現在は`FoldAccum`が`End`を受け、`FoldClose`を経て最終値へ収束する。
Collection、Stream、SpreadはSayoの構文・意味論として導入しない。複数値は新しいCollection値へ
包まず、Flatな項列として処理する。複数の右項を受け取る状態付きTermは、評価グループ終端で内部的な
`End`を受け、累積結果へ収束できる。

例えば、`Sum 1 2 3`という入力列では、`Sum`が右項を順に取り込み、Group末尾で内部`End`を受ける。その状態は、
同じ累積状態を持つ別のSpace（例：`SumEmit{6}`）へ遷移する。`SumEmit{6}`は値`6`を項列の左へ
放出し、その後に予約内部状態`Vanish{}`へ遷移する。`Vanish{}`は次の評価で空項列へ置換される。

従って、結果の出力と状態付きTermの消滅は別々の遷移である。`End`はユーザーが書くTermではなく、
Runtimeがグループ終端で発生させる内部イベントである。

## 9. 標準ライブラリと標準Native Relation

標準WorldとユーザーWorldは別のルートとして管理する。標準WorldはSayoVMのバイナリへ埋め込み、
起動時に独立した`MemoryWorld`として構築する。ユーザーWorldのディレクトリへ標準ファイルを混在させたり、
標準Worldから生成物をユーザーWorldへ逆流させたりしない。この分離基盤は実装済みであり、
現在の埋め込み標準Worldはバージョンマーカーに加えて、
`For`、`Filter`、`Fold`、`Reduce`、`Duplicate`のCapture／Tick／End Relationを持つ。

Component探索は、ユーザーWorld、埋め込み標準World、標準Nativeの順に行う。ユーザーWorldに
同名Componentが存在すれば、判定器、schema、通常Relation、Capture、`Tick`、`End`を含む標準Worldの
同名Component全体をshadowする。ユーザーComponent内に該当Relationがなくても、同名の標準Componentへ
ファイル単位ではフォールバックしない。同一Component内で複数候補がある場合は曖昧性エラーとする。

entry Componentのshadowによって到達不能になった標準Worldの内部状態Componentは、有効Worldから除外する。
これにより、同じ状態機械の標準実装とNative fallbackを途中で混在させない。ユーザーWorld自身にある
到達不能Templateは引き続きWorld検証エラーである。標準World単体の検証でも全Templateの到達可能性を要求する。

### 9.1 標準Native Relation — 実装済み

SayoVMは、ユーザーWorldから隠された標準Primitiveを提供する。`Duplicate`、`For`、`Filter`、`Fold`、`Reduce`の同名Native Relationは、
標準World定義が利用できない場合の意味論互換fallbackである。
Sayo実装だけでなく、空Resultディレクトリによる構造的CaptureもWorld RelationとしてNativeより
優先する。Nativeと同じ署名のCaptureを暗黙にNative宣言として扱うことはない。

Nativeは単なる非常用fallbackに限定せず、Sayo Relationと観測上同値な最適化実装として拡張できる。
同値性には、適用可能な入力、Result Space、最終項列、`NoStep`／`Trap`条件、左への放出順、
Component shadow、StateInstanceの個体性を含む。Nativeは複数のSayo遷移を短縮してよいため、内部遷移数と
SRIの細かな軌跡までは一致条件にしない。SRIには実際に実行されたNative最適化遷移を記録する。

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

Bootstrap段階の`Int`は`i64`である。Native算術は空白を除いた`i64`表現を受け取り、
結果を正規化した10進表記で返す。オーバーフローとゼロ除算は`Trap`である。

Bootstrap Native比較はInt同士またはString同士の`Compare left right`を`Ordering` Spaceの不変Term
`Less`、`Equal`、`Greater`のいずれかへ遷移させる。途中状態は`CompareInt{left}`または
`CompareString{left}`である。これらはBooleanではなく比較結果を分類する
暫定Primitive protocolであり、専用`if`や真偽値の評価規則は導入しない。ユーザー定義Spaceへ比較を
一般化する意味論は保留する。

標準WorldはSpace Checker向けのSayoアダプター`equal-to expected actual`も提供する。一致時は入力Stringを
一項返し、不一致時は`Miss`へ遷移する。最終的な文字列等値判定だけは制限付きNative Primitiveであり、
通常評価から直接呼び出すことはできない。

`For`は任意のTermを変換項として受け付ける。`For (Add 9) 1 2 3` は、内部で
`Each{Plus{9}}` を作り、各入力について `EachApply`、`EachEmit`を経て
`10 11 12 Each{Plus{9}}` になる。各作用が一項の結果を返す範囲で、Sayo Termも
変換項にできる。`For`は**開いた変換器**であり、Group末尾の`End`には反応しない。従って
`Each{...}`は正当なNormalFormの継続Termであり、外側の項列へ展開された後に右項が接続されれば
再び遷移できる。

`Filter`はpredicateを各入力へ一回作用させる。predicate結果は、互いに異なる標準Space
`Keep`または`Drop`に属する一項である。これは一般的なBooleanではなく、Filterの遷移を選択する
ための二つの通常Spaceである。

- Native predicateは結果TermのSpaceとして`Keep`または`Drop`を返す（`Odd`が参照実装）。
- Sayo RelationはResultSpaceを`Keep`または`Drop`として宣言する。
- Sayo実装では、標準Primitive Atom `keep`または`drop`を対応する結果として利用できる。

predicateが`Keep`と`Drop`以外を返した場合は`Trap`となる。
`Filter`も`For`と同じく開いた変換器であり、`Sieve{...}`は`End`で閉じない継続Termである。

```text
Filter Odd 1 2 3
→ Sieve{Odd} 1 2 3
→ SieveApply{Odd, 1} 2 3
→ SieveChoose{Odd, 1} Keep 2 3
→ 1 Sieve{Odd} 2 3
→ 1 SieveApply{Odd, 2} 3
→ 1 SieveChoose{Odd, 2} Drop 3
→ 1 Sieve{Odd} 3
→ 1 3 Sieve{Odd}
```

`SieveApply`のTickは動的Result `.any` を使い、左に`SieveChoose`、右端anchorにpredicate結果を返す。
Result位置の`.any`は通常RelationとTickで同じ意味を持ち、右端anchorのSpaceを固定しない。
`SieveChoose Keep`と`SieveChoose Drop`は通常Relationとして分岐する。内部の`SieveEmit`／
`SieveDiscard`はRelation評価内で遷移し切り、いずれも継続状態`Sieve`へ戻る。

`Fold`は初期値を必須とし、Group末尾の内部`End`で終了する。`Fold Add 0 1 2 3`は
`FoldClose{6}`を経て`6`へ収束する。`Fold Add 0`は`0`を返す。各結合は一項結果でなければならない。

`Reduce`は`Fold`の初期値なし版である。`Reduce Add 1 2 3`は最初の入力`1`を蓄積値として採用し、
`ReduceClose{6}`を経て`6`へ収束する。入力を一つだけ与えた場合はその入力を返す。入力を与えず
Group末尾に達した場合は、蓄積値を定められないためRuntimeの`Trap`となる。各結合は一項結果でなければ
ならない。

このため、標準状態Termは次の二種に分類する。**開いた変換器**（`For`、`Filter`）は入力を受けて値を
左へ放出しつつ継続Termを残し、`End`には`NoStep`を返す。**Collector**（`Fold`、`Reduce`）は蓄積値を
保持し、`End`で閉じて一項の結果へ収束する。閉じたCollectionを作る機能は現仕様に含めない。

## 10. World Validation — 実装済み

`sayo check --world <path>`は、実行前に少なくとも次を検証する。

- Worldがディレクトリであること
- source-backed Spaceに`space.sayo`があること
- 実装を持たない内部TemplateをSpaceと区別して受理すること
- RightとResultが既知のSpace／Templateまたは予約スロットであること
- 空のResultディレクトリを構造的捕捉Relationとして受理すること
- `.tick`と`.end`の本文あり遷移がSayo実装であること、および本文なし固定Resultがpayload 0項として
  一意に整合すること（`Trap`と`.any`は本文必須、`Vanish`は予約消失遷移）
- Sayo実装の構文が正しいこと
- `template.sayo` schemaのSayo構文が正しいこと
- `space.sayo`を持つComponentだけがsource-backed TermをmaterializeできるSpaceであること
- RelationのRightはTemplate名または予約スロット`.any` / `.tick` / `.end`として検証すること
- World内に`main.native`がないこと
- 埋め込み標準Worldとの完全shadow後に、有効なTemplateのpayload arityを一意に導出できること
- shadowで残る標準Relationとの接続を含め、Runtimeを構築できること

検証はユーザーWorld単体の構文・構造検証と、標準Worldを合成した有効World検証の両方を行う。
成功レポートのSpace、Template、Relation、実装件数はユーザーWorld所有の宣言だけを数え、標準Worldと
Native fallbackは件数へ含めない。

## 11. Bootstrap Runtimeの保護 — 実装済み

初期実装の保護として次の**既定値**を置く。

- Relation評価深度: 256
- 状態遷移: 100,000

これらは言語としての計算資源制限を確定するものではない。Rust APIの`RuntimeLimits`で
Runtimeごとに設定できる。将来、解除・より適切なプロセス管理へ移行できる。
Relation評価深度は意味論的なRelation呼び出しフレームだけを数え、Flat項列の幅は数えない。

## 12. 現在の実装範囲

現在のSayoVMで利用できる主な機能は次のとおりである。

- CLI式の評価
- `.sayo`ファイルの評価
- Sayo Relation、構造的な右項捕捉、payload位置参照`%n`
- 引用符付きUTF-8 `String` Term
- `space.sayo`によるユーザー定義Space判定（Match／`Miss`／`Trap`）
- Native整数四則演算
- NativeのInt／String比較と`Ordering`
- Space Checker用の標準Sayo `equal-to`
- Native内部状態遷移（標準World不在時の`Duplicate`／`For`／`Filter`／`Fold`／`Reduce` fallback）
- 埋め込み標準Worldによる`Duplicate`／`For`／`Filter`／`Fold`／`Reduce`状態機械
- World検証
- エラー分類表示
- 内部SRI JSON診断（`--trace-format json`）
- Web APIの完全なSVIフレーム列と描画専用Visualizer

空のユーザーWorld overlayでもNative Primitiveと標準Sayo Worldを利用できる。

REPLは安定稼働後まで保留する。CollectionやSequence値は導入せず、複数結果と入力列はFlatな項列として
実装している。汎用状態付きTermは構造的捕捉RelationとTick／End Relationで表現できる。

## 13. 保留事項

次の事項は、必要になるまで決めない。

### 条件分岐

Sayoは専用の`if`構文、Boolean、真偽値専用の評価規則を導入しない。
条件分岐は、Spaceへの所属判定とRelationの適用可否で表現する。
条件に該当しない場合はRelationが適用されず、次の候補へ進む。

実装は次の順序で拡張する。

1. Space判定による単一条件
2. 複数Relationによる条件不成立時の分岐
3. 複数Spaceによる条件の表現
4. Schema専用またはNativeの判定Primitive

Spaceの合成による複合条件（`And`、`Or`、`Not`など）は保留する。

次の項目は、初期SayoVMの安定稼働後に再検討する。

- REPL
- モジュールとパッケージ
- セルフホストへの具体的な移行手順

Native Relationのユーザー拡張は導入しない。Native実装はSayoVMの組み込みとして固定する。
