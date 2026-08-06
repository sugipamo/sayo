# Playgroundのデプロイ

[English](deployment.md) | 日本語

Sayo Playgroundは`sayo-lang` Cloudflare Pagesプロジェクトを通じて
<https://sayo-lang.pages.dev/>へ公開しています。

## Cloudflare Pagesの設定

現在のデプロイにはWrangler Direct Uploadを使用します。対応するビルド設定は次のとおりです。

| 設定 | 値 |
|---|---|
| Framework preset | None |
| Root directory | リポジトリルート |
| Build command | `npm run build:pages` |
| Build output directory | `playground` |
| Environment variables | なし |

production branchは`main`です。Playgroundのパスを変えずに、後から独自ドメインを接続できます。

ビルドコマンドは、Cargoがない環境では最小構成の最新stable Rustを導入し、
`wasm32-unknown-unknown` targetとリポジトリが要求する正確な`wasm-bindgen-cli`を準備して、通常のWasm
ビルドへ処理を渡します。生成bindingはgit管理外の`playground/pkg/`に置かれ、commitせずにPagesの
出力へ含まれます。

## 公開と確認

Pages用ビルドをローカルで実行します。

```sh
npm run build:pages
```

続けてブラウザテストを実行します。

```sh
npm run test:browser
```

現在のGit commitを紐づけて、生成済みPlaygroundをデプロイします。

```sh
npx wrangler pages deploy playground --project-name sayo-lang --branch main --commit-hash "$(git rev-parse HEAD)"
```

アップロード完了後、<https://sayo-lang.pages.dev/>を確認します。

## デプロイ境界

デプロイは現在、メンテナーが明示的に行います。SayoはPages Functions、Workers、サーバー側状態、secret、
アプリケーション用のビルド時設定を必要としません。共有するプログラムはURLのfragmentに残り、
アプリケーションデータとしてCloudflareへ送信されません。自動デプロイが必要になれば、Direct Uploadを
Git連携へ置き換えられます。
