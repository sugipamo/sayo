# 外部参照と Host 境界

[English](external-boundary.md) | 日本語

外部効果を検討するブランチの最小プロトタイプ仕様です。実装済みの参照の受け渡しと、保留中の
外部実行方針を区別します。Sayo の評価は引き続き純粋で、Relation が Agent や Host の処理を呼ぶことは
ありません。核の評価契約は [言語仕様](language-spec.ja.md) を参照してください。

## 不透明な値

```sayo
Document<blob:input>
Agent<handle:coder>
Request{Agent<handle:coder>, Document<blob:input>}
```

- `(...)` は内部を評価する Group。
- `{...}` は Sayo Term の順序付き payload。`%0`、`%1` などで各 Term 全体を参照できる。
- `<...>` は型付きの外部参照を一つ保持する。Sayo の式や raw データを直接入れる場所ではない。
- `[...]` に新しい意味は与えない。

参照種別は Rust の `ExternalKind::Blob` と `ExternalKind::Handle` として区別します。
新しい言語上の Space を導入するものではありません。

| 種別 | Host が守る契約 |
| --- | --- |
| `blob` | Host の名前空間にある不変の値。処理後は新しい blob を作り、元の値は変更しない。 |
| `handle` | 外部で変化し得る対象への接続口。同じ handle を保存しても対象の過去の状態は保存できない。 |

ID は空でない ASCII 英数字と `_`、`-`、`.` の列です。`<>` 内で escape、式、タプル、raw データは
扱いません。ID は内容 hash、資格情報、capability、対象が存在する証明ではありません。
ソース、CLI、SRI、SVI に表示されるため、秘密の値を入れないでください。Host が実体、lookup、有効期間、
要求の検証を管理します。VM は参照先の存在確認、内容の読み取り・比較・変更を行いません。
blob の不変性は Host の責務であり、内容にアクセスしない評価器が強制できる性質ではありません。

外形の role は英字または `_` で始まる ASCII 識別子です。`space.sayo` が宣言されたユーザー Space を
指定し、Native primitive と予約名は使用できません。明示的な外部 Term の構築では role によって
Space が決まり、ソース文字列の checker は実行しません。ID や内容で分類を変えないためです。
通常の Atom は従来どおり checker を使います。role ごとの参照種別は Host が検証します。
今回のテスト Host は Agent に handle、Document に blob を要求します。

外部 Term 自体には Sayo payload がなく、Template の Tick/End 規則を実行しません。
二項 Relation は Space によって Term 全体を消費・受け渡しでき、右項の構造的 capture は外部参照を保持します。
通常の payload 0 項の左項と同様、左側の外部 Term は capture で消費され、その参照は暗黙に追加されません。
必要な参照をすべて保持するには意味論内の envelope を使います。`Agent{Ready}<handle:coder>` は保留です。
schema とソースの Space checker では外部 Term を構築できません。SVI は外部 Term を分解しない atomic な項として表示します。

## 要求、処理、固定された応答

[サンプル World](../examples/external_requests/world) は通常の capture Relation を使います。

```text
ask Agent<handle:coder> Document<blob:input> accept
→ WithAgent{Agent<handle:coder>} Document<blob:input> accept
→ Request{Agent<handle:coder>, Document<blob:input>} accept
```

Host が受け取るのは初期 Group 全体の NormalForm です。内側の Group が安定しただけでは外部実行へ
進みません。最終 NormalForm までに消えた Request は実行対象になりません。Trap や上限到達では
利用可能な部分結果を返しません。評価器は Request を自動で実行しません。

テスト Host は blob の内容を読み、`upper:` で始まる場合は後続の英字を大文字化し、そうでなければ
バイト列をコピーします。内容による分岐は Host の処理だけで起きます。新しい blob を作り、すでに
要素へ分けた固定の観測を次のように渡します。

```text
receive Agent<handle:coder> Code<blob:output-1>
→ Response{Agent<handle:coder>, Code<blob:output-1>}
→ Code<blob:output-1> Agent<handle:coder>
```

Response の外形は Sayo payload なので `%1 %0` で Code と Agent の Term 全体を渡せます。
不透明な blob を分解しているわけではありません。Agent Term は新しくなりますが、handle は同じ
接続口を指します。Agent の不変 snapshot ではありません。Host は処理済み Request を除き、
残存する `accept` の後へ応答を置いて次の評価区間へ渡します。

```text
accept Code<blob:output-1> Agent<handle:coder>
→ Completed{Code<blob:output-1>} Agent<handle:coder>
```

資格情報やネットワークを必要としない決定的な mock を実行できます。

```sh
cargo run --example external_roundtrip
cargo run --example external_roundtrip -- 'plain text'
```

既定の Host 出力は `HELLO`、Host の呼び出しは一回です。二つ目はコピーする分岐を試します。
CLI で `examples/external_requests/main.sayo` を評価した場合は Request で止まり、mock Host は呼びません。

## 組み込み API と replay

`Term::external` は Host に参照種別と ID を公開します。`Runtime::payload_terms` は Template の
意味論内 payload を返し、外部 Term や Atom には `None` を返します。外部の内容は返しません。
`Runtime::evaluate_terms` は実体化済み Term を新しい純粋な評価区間へ渡します。表示文字列をソースへ
戻さず、残存状態と固定された応答を組み合わせられます。Term は同じ Runtime が作った未変更のものに
限り、所有元と ID を検証します。空の入力は空の NormalForm になります。

固定された応答の再利用は内部の計算の replay であり、外部操作の再実行ではありません。
入力 Term、World、観測が同じなら純粋な遷移を再現できます。内部の割り当て ID は同じとは限りません。
Runtime をまたぐ状態の保存・移動は保留です。handle の保存だけでは観測や Agent の履歴は保存できません。

## 保留する方針

実ファイル、HTTP、時刻、乱数、通知は、任意の [ネイティブ Host](host-effects.ja.md) で実行できます。
その Host は登録済みの操作を逐次実行し、同じ Request 出現の再実行を防ぎます。
このページの Agent／Document の mock と同様、実行方針は評価器の外にあります。

mock は一つの Request の出現を一回処理し、明示した位置へ応答を置きます。これはテスト Host の
方針であり、言語の自動規則ではありません。同じ ID や内容の Request があっても出現を統合しません。
Scheduler の順序は Host の実行順を決めません。

実際の Agent 接続、複数の未処理要求と応答の対応、retry、cancel、並行実行、不変の Agent snapshot、
参照同一性の操作、role ごとの参照種別 schema、raw データ構文、`{}` と `<>` の併用、永続的な replay、
線形な IO capability は保留です。外部実行の失敗は「外部で何も起きなかった」ことを保証しません。
IO token を追加するなら権限と消費の規則を別途定める必要があり、不透明性だけでは線形性や原子性は得られません。
