# `kome`コマンドの役割

`kome`はKomeプロジェクトの利用者向けフロントエンドです。RustにおけるCargoと
同じ位置付けで、プロジェクト、依存パッケージ、ビルドを管理します。

## `kome`が担当するもの

- `Kome.toml`の読み込みと検証
- 依存関係の解決、取得、バージョン選択
- lockfileの生成と更新
- stdlib、ViewKit、その他パッケージの探索
- Komeソースとnative libraryのパス解決
- target、profile、feature、capabilityの選択
- build scriptやコード生成の実行
- 解決済みbuild planを`komec`へ渡すこと
- `check`、`build`、`run`、`test`などの開発用コマンド

依存関係の名前、バージョン、取得元、`Kome.toml`内の`[module]`や`[lib]`は
`kome`が解決します。`komec`がプロジェクトディレクトリを走査して依存関係を
推測してはいけません。

## `komec`が担当するもの

`komec`は、`kome`から渡された解決済みの入力をコンパイルする低水準コマンドです。

- Parser、AST、Resolver、TypeChecker
- genericsの特殊化
- ownership処理
- Cranelift lowering
- JIT実行
- AOT object生成とlink
- 明示的に渡されたstdlib、package source、native libraryの利用

`komec`は以下を担当しません。

- `Kome.toml`の依存関係解決
- package registryへのアクセス
- Git repositoryの取得
- dependency versionの選択
- lockfileの更新
- ViewKit固有のcodegen

## インストール配置

標準のインストール配置は次の形です。

```text
~/.kome/
├── bin/
│   ├── kome
│   └── komec
├── stdlib/
└── viewkit/
```

`kome`はこの配置と`Kome.toml`を基に入力を解決します。`komec`は実行ファイルから
`../stdlib`を既定のstdlibとして利用できますが、ViewKitを含む通常パッケージの
解決結果は`kome`から受け取ります。

開発中のリポジトリでは、同じ役割を次のディレクトリが持ちます。

```text
vendor/stdlib
vendor/viewkit
```

## ViewKitとの境界

ViewKitのKome APIはViewKit自身の`lib/`で定義します。`kome`がViewKit packageを
解決し、Kome sourceと`libviewkit.so`の場所を`komec`へ渡します。コンパイラには
ViewKitの型、component、関数名を組み込みません。

```text
Kome.toml
  ↓ komeが解決
resolved source / library paths
  ↓ komecへ入力
Parser → TypeChecker → Codegen → JIT / AOT
```
