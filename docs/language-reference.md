# Kome言語リファレンス

Komeは、mochiOSのアプリケーションとシステムソフトウェアを記述する静的型付き言語です。
値型、ジェネリクス、静的trait dispatch、所有権管理、協調的Taskを共通の型システムで扱います。

この文書は現在の言語仕様を説明します。プロジェクトの作成手順や学習用の題材は扱いません。

## ソースファイル

Komeソースの拡張子は`.kome`です。ソースはUTF-8として読み込まれます。

空白、タブ、改行はトークン間で無視されます。文末のセミコロンは使用しません。複数の文は
通常、別の行に記述します。ブロック内では文の間にカンマを置くこともできます。

行コメントは`//`から行末までです。現在、ブロックコメントはありません。

識別子の先頭にはUnicodeの英字または`_`を使用できます。2文字目以降には数字も使用できます。
識別子は大文字と小文字を区別します。

## キーワード

次の単語はキーワードです。

| 分類 | キーワード |
| --- | --- |
| 宣言 | `component`, `struct`, `trait`, `enum`, `fn`, `recipe`, `state`, `let`, `var`, `const`, `use`, `extern`, `from` |
| 制御 | `if`, `else`, `while`, `for`, `in`, `return`, `break`, `continue`, `is` |
| 非同期 | `task`, `wait`, `cancel` |
| 値 | `true`, `false`, `null` |
| パスと型 | `self`, `super`, `mut` |

## リテラル

### 数値

整数と小数を10進表記で記述します。符号はリテラルの一部ではありません。現在、単項マイナス、
指数表記、16進表記はありません。

型の文脈がない数値は`Number`になります。`i32`や`f64`などの固定幅型が要求される場所では、
表現可能なリテラルをその型として扱います。異なる数値型の暗黙変換は行いません。

`Number`はruntime管理される正確な10進数です。整数の大きさを固定幅に制限しません。除算で
循環小数になる場合は小数部18桁で打ち切ります。

数値の直後に`%`を付けるとpercentリテラルになります。現在の値は百分率へ変換されず、
`50%`は`Number`の`50`として扱われます。

### 文字列

文字列は二重引用符で囲みます。使用できるescapeは`\"`、`\\`、`\n`、`\r`、`\t`、
`\0`、`\{`、`\}`です。文字列リテラルを複数行にはできません。

文字列内の`{expression}`は補間です。`String`、数値、`bool`、`Null`、enumを文字列表現へ
変換できます。struct、list、Taskは直接補間できません。

```kome
let message = "Hello, {name}"
```

### 真偽値とNull

真偽値は`true`と`false`で、型は`bool`です。値がないことを表す`null`の型は`Null`です。

## 型

### 組み込み型

| 型 | 意味 |
| --- | --- |
| `Number` | 任意精度のruntime管理10進数 |
| `String` | UTF-8のruntime管理文字列 |
| `bool` | 真偽値 |
| `i8`, `i16`, `i32`, `i64` | 固定幅符号付き整数 |
| `u8`, `u16`, `u32`, `u64` | 固定幅符号なし整数 |
| `isize`, `usize` | targetのpointer幅を持つ整数 |
| `f32`, `f64` | IEEE 754浮動小数点数 |
| `Null` | 値を持たない型 |
| `T[]` | 要素型`T`のlist |
| `T?` | `T`または`null` |
| `Task<T>` | 非同期に完了する`T` |
| `*const T`, `*mut T` | C相互運用用pointer |

関数が戻り値型を省略した場合、戻り値は内部的な`Void`です。`Void`は通常の型注釈として
記述する型ではありません。値のない式には`Null`を使用します。

### Optional型

`T?`は`T`または`null`を保持します。`null`はoptional型とのみ互換になります。
後置`!`は値が存在することを確認して`T`を取り出します。値が`null`の場合は、明確な実行時
エラーになります。`!`による取り出しは所有権を引き継ぐため、実行時管理値でも二重解放されません。

