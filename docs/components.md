# Component

ComponentはmochiOSアプリケーションの宣言的なUIと状態を表す言語要素です。componentのparameter、
local state、計算済みbinding、recipe、functionを1つの宣言にまとめます。

ViewKitの具体的なcomponentと描画処理はKomeコンパイラに組み込みません。ViewKit packageがKome APIと
C ABIを定義し、`kome`がpackage sourceとnative libraryを解決します。

## 宣言

componentは名前とtyped parameterを持ちます。parameterには既定値を指定できます。bodyを省略した
componentは外部packageが実装を提供する宣言として使用できます。

```kome
component Badge(text: String, emphasized: bool = false)
```

component bodyには次のmemberを置けます。

| Member | 用途 |
| --- | --- |
| `state` | componentが所有する変更可能な状態 |
| `let` | 変更しないlocal binding |
| `var` | 変更可能なlocal binding |
| `recipe` | lifecycle、event、view処理 |
| `fn` | component内function |

component parameterとmemberはcomponent instanceのscopeに入ります。functionとrecipeからstateやbindingを
参照できます。

## Application component

`@application`を付けたcomponentはアプリケーションのrootです。sourceに`fn main()`がない場合、
コンパイラがapplication componentを生成する`main`を作ります。

application componentは必須引数を持てません。parameterを持つ場合、すべてに既定値が必要です。
1つのmoduleに複数の`@application` componentは置けません。

## Stateとbody

`state`は変更可能で、initializerまたは型注釈を持ちます。component functionとrecipeから通常の
変更可能bindingとして更新できます。

`@body`を付けたbindingはcomponentのview bodyを表します。block expressionとcomponent expressionを
組み合わせて宣言的なtreeを構築できます。

```kome
@application
component App() {
	state title = "Kome"
	@body let body = {
		VStack {
			Text(title)
		}
	}
}
```

`@body`はbindingの役割を示します。生成されるnative view treeの具体的な意味はViewKit側の定義に
従います。

## Component expression

componentは通常のcallと同じように位置引数または名前付き引数を受け取ります。brace内にはchild
component expressionを並べます。

`VStack()`と`VStack { ... }`は異なります。前者は引数だけを持つcall、後者はchildrenを持つ
component expressionです。引数とchildrenを同時に指定する場合は`VStack(gap: 8) { ... }`とします。

component expressionの生成結果は現在`Null`として扱われます。UI nodeの実体はViewKit runtimeが
管理します。

## Recipe

recipeはcomponent lifecycle、event handler、view構築処理を表します。名前の後ろに`: source`を
付けるとevent sourceを関連付けられます。

`@startup` recipeはcomponent生成時に実行されます。`view`という名前のrecipeもcomponent評価時に
実行されます。

```kome
@startup
recipe initialize {
	count = 1
}

recipe submit: button {
	count += 1
}
```

event sourceの接続方法とevent payloadはcomponent package側の契約です。言語runtimeは現時点で
汎用event busを提供しません。

## Component function

component内の`fn`はcomponent scopeをcaptureします。stateの読み書きとparameterの参照ができます。
呼び出しと既定引数の規則はtop-level関数と同じです。

## Modifier call

component expressionの後ろにはmember callを連結できます。たとえば`Text("Kome").padding(8)`の
ような式です。modifierの解決はreceiver型またはcomponent packageが提供する定義に従います。

## 現在の境界

- UI layout、theme、renderingはViewKitの責務です。
- component名やmodifier名をコンパイラへhard-codeしません。
- state persistenceと外部event配信はruntime/package側の契約です。
- component generic parameterはまだありません。
- component inheritanceはありません。
