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
├── appcore/
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
vendor/devkit/crates/appcore
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

## 現在の`Kome.toml`

アプリケーションは次のように定義します。`[application]`を省略した場合、
`src/main.kome`を使用します。

```toml
[package]
name = "hello"
version = "0.1.0"

[application]
source = "src/main.kome"
```

ライブラリパッケージは`[lib]`で公開するソースを指定します。

```toml
[package]
name = "widgets"
version = "0.1.0"

[lib]
source = "src/lib.kome"
```

初期実装ではローカル依存をサポートします。依存パスはアプリケーションの
`Kome.toml`があるディレクトリを基準に解決します。

```toml
[dependencies]
viewkit = { path = "../viewkit" }
widgets = "vendor/widgets"
```

AppCoreやViewKitのようにKomeと一緒に配布されるpackageはsystem dependencyとして
指定できます。開発時は`vendor/`、インストール後は`kome`と同じprefixから探索します。

```toml
[dependencies]
appcore = { system = true }
```

system dependencyの依存も再帰的に解決されるため、AppCoreからViewKitが自動的に
読み込まれます。

依存パッケージの`[lib].source`は`kome`が解決し、`komec --source`へ明示的に
渡します。native libraryの探索パスも依存ルートから組み立てて渡します。
`komec`自身は`Kome.toml`を読みません。

## コマンド

```text
kome check [--manifest-path path/to/Kome.toml]
kome run [--manifest-path path/to/Kome.toml]
kome build [--manifest-path path/to/Kome.toml] [--output path]
kome test [--manifest-path path/to/Kome.toml]
```

`KOMEC`で使用するコンパイラを明示できます。未指定時は`kome`と同じ
ディレクトリにある`komec`を優先し、その後`PATH`上の`komec`を使用します。

`kome test`は`tests/`直下の`.kome`ファイルを名前順にJIT実行します。各ファイルは
独立した`main`関数を持つintegration testです。