標準ライブラリの`Result<T, E>`はoptional値を内部表現に使います。`isOk()`と`isErr()`で状態を
確認し、`value()`または`error()`で対応する値を取得できます。`valueOr(fallback)`は成功値が
なければ指定された代替値を返します。

Komeは回復可能な失敗のための例外機構を持ちません。入出力や通信などで想定される失敗は
`Result<T, E>`として返し、通常の値と同じ制御構文で処理します。誤った`value()`または`error()`の
呼び出しなど、処理を継続できない状態は実行時エラーになります。

### List型

`T[]`は同じ型の値を順序付きで保持します。indexは`Number`で指定します。list literalの
要素型は文脈または最初の値から決まります。

空欄の要素は、その型の既定値で初期化されます。

```kome
let names: String[] = [, "Kome", ,]
names[0] = "Mochi"
```

範囲外のindexはruntime errorです。listは`for ... in`で反復できます。

### Pointer型

`*const T`は読み取り対象、`*mut T`は変更可能な対象を指すC互換pointerです。pointerは
主に`extern "C"`宣言で使用します。Komeにはpointer演算やdereference構文はありません。

## Binding

`let`は変更できないbinding、`var`は変更可能なbindingです。`const`は変更できない名前を
宣言し、top levelまたは型実装内で使用できます。

```kome
let title = "Kome"
var count: Number = 0
count += 1
const MAXIMUM: i32 = 100
```

`let`にはinitializerが必要です。`var`は型注釈またはinitializerのいずれかが必要です。
initializerを省略した`var`は型の既定値を持ちます。初期化されていない値の読み取りは
コンパイル時に拒否されます。

bindingはlexical scopeを持ち、内側のscopeでは外側と同じ名前をshadowできます。
変更可能なbinding以外への代入はエラーです。

top-levelの`let`、`var`、`const`はglobal bindingです。globalのinitializerはプログラム開始時に
評価されます。

## 関数

関数は`fn`で宣言します。通常のparameterには型注釈が必要です。型実装の`self`だけは対象型が
自動的に設定されます。戻り値型を省略した関数から値を返すことはできません。

```kome
fn distance(x: Number, y: Number = 0) -> Number {
	return x + y
}
```

parameterには既定値を指定できます。既定値を持つparameterを省略して呼び出せます。
呼び出しでは位置引数と名前付き引数を使用できます。名前付き引数は宣言上のparameter名へ
対応し、最終的な評価順はparameter順です。

関数は宣言より前から参照できます。top-level関数同士の前方参照をサポートします。

Komeプログラムの通常のentry pointは、引数を持たない`fn main()`です。`main`がない場合、
引数を要求しない`@application` componentからentry pointを生成できます。

## Struct

`struct`は名前付きの値型です。fieldの配置順は宣言順で固定されます。構築時にはfield名と値を
指定します。

```kome
struct Point {
	x: Number,
	y: Number,
}

let point = Point { x: 10, y: 20 }
```

fieldは`point.x`で読み取ります。変更可能なstruct bindingに対しては`point.x = value`と
`point.x += value`を使用できます。field lookupはreceiverのstruct型を基準に行われます。

fieldを持たない`struct Name`はopaque struct宣言です。通常のKome値として構築できず、
`@runtime`または`extern "C"`と組み合わせて使用します。

### Structural object literal

`{ name: value }`はobject literalです。期待されるstruct型が明確な場所では、そのstruct値を
構築できます。通常は型名を含むstruct構築を推奨します。文字列、数値、計算式もobjectのkeyに
できますが、code generationで使用できるのは静的に解決可能な構造です。

## Enum

`enum`はcaseの集合です。caseには任意のraw valueを付けられます。enum値は`Color.blue`、
期待型が明確な場所では`.blue`と記述できます。

```kome
enum Status {
	ready,
	failed = "failed",
}
```

