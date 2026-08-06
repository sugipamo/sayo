# Sayo Reduction IR JSON — version 1

[English](sri-json.md) | 日本語

SRI JSONは、SayoVMのReducerイベントを記録・診断し、Rust SVI Builderへ渡すための内部形式である。
Visualizerへ直接渡す公開描画契約ではない。
言語のNormalFormやSayo Termではない。

Visualizerとの公開契約は、完全な表示フレーム列を持つ`svi-json.md`である。
SRIはSayoVM内部のReducerイベント記録であり、VisualizerはSRIを直接再生しない。

## Document

一回の評価を一つのJSONドキュメントとして表す。

```json
{
  "format": "sayo-reduction-ir",
  "version": 1,
  "events": []
}
```

- `format`は常に`"sayo-reduction-ir"`
- `version`はこの契約では整数`1`
- `events`は観測順を保持する配列
- 未知のversionをversion 1として解釈してはならない

CLIの`--trace-format json`はこのドキュメントだけをstderrへ出す。成功時のNormalFormはstdoutへ出る。
Trap時はstdoutを空にし、Trapイベントを含む妥当なJSONをstderrへ出した後、失敗statusで終了する。
stderrへ別の表示を追加する`--show-space`／`--show-kind`とは同時指定できない。

## IDs

Group、Cell、TermのIDはJSONでは非負整数として表し、表示用の接頭辞を含めない。

- Group IDは一回のTrace内で一意
- Cell IDは一回のTrace内で一意。書き換えで削除されたCell IDを再利用しない
- Term IDはRuntime内のTermを参照する。同一Termが別Cellへ移ってもTerm IDは維持される
- materialize前などIDを持たないTermの`term_id`は`null`

## Term and Relation

Termは次の形で埋め込む。

```json
{
  "term_id": 1,
  "source": "add",
  "space": "Add",
  "implementation": "atom",
  "fragments": []
}
```

`source`はトレース上の安定した表示文字列であり、Native状態にも`Plus{5}`のような表示を持たせる。
`implementation`はversion 1では`atom`、`sayo`、`native`のいずれかである。

状態付きTermでは、省略可能な`fragments`が表示内部に保持されたTermの由来を表す。

```json
{
  "term_id": 12,
  "source": "EachApply{Plus{9}, 1}",
  "space": "Each",
  "implementation": "native",
  "fragments": [
    { "path": [0], "origin_id": 8 },
    { "path": [0, 0], "origin_id": 3 },
    { "path": [1], "origin_id": 10 }
  ]
}
```

- `path`は`source`の状態payloadを外側から辿るzero-based index列
- `origin_id`はその位置に保持されているTerm ID
- 同じ`origin_id`が遷移前後にあれば、表示文字列や位置が変わっても同一Termの継続である
- `fragments`が空の場合はJSONから省略される。古いversion 1 Readerは未知fieldとして無視できる

Relationは次の形で埋め込む。

```json
{
  "left_space": "Add",
  "right_space": "Int",
  "result_space": "Plus",
  "executor": "native"
}
```

## Events

全イベントは文字列`event`を持つ。version 1のevent tagは次のとおり。

| event | 主なfield | 意味 |
|---|---|---|
| `group_started` | `group`, `parent`, `cells`, `sources` | 評価Groupと未評価の初期ソース列を作る |
| `materialized` | `group`, `cell`, `term` | 遅延CellをTermへ評価する |
| `attempted` | `group`, `left_cell`, `right_cell`, `left`, `right` | 隣接二項への作用を試す |
| `not_applicable` | `group`, `left_cell`, `right_cell`, `reason` | `NoStep`として右へ進む |
| `rewritten` | `group`, `removed_left`, `removed_right`, `inserted`, `relation` | 二CellをRelation結果へ置換する |
| `state_transition` | `group`, `removed`, `inserted`, `control_id?` | Tickによる一Cellの状態遷移 |
| `end_transition` | `group`, `removed`, `inserted`, `control_id?` | Group末尾のEnd遷移 |
| `cursor_moved` | `group`, `from`, `to`, `reason` | 明示的なScheduler cursor移動 |
| `yielded` | `group`, `cell`, `term` | GroupのNormalFormへTermを残す |
| `trapped` | `group`, `category`, `message` | 評価を失敗で停止する |

`parent`、`from`、`to`、Trap発生位置がGroup外の場合の`group`は`null`になりうる。

制御Relationのpayloadを評価する`group_started`は、追加で`control`を持つ。

```json
{
  "control_id": 3,
  "transition": "tick",
  "result_template": "EachEmit",
  "implementation_source": "%0 (%0 %1)",
  "slots": ["%0", "(%0 %1)"],
  "slot": 0,
  "slot_count": 2
}
```

`control_id`は一回のTrace内で一意であり、一つの制御実行に属する全slotの子Groupで共有される。

これによりSVI Builderは、子Groupを親状態から独立した状態変化と誤認せず、どの遷移先Templateの
何番目のpayloadを組み立てているかを表示できる。通常のGroupでは`control`を省略する。

`inserted`は順序付き配列で、各要素は次の形を持つ。

```json
{
  "cell": 3,
  "term": {
    "term_id": 8,
    "source": "8",
    "space": "Int",
    "implementation": "atom"
  }
}
```

配列順が書き換え後の左から右のCell順である。空配列は状態とpayloadが消滅したことを表せる。

## Replay contract

SRI ReaderまたはSVI Builderは`events`を先頭から一度だけ適用すれば状態を再構成できる。

1. `group_started`でGroupと初期Cell順を作る。`sources`は`cells`と同じ長さで、同じindexのCellが評価開始時に保持するcanonical Sayoソースを表す。SVI Builderはmaterialize前からこの列を初期状態として構成できる。
2. `materialized`でCellへTermを関連付ける。
3. `rewritten`、`state_transition`、`end_transition`でremoved Cellを削除し、同じ位置へ`inserted`を順に置く。
4. `yielded`で最終残存Cellを確認する。
5. `trapped`があれば、その時点の状態を失敗状態として保持する。

version 1ではイベントの省略、並べ替え、Cell ID再利用を許可しない。将来ストリーミングが必要になった場合は、
この単一ドキュメントを正本に保ったまま別versionまたは別transportとしてJSON Linesを検討する。
