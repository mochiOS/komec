# C相互運用

KomeはC ABIを介してOS、runtime、ViewKitなどのnative libraryを利用できます。JITとAOTは同じ
`extern "C"`宣言を使用します。

## External declaration

process内またはlink時に解決されるC symbolは`extern "C"` blockで宣言します。特定のlibraryから
読み込む場合は`from`を付けます。

```kome
extern "C" from "viewkit" {
	fn vk_abi_version() -> u32
}
```

`from`の値はlibrary名またはpathです。単純な名前はJITでは`lib{name}.so`などの候補として探索し、
AOTではlinkerの`-l{name}`へ対応します。pathを含む値はそのlibraryを直接使用します。

探索directoryは`KOME_LIBRARY_PATH`で指定します。`kome`は依存packageのroot、`target/debug`、
`target/release`を解決済み探索先として追加します。AOT binaryには必要なruntime search pathも付加します。

現在サポートするexternal ABI名は`C`だけです。

## C互換型

| Kome型 | Cでの用途 |
| --- | --- |
| `i8`から`i64` | 固定幅符号付き整数 |
| `u8`から`u64` | 固定幅符号なし整数 |
| `isize`, `usize` | pointer幅整数 |
| `f32`, `f64` | 浮動小数点数 |
| `bool` | Kome ABIのboolean表現 |
| `*const T`, `*mut T` | raw pointer |
| opaque external struct | pointerのpointee型 |

`String`と`Number`はKome runtime handleであり、Cの`char *`や整数そのものではありません。
UTF-8 bytesへアクセスするruntime ABIなど、所有権を明示したadapterを使用します。

Komeの通常structはruntime管理value layoutを持ちます。Cのby-value struct ABIとして渡すことは
想定していません。C structを扱う場合はexternal block内でlayoutを宣言するか、opaque structへの
pointerを使用します。

## Opaque struct

fieldを省略したexternal structは、Kome側でlayoutを知らないC型です。pointerを保持してC関数間で
受け渡せます。

```kome
extern "C" from "viewkit" {
	struct VkRuntime
	fn vk_runtime_create(id: u64) -> *mut VkRuntime
	fn vk_runtime_destroy(runtime: *mut VkRuntime) -> i32
}
```

opaque型そのものをKome valueとして構築、copy、field accessすることはできません。

## External functionの制約

- bodyを持ちません。
- generic parameterを持ちません。
- default argumentを持ちません。
- parameterと戻り値はcode generation可能なconcrete型でなければなりません。
- C側のsymbol名はKomeのfunction名と同じです。

Kome compilerはC headerを解析しません。宣言と実際のABIが一致する責任はlibrary側にあります。
不一致はmemory破壊を起こし得ます。

## `@native`

`@native("symbol")`はKome runtimeのnative registryを呼び出す関数を宣言します。これはC dynamic
libraryを直接呼ぶ`extern "C"`とは異なります。

```kome
@native("io.sleep")
fn sleep(milliseconds: Number) -> Null
```

`@native`は標準runtimeとの統合向けです。registryはsymbol名からhost functionを検索し、Kome ABIの
tagged valueを渡します。利用可能なsymbolと型はruntime実装に依存します。通常の外部libraryでは
`extern "C" from "library"`を使用します。

## `@runtime`

`@runtime("kind")`はopaqueなKome型を組み込みruntime representationへ対応付けます。現在のcompilerが
認識するruntime kindだけを指定できます。標準ライブラリの`Socket`などに使用します。

このattributeはruntimeとcompilerの内部ABIです。一般packageが独自のruntime kindを追加する仕組みでは
ありません。ViewKitのhandleはC opaque structとpointerで表現します。

## Ownership contract

C関数へpointerを渡しても、Kome値の所有権は自動的にはCへ移りません。borrowed pointerの有効期間は
元のKome値より長くできません。

C側がhandleを保存する場合、対応するretain/release ABIを使用するか、library自身のowned handleへ
copyする必要があります。Cから返されたowned pointerは、library契約で定めたdestroy関数を必ず
呼び出します。

Kome compilerは外部C関数のownership annotationをまだ持ちません。FFI wrapperをpackage側にまとめ、
raw pointerをアプリケーションへ露出させない設計を推奨します。

## JITとAOT

JITは実行前に共有libraryをloadし、宣言されたsymbolを登録します。AOTはobject生成後、native runtimeと
external libraryをC linkerでlinkします。どちらも同じlibrary名とABI宣言を使います。

Linuxの共有libraryは通常`.so`です。現在の非同期I/O reactorもLinuxを対象としており、macOS固有の
dynamic library規則やkqueue対応は提供していません。