同じenumの値は`==`と`!=`で比較できます。`is`によるcase matchingにも使用できます。
enumは現在generic parameterを持ちません。

## 型実装

`for Type`は既存型へmethodとassociated constantを追加します。最初のparameterが`self`なら
instance method、それ以外はstatic methodです。

```kome
for Point {
	const ZERO: Point = Point { x: 0, y: 0 }
	fn length(self) -> Number { return self.x + self.y }
	fn make(x: Number, y: Number) -> Point { return Point { x: x, y: y } }
}
```

instance methodは`point.length()`、static methodは`Point.make(...)`、associated constantは
`Point.ZERO`で参照します。dispatchはreceiver型からコンパイル時に決まり、動的lookupは
行いません。

## Trait

`trait`は型が提供すべきmethod signatureを宣言します。`for Type: Trait`がconformanceです。

```kome
trait Value {
	fn value(self) -> Number
}

for Point: Value {
	fn value(self) -> Number { return self.x + self.y }
}
```

コンパイラはmethod名、parameter数、parameter型、戻り値型を検証します。trait methodも
静的dispatchです。trait object、`dyn Trait`、vtable、runtime trait lookupはありません。

## Generics

struct、関数、traitはgeneric parameterを宣言できます。generic実装では、対象型に現れる
generic parameterが実装scopeへ導入されます。

```kome
struct Container<T> { value: T }
fn identity<T>(value: T) -> T { return value }
for Container<T> {
	fn get(self) -> T { return self.value }
}
```

型引数は`Container<Number>`のように指定します。generic関数では`identity<Number>(42)`と
明示するか、引数から推論できます。複数のparameterも引数ごとに推論されます。同じ型parameterへ
矛盾する型が推論された場合はコンパイルエラーです。

genericコードはconcrete型ごとにmonomorphizeされます。同じ宣言と型引数の組み合わせは
一度だけ生成されます。型消去やruntime generic dispatchは行いません。

直接またはgeneric substitution後に無限の大きさになる再帰value layoutは使用できません。
pointerを介さない再帰structはコンパイルエラーです。

## Closure

closureは`|parameter| expression`で記述します。parameterへ型注釈を付けられます。closureは
外側のbindingをcaptureでき、localまたはglobal bindingへ保存できます。

```kome
let add = |value: Number| value + offset
let result = add(2)
```

空parameterのclosureは現在記述できません。closure本体は1つのexpressionです。複数の処理が
必要な場合はblock expressionを使用します。

## 演算子

次の表は結合の強い順です。

| 優先順位 | 演算子 | 結合 |
| --- | --- | --- |
| 1 | call `()`, member `.`, index `[]`, generic specialization | 左 |
| 2 | `!`, `task`, `wait`, `cancel` | 右 |
| 3 | `*`, `/` | 左 |
| 4 | `+`, `-` | 左 |
| 5 | `<`, `<=`, `>`, `>=` | 左 |
| 6 | `==`, `!=` | 左 |
| 7 | `&&` | 左 |
| 8 | `||` | 左 |
| 9 | `=`, `+=` | 右 |

算術演算は同じ数値型同士に限ります。`String + String`は連結です。`&&`と`||`はshort-circuit
します。比較は`bool`を返します。

代入先には変更可能なbinding、変更可能なstruct field、list elementを指定できます。

## 制御フロー

### 条件分岐

`if`の条件は`bool`でなければなりません。`else if`と`else`を使用できます。現在、`if`は
statementであり、値を返すexpressionではありません。

### Loop

`while condition`は条件がtrueの間、blockを繰り返します。`for item in list`はlistの要素を
宣言順に処理します。`break`と`continue`は最も内側のloopを対象にします。label付きloopは
まだありません。

### Return

`return expression`は現在の関数から値を返します。戻り値のない関数では`return`だけを
使用できます。scopeを離れる際、所有しているruntime管理値は自動的に解放されます。

