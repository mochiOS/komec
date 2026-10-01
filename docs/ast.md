# Kome AST

この文書はコンパイラ実装者向けのAST一覧です。source-levelの仕様は
[言語リファレンス](language-reference.md)を参照してください。

## Module

`Module`は1つのsource入力を表し、top-level `Declaration`を宣言順に保持します。すべてのnodeは
元sourceのUTF-8 byte offsetによる`Span`を持ちます。

## Declaration

| Variant | Source construct |
| --- | --- |
| `Component` | `component` |
| `Function` | `fn` |
| `Struct` | `struct`またはopaque struct |
| `Trait` | `trait` |
| `For` | inherentまたはtrait implementation |
| `Let` | top-level `let`または`var` |
| `Constant` | top-level `const` |
| `Use` | `use` |
| `Enum` | `enum` |
| `Extern` | `extern "C"` block |

`ForDeclaration`はtarget型、optional trait型、method、associated constantを保持します。
`ExternDeclaration`はABI、optional library名、external struct/functionを保持します。

## Statement

| Variant | Source construct |
| --- | --- |
| `Block` | statement block |
| `Expression` | valueを破棄するexpression |
| `Let` | local `let`, `var`, `const` |
| `If` | `if` / `else` |
| `While` | `while` |
| `ForIn` | list iteration |
| `Return` | `return` |
| `Break` | `break` |
| `Continue` | `continue` |
| `Is` | single-arm pattern matching |
| `Declaration` | block内declaration用の内部variant |

## Expression

| Variant | Source construct |
| --- | --- |
| `Literal` | number、string、boolean、null、percent |
| `Ident` | identifier |
| `Unary` | `!` |
| `Task` | `task expression` |
| `Wait` | `wait expression` |
| `Cancel` | `cancel expression` |
| `Binary` | arithmetic、comparison、logical operation |
| `Call` | function、method、static call |
| `Member` | member access |
| `Index` | index access |
| `Assign` | `=`または`+=` |
| `Group` | parenthesized expression |
| `Block` | tail valueを持てるblock expression |
| `List` | homogeneous list literal |
| `Object` | structural object literal |
| `Struct` | named struct construction |
| `Template` | interpolated string |
| `Closure` | closure literal |
| `DotIdent` | context-dependent enum case |
| `Is` | inline single-arm matching |
| `Component` | component expressionとchildren |

## Type

| Variant | 意味 |
| --- | --- |
| `Primitive` | source-level primitive type |
| `Named` | user type、type parameter、applied generic type |
| `List` | `T[]` |
| `Optional` | `T?` |
| `Pointer` | `*const T`または`*mut T` |
| `Function` | function type用AST表現 |
| `Object` | structural object type用AST表現 |

generic parameter declarationは`GenericParameter`として独立して保持します。意味解析後は
`SemanticType::TypeParameter`となり、concrete named typeとは区別されます。

## Pattern

通常の`Pattern`はliteralとidentifierを表します。`IsPattern`はそれらにdot-prefixed enum caseを
加えます。現在のParserがbindingとparameterで生成するpatternは主にidentifierです。

## Compiler pipeline

ASTはResolver、InitializationChecker、TypeCheckerを通過した後、generic monomorphizationとmodule
analysisへ渡されます。Codegenは意味解析済みの宣言、concrete type layout、静的dispatch結果を使用し、
source textから型を再推測しません。
