# ネイティブ Host の副作用

[English](host-effects.md) | 日本語

`host-effects` feature は、明示的に起動するネイティブ Host を追加します。通常の `sayo`、Wasm、
Playground の評価器に副作用を組み込む変更ではありません。必要な操作だけを Host が handle に登録し、
全体の NormalForm に残った Request を処理します。

## 実行

```sh
cargo run --locked --features host-effects --bin sayo-host -- \
  --config examples/host_effects/host.json examples/host_effects/main.sayo
```

サンプルは現在時刻、OS の乱数16バイト、stderr への通知を実際に実行します。stdout には三つの
`Succeeded{Data<blob:...>}` が残り、stderr には通知と処理件数が出ます。Linux のビルドには
OpenSSL 開発用ライブラリと pkg-config が必要です。

結果の実体も保存するには `--export-blobs /tmp/sayo-observations` を追加します。結果ごとに
`ID.blob` を新規作成し、既存ファイルは上書きしません。乱数や HTTP body は UTF-8 とは限りません。
出力内容には外部データが含まれるため、export は明示的なオプションです。
操作の `Failed` は通常の応答としてプログラムへ渡るため、それだけでは CLI の終了コードは失敗になりません。
設定エラー、Host の上限、純粋評価の失敗は非0の終了コードになります。

## 要求と応答

```sayo
Request{Service<handle:clock>, Data<blob:empty>}
```

`Service` は登録された処理先の handle、`Data` は Host 内の不変な入力 blob です。応答は
`Succeeded{Data<blob:新しいID>}` または `Failed{Data<blob:新しいID>}` になります。入出力を同じ
`Data` role にするため、返された Term を次の要求へ渡せます。blob の中身は Sayo が解釈しません。

`--world` を省略した CLI は `effect_world()` のユーザー World を使います。これは Service／Data の
Space checker、`send Service<handle:...> Data<blob:...>` から Request を作る capture、成功・失敗の
capture と `template.sayo` による payload の Space 検証を持ちます。新しい Native Relation ではありません。
独自 World を使う場合は `--world PATH` を指定し、同じ envelope を宣言してください。外部参照の種別と
登録の確認は Host が行います。Host の書き込み先で World や参照ソースを変更してはいけません。

## 登録できる操作

| `kind` | 入力 blob | 成功時の出力 blob |
| --- | --- | --- |
| `read-file` | 空 | 登録した通常ファイルのバイト列 |
| `write-file` | 書き込むバイト列 | `{"bytes_written":N}`。新規作成のみで、既存ファイルは変更しない |
| `http` | GET／HEAD は空、それ以外は body のバイト列 | 2xx 応答の body。非2xx、redirect、timeout は Failed |
| `clock` | 空 | `{"unix_ms":N}` |
| `random` | `{"bytes":16}` のような JSON | 指定したバイト数の OS 乱数 |
| `notify` | 通知するバイト列 | `{"bytes_written":N}`。CLI の通知先は stderr |

HTTP の method は GET、HEAD、POST、PUT、PATCH、DELETE に対応します。登録した URL にだけ要求し、
redirect は追いません。リクエストごとの URL、ファイルパス、method、header、通知先は入力 blob から
指定できません。Host の登録が権限を決めます。読み取り先の通常ファイルに向く symlink は追います。
登録パス自体は Host が管理する信頼された設定であり、ファイルシステム sandbox を提供するものではありません。

## Host 設定

```json
{
  "blobs": {
    "empty": "",
    "message": "Hello\n",
    "random-size": "{\"bytes\":16}"
  },
  "blob_files": { "binary-input": "input.bin" },
  "services": {
    "reader": { "kind": "read-file", "path": "input.bin" },
    "writer": { "kind": "write-file", "path": "new-output.bin" },
    "web": {
      "kind": "http", "method": "POST", "url": "https://example.com/api",
      "headers": {
        "Content-Type": "application/json",
        "Authorization": { "env": "API_TOKEN", "prefix": "Bearer " }
      }
    },
    "clock": { "kind": "clock" },
    "random": { "kind": "random" },
    "notice": { "kind": "notify" }
  }
}
```

ファイルのパスは設定ファイルのディレクトリから解決します。`blobs` は UTF-8 文字列、`blob_files` は
バイナリ入力の初期登録です。header の資格情報は環境変数から取得でき、設定ファイルへ secret を
書く必要はありません。HTTP は既存の proxy と CA trust を使用し、TLS 検証は有効なままです。

## 実行方針と限界

- `EffectHost::execute` は、呼び出し元が選んだ Runtime 所有の Request 一つを処理します。
- `EffectHost::run` は全体をまず純粋に安定させ、トップレベルの Request を左から逐次実行します。
  各出現をその位置で固定応答に置き換えてから、新しい純粋な評価区間へ進みます。
- payload 内の Request は自動で実行しません。Trap や評価上限で NormalForm を得られない区間では、
  その区間の Request を実行しません。
- 同じ Runtime／Term ID の Request は Host の存続中に記録されます。再び渡したときは同じ観測を返し、
  実行しません。同じ handle／blob を持つ別の出現は別の操作です。
- blob の ID は Host 内で一度だけ登録でき、元の内容は変更されません。返された固定応答を
  `Runtime::evaluate_terms` で replay しても、外部処理は実行しません。
- 失敗の blob は `code`、`message`、`effects_may_have_occurred` を持つ JSON です。HTTP timeout や
  部分的な書き込みは、外部で何も起きなかったことを保証しません。自動 retry や rollback はしません。
- Host の上限や後続の純粋評価が失敗しても、それまでの操作は取り消されません。組み込み API の
  `host.observations()` から完了済みの観測を調べられます。

既定上限は blob ごとに1 MiB、保存総量32 MiB、Host の存続中に128 Request、`run` ごとに32評価区間、
HTTP timeout は5秒です。設定の `limits` または `HostLimits` で変更できます。出力保存領域は
実行前に確保できるか確認し、取得するデータと乱数の大きさも制限します。

この記録は永続的な exactly-once 保証ではありません。Host の再起動をまたぐ再試行、並行実行、
cancel、ストリーミング、トランザクション、Agent 接続・履歴保存は引き続き保留です。通知は登録済みの
出力先への書き込みで、メールやメッセージサービスへの接続は含みません。Playground は Request の
生成と遷移を表示し、このネイティブ Host をブラウザから実行しません。
