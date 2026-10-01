# Releaseの作成

`scripts/release.pl`は`kome`、`komec`、Kome標準ライブラリの配布archiveを作成します。
Cargoの中間生成物と配布物は別のdirectoryへ出力されます。

## Version

repository rootの`version`が各配布物のversionを定義します。

```text
kome=0.1.0
komec=0.1.0
kome-std=0.1.0
```

`kome`と`komec`の値は、対応するCargo packageのversionと一致しなければなりません。不一致の場合、
scriptはbuild前に停止します。`kome-std`は標準ライブラリ単独のrelease versionです。

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

配布物はproductごとのdirectoryに生成されます。

| Product | Output directory | Archive content |
| --- | --- | --- |
| `kome` | `target/release/kome` | release binary `kome`のみ |
| `komec` | `target/release/komec` | release binary `komec`のみ |
| `kome-std` | `target/release/kome-std` | `vendor/stdlib`内の`.kome`ファイルのみ |

archive名は`{arch}-{product}-{version}.tar.zst`です。同じdirectoryに
`{archive}.sha256`を生成します。

checksum fileはarchive basenameを記録します。検証時はarchiveのdirectoryで`sha256sum -c`を
実行します。

## Reproducibility

tar entryは名前順に固定し、ownerとgroupを`0`に正規化します。mtimeには`SOURCE_DATE_EPOCH`を使用し、
未指定時は現在のGit commit timestampを使用します。archive metadataは同じ入力に対して一定です。
binary自体の再現性は使用するRust toolchainとnative linkerにも依存します。
