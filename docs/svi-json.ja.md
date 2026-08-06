# Sayo Visualization IR JSON — version 1

[English](svi-json.md) | 日本語

SVIは、SayoVMの評価をVisualizerが意味論を推測せず描画するためのHost向け中間表現である。
コンパイラ最適化やJITのためのIRではない。現在のSRIイベントをRust側のSVI Builderが完全な
表示フレーム列へ変換し、Visualizerは状態復元を行わず各フレームを描画する。

```text
SayoVM -> internal Reducer events -> Rust SVI Builder -> SVI JSON -> Visualizer
```

Web API version 1はこのSVI JSONを返す。SRI JSONはSayoVM内部の診断用Reducerイベント形式として残す。

## Document

```json
{
  "format": "sayo-visualization-ir",
  "version": 1,
  "result": {
    "status": "normal",
    "terms": ["10", "11", "12", "Each{Plus{9}}"]
  },
  "frames": []
}
```

`result.status`は`normal`、`trap`、`limit`のいずれかである。各Frameは単独で完全な画面を
復元できなければならない。Readerは前FrameからSceneを再構成してはならない。
SVI Builderは前Frameと表示上同一のSceneを持つReducer eventをFrameへ追加しない。探索、判定、
Relation試行などの全イベントはSRIに残るが、SVIの再生時間は実際に表示列が変化した回数に比例する。
Trapは停止を表示する必要があるため、Sceneが同一でもFrameとして保持する。

## Frame

```json
{
  "frame_id": 14,
  "cause": {},
  "scene": { "rows": [] },
  "focus": { "view_ids": [] },
  "changes": []
}
```

- `frame_id`は文書内で一意な連番
- `cause`はFrameが存在する意味と、元Reducer eventとの対応を表す
- `scene`はその時点で描画する完全な表示状態
- `focus`は自動スクロールやTrap表示で注目する対象を`view_id`で指定する
- `changes`は直前Frameとの対応を明示する。先頭Frameでは空配列

## Scene rows

```json
{
  "rows": [
    { "row_id": "context-3", "role": "context", "separator": "space", "items": [] },
    { "row_id": "reduction-4", "role": "reduction", "separator": "space", "items": [] }
  ]
}
```

version 1の`role`は次の四つとする。

- `main`: 通常の評価列
- `context`: 子Group評価中に保持する祖先の列
- `template`: 内部評価中に固定表示する制御Template
- `reduction`: 現在評価している子Groupまたは制御payloadの列

ネストしたGroupの評価では、初期の外側列から直接の呼び出し元まで、すべての祖先`context` Rowを順番に置き、
最後に現在の`reduction` Rowを置く。これにより評価が内側へ進んでも起点となる式を表示し続ける。
Rowの順序が画面上の上から下の順序である。VisualizerはGroup親子関係からRowを推測しない。
`separator`は`space`または`comma`である。通常のSayo列とGroup展開は`space`、State payloadを並べる
overviewは`comma`を使う。VisualizerはRow名やroleから区切り記号を推測しない。
Template制御の内部評価では、`context`、制御Template、現在評価中のpayloadを別々のRowとして表せる。
同じ`reduction` roleを持つRowが複数存在してよく、Visualizerはそれらを一行へ平坦化しない。
制御TemplateのRowは各slotの内部評価中には書き換えず、placeholderを含む元の形を保持する。値の変化は
作業Rowだけに現れる。全slotの解決後は、detailをoverviewへ確定する`control_completed`、overviewを
完成したTemplateへ組み立てる`control_assembled`、完成状態を呼び出し元へ返す`control_returned`の順に畳む。
作業Rowはslot全体を保持する`overview`と、内部展開中のslotだけを示す`detail`に分けられる。単純なslotは
overview上で置換し、Groupのslotはoverviewを保持したままdetailを一段下へ追加して評価する。detail完了後に
その結果をoverviewへ戻す。

