# Kome Language Specification Notes

この文書はKome言語の現在の仕様を整理するためのメモです。

厳密な言語仕様書ではないです

## 1. Kome について

Kome は mochiOS 向けのアプリケーション開発を主目的としたプログラミング言語です。

GUI アプリケーションを簡潔に記述できることを重視しますが、GUI 専用言語にはせず、通常のアプリケーションロジックやシステムプログラミングにも利用できることを目指します。

構文は Rust に近いものを採用します。

ただし、Rust のように「ほぼすべてが式である」といった強い式指向の設計にはしません。

また、Rust の所有権・Clone・Copy などの複雑な概念を、そのまま通常の Kome プログラマに見せることは避けます。

## 2. 型システム

Kome は静的型付け言語です。

以下を提供します。

* 型注釈
* 型検査
* 型推論

例:

```kome
let name: String = "Kome"
let count = 10
```

`count` の型は初期値から推論されます。

## 3. 文

文末のセミコロンは任意です。

以下はどちらも有効です。

```kome
let x = 10
print(x)
```

```kome
let x = 10;
print(x);
```

## 4. 変数

通常の変数には `let` と `var` を使用します。

### let

`let` は不変です。

```kome
let name = "Kome"
```

`let` は必ず宣言時に初期化する必要があります。

以下は無効です。

```kome
let name: String
```

以下は有効です。

```kome
let name: String = "Kome"
```

### var

`var` は可変です。

```kome
var count = 0

count += 1
```

`var` は型注釈がある場合、未初期化宣言を許可します。

```kome
var value: String
```

未初期化状態の値を読み取ることはできません。

## 5. const

コンパイル時定数には `const` を使用します。

```kome
const PI: Number = 3.14159
```

詳細な compile-time evaluation の範囲は未決定です。

## 6. 数値型

通常のアプリケーション開発では `Number` を使用します。

```kome
let a = 10
let b = 3.14
```

どちらも `Number` です。

システムプログラミング向けには固定幅数値型も用意します。

```text
i8
i16
i32
i64

u8
u16
u32
u64

f32
f64
```

`Number` と固定幅数値型の間では暗黙変換を行いません。

```kome
let x: i32 = 10
let y: Number = x
```

このようなコードは型エラーになります。

明示的な変換を必要とします。

変換構文の詳細は未決定です。

## 7. String

文字列型は `String` 一種類です。

Rust の `String` と `&str` のような区別は、Kome の通常のコードには露出させません。

この方針は初期の型設計でも明示されています。

```kome
let message: String = "Hello"
```

## 8. bool

真偽値型は `bool` です。

```kome
let enabled: bool = true
```

## 9. null と Optional

値が存在しないことを表すために `null` を使用します。

Optional 型は `T?` と書きます。

```kome
let name: String? = null
```

`T?` は概念的には、

```text
T | null
```

です。

そのため、

```kome
let name: String = null
```

は型エラーです。

### Optional narrowing

null check の後では型を絞り込みます。

```kome
if name != null {
	print(name)
}
```

このブロック内では `name` を `String` として扱えます。

## 10. struct

名前付き構造体は `struct` で定義します。

```kome
struct User {
	name: String
	age: Number
}
```

インスタンスは以下のように生成します。

```kome
var user = User {
	name: "Alice"
	age: 15
}
```

struct のフィールドは可変です。

```kome
user.age = 16
```

`let` で保持された struct のフィールド変更を許可するかどうかは未決定です。

## 11. Object literal

Object literal は匿名 struct として扱います。

```kome
let user = {
	name: "Alice"
	age: 15
}
```

この値は、概念的には以下のような匿名型を持ちます。

```text
{
	name: String
	age: Number
}
```

同じフィールド名・同じ型を持っていても、異なる object literal から生成された匿名 struct は別型です。

```kome
let a = {
	name: "A"
	age: 10
}

let b = {
	name: "B"
	age: 20
}
```

`a` と `b` は別型です。

再利用可能な同一型として扱いたい場合は、名前付き `struct` を定義します。

## 12. enum

列挙型は `enum` で定義します。

```kome
enum Color {
	red
	green
	blue
}
```

raw value を持たせることもできます。

```kome
enum HttpStatus {
	ok = 200
	notFound = 404
}
```

raw value を持つ場合、すべての case の raw value は同じ型である必要があります。

Rust の enum のような、各 case が異なる associated data を保持する機能は現時点では導入しません。

## 13. Dot Identifier

`.blue` のような構文を利用できます。

```kome
Text("Hello", color: .blue)
```

`.blue` は期待型から関連する値を解決します。

例えば `color` の期待型が `Color` なら、

```text
.blue
```

は概念的に、

```text
Color::blue
```

として解決されます。

現在の Komec でも `DotIdent` として AST に存在します。

## 14. 関数

関数は `fn` で定義します。

```kome
fn greet(name: String) -> String {
	return "Hello, " + name
}
```

デフォルト引数を使用できます。

```kome
fn add(
	a: Number,
	b: Number = 0,
) -> Number {
	return a + b
}
```

