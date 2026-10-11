# ファイル → HTTP → 新規ファイル

[English](README.md) | 日本語

サンプルファイルを読む、ローカル HTTP サーバーへ POST する、成功した応答本文を新しいファイルへ
保存する例です。既存の逐次 `EffectHost::run` を使い、次の操作と失敗後の終了を Sayo の World に記述します。
本文は `Data<blob:...>` の不透明なバイト列のままで、HTTP status と成功・失敗だけを意味論内に渡します。

リポジトリのルートで実行してください。Rust のビルド環境と Python 3 を使います。

```sh
cargo build --locked --features host-effects --bin sayo-host
python3 examples/file_http_pipeline/run.py
python3 examples/file_http_pipeline/run.py --status 503
python3 examples/file_http_pipeline/run.py --missing-input
python3 examples/file_http_pipeline/run.py --existing-output
```

`run.py` は `127.0.0.1` の空きポートでテストサーバーを起動し、チェックイン済みの `sample.txt` を
新しい作業ディレクトリへコピーして `host.json` を生成します。使用する HTTP 接続先はそのローカル
サーバーだけです。終了時にサーバーを停止し、作業ディレクトリのパスを表示します。
`--output-dir PATH` で保存場所を指定でき、そのディレクトリは新規作成に限ります。
`--host-bin PATH` でビルドした実行ファイルの場所を指定できます。

| 実行 | 最終項 | HTTP 要求数 | 新規保存 |
| --- | --- | --- | --- |
| 既定（201） | `Saved{Data<blob:...>} HttpStatus{201}` | 1 | `response.bin` に応答本文を保存 |
| `--status 503` | `HttpFailed{Data<blob:...>} HttpStatus{503}` | 1 | 行わない |
| `--missing-input` | `ReadFailed{Data<blob:...>}` | 0 | 行わない |
| `--existing-output` | `SaveFailed{Data<blob:...>} HttpStatus{201}` | 1 | 既存ファイルを保持 |

作業ディレクトリには `input.txt`、`host.json`、各観測の `observations/*.blob` が残ります。
HTTP の成功・非2xx応答の blob は本文そのものです。読取・保存・通信・サイズ上限の失敗は診断 JSON の
blob です。`Saved` の blob は `bytes_written` を含む保存結果で、本文は `response.bin` にあります。
操作の失敗は Sayo の結果として扱うため、CLI の終了コードは0です。設定・Host 上限・純粋評価の
失敗は非0です。HTTP の受信後に本文の読取が失敗した場合は、その status と診断 blob が残ります。

## プログラムの流れ

`main.sayo` は読取要求一つと `ReadStage{}` を残します。Host が読取を終えると、World の
`ReadStage/Succeeded/ReadReady` が成功応答を捕捉し、`ReadReady/.tick/.any/main.sayo` が
データを HTTP に渡す次の Request を作ります。読取失敗は `ReadFailed` で終了します。

HTTP 登録の `response_metadata: true` により、Host は次の構造を返します。

```sayo
HttpResponse{HttpStatus{201}, Succeeded{Data<blob:body>}}
HttpResponse{HttpStatus{503}, Failed{Data<blob:body>}}
HttpResponse{NoHttpStatus{}, Failed{Data<blob:diagnostic>}}
```

`HttpReady` は `inspect` を明示的に作用させて、固定された envelope 内の outcome と status を取り出します。
`HandleHttp/Succeeded/HttpOk` だけが保存要求を作り、`HandleHttp/Failed/HttpError` は `HttpFailed` で
終了します。`payload` は envelope 内の `Data` Term を取り出す World の Relation で、blob 本文の解釈では
ありません。status は項列に保持され、保存の成功・失敗は `Saved`／`SaveFailed` に分岐します。

独自のローカル設定を使う場合の CLI 呼び出しは次の形です。パスは設定ファイルから相対解決されます。

```sh
target/debug/sayo-host --config PATH/host.json \
  --world examples/file_http_pipeline/world \
  --export-blobs PATH/observations examples/file_http_pipeline/main.sayo
```

この例は各段階で一つの Request だけを生成します。自動 retry、redirect 追従、並列実行、
ストリーミング、Agent 接続は行いません。HTTP に失敗した後も保存に進む、別の保存先へ送るなどの
振る舞いは、World の失敗 Relation を明示的に変更して記述できます。同じ Host 内の同じ Request を
再実行しても固定観測を返すため、新しい操作を試す場合は別の Request 出現が必要です。

## 検証

```sh
cargo run --bin sayo -- check --world examples/file_http_pipeline/world
cargo test --locked --features host-effects --test host_effects
```

テストはローカル HTTP サーバーと合成バイナリで、成功保存、503／302 の本文保持と保存停止、
読取失敗時の HTTP 未実行、保存先の非上書き、timeout、本文サイズ上限、固定応答の replay を検証します。
Host 全体の契約は [ネイティブ Host の仕様](../../docs/host-effects.ja.md) を参照してください。