## View item and identities

```json
{
  "view_id": "view-28",
  "cell_id": 12,
  "term": {}
}
```

- `term_id`はSayoVM上のTermの同一性
- `cell_id`はReducer列上のCellの同一性
- `view_id`は画面上の出現個体の同一性

同じTermを複数箇所へ表示する場合は、同じ`term_id`と異なる`view_id`を持たせる。
`focus`と`changes`は表示上の曖昧さを避けるため`view_id`を参照する。

## Structured terms

Visualizerは`source`を解析しない。Termは`atom`、`state`、`group`、`placeholder`のいずれかとして
再帰的に構造化する。`source`はコピー、ログ、アクセシビリティ用途のcanonical表示として残す。

### Atom

```json
{ "kind": "atom", "term_id": 8, "source": "9", "space": "Int" }
```

### State

```json
{
  "kind": "state",
  "term_id": 20,
  "source": "Plus{9}",
  "space": "Plus",
  "head": "Plus",
  "payload": [
    {
      "view_id": "view-20-payload-0",
      "term": { "kind": "atom", "term_id": 8, "source": "9", "space": "Int" }
    }
  ]
}
```

### Group

```json
{
  "kind": "group",
  "term_id": null,
  "source": "(%0 %1)",
  "items": [
    { "view_id": "group-4-0", "term": { "kind": "placeholder", "term_id": null, "source": "%0", "name": "%0" } },
    { "view_id": "group-4-1", "term": { "kind": "placeholder", "term_id": null, "source": "%1", "name": "%1" } }
  ]
}
```

## Cause and control identity

```json
{
  "kind": "control_slot_resolved",
  "source_event": 14,
  "group_id": 4,
  "control_id": 3,
  "slot": 0
}
```

`control_id`はRuntimeが発行する一意IDであり、親Group、Template名、実装ソースから推測しない。
一つのReducer eventから複数の表示Frameを作れるため、複数Frameが同じ`source_event`を持ってよい。

制御処理では少なくとも`control_started`、`control_payload_exposed`、`control_slot_started`、
`control_slot_resolved`、`control_completed`、`control_assembled`、`control_returned`を表現できるものとする。

## Focus

```json
{
  "view_ids": ["view-28", "view-29"],
  "control_id": 3,
  "style": "active"
}
```

`style`は`active`または`trap`。`active`は注目対象、`trap`は失敗位置を表し、具体的な発光・配色は
Presentation側が決める。Visualizerは対象をSceneから推測しない。

## Changes

```json
{
  "kind": "rewrite",
  "from": ["view-plus-9", "view-int-1"],
  "to": ["view-int-10"],
  "operation": "relation_applied"
}
```

version 1の`kind`は`preserve`、`replace`、`rewrite`、`copy`、`remove`とする。`from`と`to`を
明示し、Visualizerは文字列一致や位置から項の継続、複製、統合、消失を推測しない。

## Presentation boundary

SVIは「何が起きたか」を確定するが、「どう見せるか」は指定しない。座標、色、CSS class、時間、
easing、移動方向をSVIへ含めない。VisualizerはFrame、Focus、Changesを変更せず、表示方法だけを選ぶ。

## Builder architecture

Rust SVI BuilderはSceneを直接分岐で組み立てず、順序付きの`ScenePlan`へ
`Context`、`Template`、`Overview`、`Detail`のLayerを積み、その計画を完全なRow列へ投影する。
Layerの選択とTermの構造化は別責務とし、今後Detailが深くなってもRow生成規則を追加できる形を保つ。

Builder内部では、Reducer由来の意味論的なViewと、Template／Overview／Detailに複写された表示用Viewを
明示的に区別する。この内部属性はJSONへ出力しない。公開SVIでは、その判定を済ませた`changes`だけを
Visualizerへ渡す。これにより、表示用の複写を意味論上の`copy`として誤認しない。
