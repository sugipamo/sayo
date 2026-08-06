# Contributing to Sayo

[English](CONTRIBUTING.md) | 日本語

SayoへのIssue、設計提案、実装、ドキュメント改善を歓迎します。

## 始める前に

言語意味論に関わる変更は、実装より先にIssueで目的と状態遷移例を共有してください。現在の仕様は
[`docs/language-spec.ja.md`](docs/language-spec.ja.md)、保留事項は[`docs/roadmap.ja.md`](docs/roadmap.ja.md)にあります。

小さなバグ修正、テスト、誤字修正は直接Pull Requestにして構いません。

## ローカル確認

開発には最新のRust stableを使用してください。現在は固定したMSRVを設けていません。

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Playgroundを変更した場合は、Wasm環境を準備したうえで次も実行してください。

```sh
npm ci
npm run test:browser
```

必要なWasmツールはREADMEのBrowser / WebAssembly節を参照してください。

## Pull Request

- 一つのPull Requestには、できるだけ一つの目的だけを含めてください。
- 公開仕様を変更する場合は、同じPull Requestで仕様書とテストを更新してください。
- Native最適化を追加する場合は、Sayo実装との観測上の同値性をテストしてください。
- 生成物の`target/`、`playground/pkg/`、`node_modules/`はcommitしないでください。

投稿されたContributionは、プロジェクトと同じMIT Licenseで提供されるものとして扱います。