### `is` matching

`is value pattern => body`は1つのpatternを照合します。patternにはliteral、identifier、
`.case`を使用できます。bodyはblockまたは単一statementです。inline形式はmatching時に
expressionを評価します。

現在の`is`はsingle-armです。網羅的な`match`、guard、複数armはありません。

## Block expression

brace内の最後のexpressionはblock全体の値になります。最後がstatementの場合、blockは値を
返しません。object literalと曖昧になる場合は、propertyの`:`の有無で判定されます。

## Import

`use`はパッケージやモジュールへの参照を宣言します。パスの区切りは`::`、値のメンバー参照は
`.`です。`use std::io`の後は`io::println(...)`、`use std::io::println`の後は
`println(...)`と記述します。`use std::io as console`による別名と、`use std::io::*`による
公開宣言の一括取り込みも使用できます。一括取り込みで同名の宣言が生じた場合はエラーです。
`pub use`は取り込んだ宣言を別パッケージへ再公開します。別名と一括再公開も使用できます。
元の宣言より広い公開範囲で再公開することはできません。

宣言の完全な名前は、利用側の別名に左右されません。たとえば`io::println`と
`console::println`は、どちらも`std::io::println`を参照します。異なるパッケージにある同名宣言は
別の宣言として扱われます。モジュール間の循環参照はエラーです。

パッケージの場所とソースファイルは`kome`が解決します。`komec`は
`--package-source <package> <source>`で解決済みソースを受け取り、パッケージの登録先や
`Kome.toml`を探索しません。

## 公開範囲

修飾子のない宣言は、そのモジュール内だけで参照できます。`pub(package)`は同じパッケージ内、
`pub`は依存する別パッケージからも参照できます。この規則はトップレベル宣言、構造体の
フィールド、固有実装の関数、関連定数に適用されます。traitに宣言した関数は公開されます。

公開フィールドを非公開の構造体に置くことや、公開関数の引数または戻り値に、それより狭い
公開範囲の型を使うことはできません。構造体の生成でもフィールド参照でも、利用位置から見える
フィールドだけを使用できます。

```kome
pub struct User {
	pub name: String,
	id: Number,
}

for User {
	pub fn name(self) -> String {
		return self.name
	}
}
```

## Attribute

attributeは宣言の直前に`@name`または`@name(arguments)`として付けます。現在code generationが
意味を認識する主なattributeは次のとおりです。

| Attribute | 対象 | 意味 |
| --- | --- | --- |
| `@application` | component | `main`がない場合のapplication entry point |
| `@body` | component binding | applicationのview body |
| `@startup` | recipe | component生成時に実行するrecipe |
| `@native("symbol")` | function | Kome runtime registryのnative関数 |
| `@runtime("kind")` | opaque struct | 組み込みruntime representationとの対応 |

未知のattributeをParserが保持する場合でも、runtime上の効果はありません。

## 所有権

Komeのsourceには明示的なretain、release、borrow構文はありません。値のcopy、引数渡し、return、
field保存、Task結果の保持に必要な所有権操作をコンパイラが生成します。

`String`、`Number`、list、Task、runtime-backed型、runtime管理fieldを含むstructは参照管理の
対象です。値を複数箇所から使用しても、それぞれの生存期間が終了するまで内部storageを保持します。
最後の使用では不要なretainを省けますが、これはobservableな言語仕様ではありません。

scope終了、早期return、代入による置換、未取得Task結果、cancelされたTaskの結果でも、所有する
値を一度だけ解放します。利用者が手動で解放する必要はありません。C FFIへ所有権を渡す場合だけ、
ABI契約を明示する必要があります。

## 現在の制限

- trait objectとdynamic dispatchはありません。
- generic制約、associated type、const generic、default type argumentはありません。
- operator overloadはありません。
- exception処理とruntime errorをcatchする構文はありません。
- pointer演算とKome側のdereferenceはありません。
