# Sayo Roadmap

[English](roadmap.md) | 日本語

この一覧は、現在の実装に対して残っている作業だけを管理する。
完了済みの意味論と実装範囲は`language-spec.md`、設計上の背景は`design.md`を正本とする。

## 意味論決定後に実装する候補

- Template局所SpaceとWorld直下Spaceの可視性・shadow規則
- ユーザー定義Spaceへの比較一般化
- Space合成による複合条件（`And`／`Or`／`Not`）
- schemaで利用できる検証Primitiveの拡張
- 内部`Pass{}`を一般Primitiveとして公開する場合の残存・作用・終了規則
- Bootstrap resource limitをCLIから設定または解除する方法

## 安定稼働後に再検討する項目

- REPL
- モジュールとパッケージ
- セルフホストへの具体的な移行
- StateInstanceのGC・永続化・Runtime間共有
- 安定したRust組み込みAPIまたは`cargo install`の明確な需要が生まれた場合の`sayo-lang` crates.io公開

採用しない機能と実装済みの範囲は`language-spec.md`を正本とし、このファイルには重複して記載しない。
