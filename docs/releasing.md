# Releaseの作成

`scripts/release.pl`は`kome`、`komec`、Kome標準ライブラリの配布archiveを作成します。
Cargoの中間生成物と配布物は別のdirectoryへ出力されます。

## バージョン

リポジトリ直下の`version`がSDK版と各構成要素の内部版を定義します。

```text
kome-sdk=27.0-dp.1
kome=0.1.0
komec=0.1.0
kome-std=0.1.0
```

`kome`と`komec`の値は、対応するCargo packageのversionと一致しなければなりません。不一致の場合、
scriptはbuild前に停止します。配布archiveの名前には`kome-sdk`を使用します。

## Build

repository rootで`scripts/release.pl`を実行します。scriptはlocked dependencyでrelease buildし、
`target/release-build`をCargo build用directoryとして使用します。

必要なcommandは`cargo`、`rustc`、GNU tar、`zstd`、`git`です。checksumはPerl標準moduleの
`Digest::SHA`で生成します。

host以外をbuildする場合はRust targetを指定できます。artifact名のarchitectureはtarget tripleの
先頭要素になります。配布上の名前を変える場合は`--arch`も指定します。

```text
scripts/release.pl --target x86_64-unknown-linux-gnu
scripts/release.pl --target aarch64-unknown-linux-gnu --arch aarch64
```

## Output

配布物はすべて`target/release`直下に生成されます。

| Product | Archive content |
| --- | --- |
| `kome` | release binary `kome`のみ |
| `komec` | `komec`、`kome-lsp`、コンパイル済みAOT runtime archive |
| `kome-std` | `vendor/stdlib`内の`.kome`ファイルのみ |

archive名は`{arch}-{product}-{kome-sdk}.tar.zst`です。3つのarchiveのSHA-256 checksumは
`target/release/SHA256SUMS`へまとめて生成します。

`kome-lsp`と`libkome_native_rt.a`は`komec`と同じdirectoryへinstallします。
`libkome_native_rt.a`は`komec build`が生成したprogramへlinkするnative runtimeです。
それ以外のCargo中間生成物は配布物へ含めません。

`SHA256SUMS`はarchive basenameを記録します。検証時は`target/release`で
`sha256sum -c SHA256SUMS`を実行します。

## Reproducibility

tar entryは名前順に固定し、ownerとgroupを`0`に正規化します。mtimeには`SOURCE_DATE_EPOCH`を使用し、
未指定時は現在のGit commit timestampを使用します。archive metadataは同じ入力に対して一定です。
binary自体の再現性は使用するRust toolchainとnative linkerにも依存します。

## 27.0-dp.1の作成

最初のSDKではLinux x86_64を対象とします。最初にKomeの全テストを実行します。

```text
cargo test --workspace --locked
cargo check --workspace --all-targets
git diff --check
```

配布物を作成して検証します。

```text
perl scripts/release.pl
cd target/release
sha256sum -c SHA256SUMS
```

生成されるファイルは次の4つです。

```text
x86_64-kome-27.0-dp.1.tar.zst
x86_64-komec-27.0-dp.1.tar.zst
x86_64-kome-std-27.0-dp.1.tar.zst
SHA256SUMS
```

`mochiOS/komec`に`27.0-dp.1`タグの下書きReleaseを作り、4ファイルを添付します。

```text
gh release create 27.0-dp.1 \
  target/release/x86_64-kome-27.0-dp.1.tar.zst \
  target/release/x86_64-komec-27.0-dp.1.tar.zst \
  target/release/x86_64-kome-std-27.0-dp.1.tar.zst \
  target/release/SHA256SUMS \
  --draft \
  --prerelease \
  --title "Kome SDK 27.0 Developer Preview 1"
```

ViewKit、AppCore、mochiOS向けツールチェーン、komeupにも同じ`27.0-dp.1`タグを使用します。
すべての下書きへ成果物を添付した後、ViewKit、AppCore、ツールチェーン、Kome、komeupの順で公開します。

公開後は空のインストール先で一連の操作を確認します。

```text
KOME_HOME=/tmp/kome-sdk-27.0-dp.1 komeup install 27.0-dp.1
/tmp/kome-sdk-27.0-dp.1/bin/kome check --manifest-path examples/testapp/Kome.release.toml
/tmp/kome-sdk-27.0-dp.1/bin/kome build \
  --manifest-path examples/testapp/Kome.release.toml \
  --output /tmp/kome-sdk-27.0-dp.1/testapp
```

検証が終わるまで既存の`latest`を参照せず、必ずSDK版を明示します。
