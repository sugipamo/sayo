# Sayo

[English](README.md) | 日本語

Sayoは、計算をTermの作用と状態遷移として表面化させる実験的な言語です。

```text
Add 5 3
→ Plus{5} 3
→ 8
```

Sayoのコアモデルには、関数、メソッド、Callable objectがありません。順序付きTerm列が現在状態、Relationが
遷移規則であり、SayoVMは左端にある適用可能な遷移を繰り返します。どの遷移も適用できない平衡状態
**NormalForm**へ到達すると評価が終わります。

## なぜSayoか

通常のプログラムも、コードが規則を与えて内部状態が変化する力学系です。Sayoはその過程を言語の表面へ
持ち出します。

- **遷移そのものが言語です。** 計算はRelationを通じたTermの変化として表現されます。
- **状態が明示されます。** `Plus{5}`のような状態Termが、closureやobjectを導入せずにTermを保持します。
- **結果は残存構造です。** 隠れたreturn channelではなく、平衡時に残った順序付きTerm列が結果です。
- **コアは純粋です。** Hostが初期TermとWorld snapshotを渡し、得られたNormalFormを観測します。
- **実行を見られます。** SayoVMが構造化された可視化frameを生成するため、ブラウザ側で意味論を再構築せず
  reductionをアニメーションにできます。

SayoVMはRustで実装されています。基礎的な操作にはNative実装を利用でき、通常の状態機械は埋め込みSayo
Worldに置かれます。どちらも同じ観測可能な意味論に従います。

## 小さな例

`For`は変換Termを受け取り、後続する各Termへ作用させます。

```text
For (Add 9) 1 2 3
→ Each{Plus{9}} 1 2 3
→ 10 Each{Plus{9}} 2 3
→ 10 11 Each{Plus{9}} 3
→ 10 11 12 Each{Plus{9}}
```

最後に残る`Each{Plus{9}}`は破棄されません。外側の列から後でTermが与えられれば、さらに受け取れる開いた
継続状態です。

## 現在の状態

Sayoはversion **0.1.0**です。現在の実装には次が含まれます。

- Rust CLIと組み込み可能なSayoVM
- `.sayo`ソースとディレクトリで表すWorld
- Space、Relation、Template、構造的Capture、Tick、End、状態payload
- Native算術と比較
- 複製、変換、Filter、Fold、Reduceの標準Sayo状態機械
- World検証とversion付きReduction／Visualization IR
- 評価をアニメーション表示するWebAssembly Playground

言語は意図的に小さく、まだ実験段階です。REPL、module、package、具体的なセルフホスト手順は保留しています。
Sayo 0.1.0はcrates.ioではなく、GitHubとPlaygroundを通じて配布します。

## 試す

最新のstable Rust toolchainを使い、このリポジトリからビルドして実行します。

```sh
cargo run -- --world examples/native_arithmetic/world add 5 3
```

```text
8
```

リポジトリをpublicにした後は、GitHubから直接コマンドをインストールできます。

```sh
cargo install --git https://github.com/sugipamo/sayo
```

公開Playgroundは後日の公開作業でCloudflare Pagesへ配置します。

## ドキュメント

- [Getting Started](docs/getting-started.ja.md) — ビルド、CLI、World、WebAssembly、テスト
- [Language Specification](docs/language-spec.ja.md) — 言語意味論
- [Design Notes](docs/design.ja.md) — Host境界、Native実行、状態設計
- [Sayo Reduction IR](docs/sri-json.ja.md) — 内部Reducer event契約
- [Sayo Visualization IR](docs/svi-json.ja.md) — 完全なVisualizer frame契約
- [Roadmap](docs/roadmap.ja.md) — 意図的に保留している作業

ドキュメント一覧は[`docs/README.ja.md`](docs/README.ja.md)にあります。

## Contribution

Issue、意味論の提案、実装、文書改善を歓迎します。英語の[CONTRIBUTING.md](CONTRIBUTING.md)と
[Code of Conduct](CODE_OF_CONDUCT.md)を参照してください。

## License

Sayoは[MIT License](LICENSE)で配布します。
