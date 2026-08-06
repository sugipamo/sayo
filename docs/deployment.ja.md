# Playgroundのデプロイ

[English](deployment.md) | 日本語

Sayo PlaygroundはCloudflare Pagesへ公開できる状態に準備していますが、このリポジトリの設定だけでは
Pagesプロジェクトの作成や公開を行いません。公開はメンテナーが後から行う別作業です。

## Cloudflare Pagesの設定

Cloudflare PagesのGit連携から`https://github.com/sugipamo/sayo`を接続し、次の値を設定します。

| 設定 | 値 |
|---|---|
| Framework preset | None |
| Root directory | リポジトリルート |
| Build command | `npm run build:pages` |
| Build output directory | `playground` |
| Environment variables | なし |

Pagesプロジェクトの作成時に、リポジトリのproduction branchを選択します。最初の公開先には、プロジェクトへ
割り当てられる`*.pages.dev`サブドメインを使用します。Playgroundのパスを変えずに、後から独自ドメインを
接続できます。

ビルドコマンドは、Cargoがない環境では最小構成の最新stable Rustを導入し、
`wasm32-unknown-unknown` targetとリポジトリが要求する正確な`wasm-bindgen-cli`を準備して、通常のWasm
ビルドへ処理を渡します。生成bindingはgit管理外の`playground/pkg/`に置かれ、commitせずにPagesの
出力へ含まれます。

## 公開前の確認

Pages用ビルドをローカルで実行します。

```sh
npm run build:pages
```

続けてブラウザテストを実行します。

```sh
npm run test:browser
```

初回デプロイ後、割り当てられた`pages.dev`のURLを手動確認してからルートREADMEへ追加します。正確な
サブドメインはCloudflare Pagesプロジェクト作成時に決まるため、推測したURLは記載しません。

## デプロイ境界

プロジェクト接続後の自動ビルドとデプロイはGit連携が担当します。現在のSayoはPages Functions、Workers、
サーバー側状態、secret、アプリケーション用のビルド時設定を必要としません。共有するプログラムはURLの
fragmentに残り、アプリケーションデータとしてCloudflareへ送信されません。

Cloudflareプロジェクトの作成、GitHubの認可、production branchの選択、初回デプロイは、公開作業まで
意図的に保留します。