デフォルト引数の構文は、

```text
name: Type = default
```

です。

## 15. Component

UI Component は `component` で定義します。

```kome
component Counter() {
	state count = 0
}
```

以前存在した `bundle` 構文は完全に廃止します。仕様議論でも `bundle` から `component` へ移行しています。

Component の外部入力は普通の parameter として定義します。

```kome
component UserCard(
	name: String,
	age: Number,
) {
	...
}
```

以前検討されていた `prop` は使用しません。

## 16. state

`state` は Component 内部の状態です。

```kome
state count = 0
```

`state` には通常の型推論が適用されます。

```kome
state count = 0
state name: String = "Kome"
```

`state` は常に可変です。

```kome
state count = 0

count += 1
```

`state` が変更されると、それに依存する `recipe` が再評価されます。

以前の `state let mut` 形式は廃止し、`state name = value` に簡略化する方針になっています。

## 17. recipe

`recipe` は関数ではありません。

依存する状態が変更された際に再評価される処理単位です。

```kome
recipe counter: count {
	...
}
```

依存関係付きの再評価単位として設計されていることは、初期仕様でも明示されています。

### 依存関係

依存関係は基本的に自動判定します。

例えば、

```kome
recipe counter {
	Text("Count: {count}")
}
```

で `count` を参照している場合、Compiler / Runtime はこの recipe が `count` に依存していることを自動的に判定します。

必要な場合は明示的に依存対象を指定できます。

```kome
recipe counter: count {
	...
}
```

依存対象は同じ構文で記述しますが、対象の型によって意味が変わる場合があります。

## 18. UI の基本モデル

Kome の UI は SwiftUI に近い宣言的 UI とします。

既存 UI の特定要素を命令的に変更するのではなく、

```text
現在の状態なら UI はこうである
```

という形で UI を定義します。

この考え方は初期の仕様議論でも明確に採用されています。

## 19. @body

`@body` は Kome 言語自体が意味を定義するものではありません。

ViewKit が定義する Attribute です。

Komec は通常の Attribute として解析します。

基本形は、

```kome
@body
let view = {
	VStack {
		Text("Hello")
	}
}
```

です。

`@body` が付いた `let` の変数名は任意です。

```kome
@body
let content = {
	...
}
```

も有効です。

`@body` の意味は ViewKit が解釈します。

## 20. UI Component identity

UI の再評価時に、前回の Component と新しい Component をどのように同一視するかは未決定です。

固定された UI tree では構造上の位置などを利用できる可能性があります。

一方で、

```kome
for item in items {
	ItemView(item)
}
```

のような動的 UI では、並び替え・追加・削除に耐えられる安定した identity が必要になります。

`key` や Attribute を使う方式などが候補ですが、具体的な仕様はまだ決定していません。

## 21. is

`is` は pattern matching の簡略構文です。

```kome
is x 1 => foo()
```

概念的には、

```kome
match x {
	1 => foo()
	_ => {}
}
```

に相当します。

`is` は `match` の糖衣構文として設計されています。

## 22. match

通常の `match` 構文も残します。

```kome
match x {
	0 => print("zero")
	1 => print("one")
	_ => print("other")
}
```

`is` と `match` は両方使用できます。

## 23. if

通常の条件分岐も使用できます。

```kome
if count > 10 {
	count = 0
} else {
	count += 1
}
```

## 24. loop

以下の制御構文を使用できます。

```kome
while condition {
	...
}
```

```kome
for item in items {
	...
}
```

```kome
break
continue
```

## 25. closure

Closure を利用できます。

```kome
items.each(|item| print(item))
```

```kome
items.sort(|a, b| a > b)
```

capture semantics の詳細は未決定です。

## 26. List

List literal は、

```kome
let values = [1, 2, 3]
```

のように書きます。

List の型規則、異種型を許可するかどうかは未決定です。

## 27. Template String

文字列中に式を埋め込めます。

```kome
let count = 10
let message = "Count: {count}"
```

## 28. 名前空間と member access

モジュール・型・名前空間の区切りには `::` を使用します。

オブジェクトの field / method access には `.` を使用します。

```kome
File::open("test.txt")
file.read()
```

この区別は仕様議論でも採用されています。

## 29. use

モジュールやパッケージの読み込みには `use` を使用します。

```kome
use std::io
use viewkit::prelude
```

ローカルモジュールと外部パッケージの名前解決方法の詳細はまだ未決定です。

## 30. メモリ・所有モデル

通常の Kome コードでは Rust の Clone / Copy / ownership を直接扱わせない方針です。

過去の仕様案では値を以下のように分類しています。

```text
value
object
resource
unsafe
```

* `value`: 通常のコピー可能な値
* `object`: GC / ARC 等で管理されるオブジェクト
* `resource`: File や Capability など単一所有を必要とするもの
* `unsafe`: raw pointer や MMIO など

通常のアプリケーション開発では `value` と `object` を中心に扱い、システムプログラミング時のみ `resource` や `unsafe` を意識する設計を想定しています。

この分類の詳細はまだ未確定です。
