# Komeドキュメント

このディレクトリには、現在のコンパイラ実装に対応するKomeのリファレンスを収録します。
段階的な学習を目的とするチュートリアルではなく、構文や動作を確認するための文書です。

## 言語

- [言語リファレンス](language-reference.md)
- [コンポーネント](components.md)
- [Taskと非同期処理](concurrency.md)
- [C相互運用](interoperability.md)

## ツール

- [`kome`コマンドの役割](kome-yakuwari.md)

## 実装資料

- [ASTノード一覧](ast.md)

## 文書の基準

リファレンスは、ParserだけでなくResolver、TypeChecker、JIT、AOTまで実装された挙動を
基準にします。構文として解析できても実行基盤が未完成な機能は、対応済みとして扱いません。
