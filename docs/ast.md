# Kome AST Nodes

## Expression (`expressions.rs:7`)

| Node | Description |
|---|---|
| `Literal` | Literal value (`42`, `"hello"`, `true`, `null`, `50%`) |
| `Ident` | Identifier (`foo`, `Bar`) |
| `Unary` | Unary operation (`!expr`) |
| `Binary` | Binary operation (`a + b`, `x == y`) |
| `Call` | Function call (`foo()`, `obj.method(x)`) |
| `Member` | Property access (`object.property`) |
| `Index` | Index access (`object[index]`) |
| `Assign` | Assignment (`target = value`, `x += 1`) |
| `Group` | Grouping (`(expr)`) |
| `List` | List literal (`[a, b, c]`) |
| `Object` | Object literal (`{ key: value }`) |
| `Template` | Template string (`"hello {name}"`) |
| `Closure` | Closure (`\|x\| expr`) |
| `DotIdent` | Dot identifier (`.blue`, `.entered`) |
| `Is` | Inline is expression (`is x 1 => "one"`) |
| `Component` | Component call (`VStack { ... }`) |

## Statement (`statements.rs:7`)

| Node | Description |
|---|---|
| `Block` | Block (`{ ... }`) |
| `Expression` | Expression statement (result discarded) |
| `Let` | Let statement |
| `If` | If / else |
| `While` | While loop |
| `ForIn` | For-in loop (`for item in iter`) |
| `Return` | Return |
| `Break` | Break (optional label) |
| `Continue` | Continue (optional label) |
| `Empty` | Empty statement (`;`) |
| `Is` | Is pattern-match statement |
| `Declaration` | Declaration (see below) |

## Declaration (`declarations.rs:7`)

| Node | Description |
|---|---|
| `Component` | Component declaration |
| `Function` | Function declaration |
| `Let` | Let binding (top-level) |
| `Constant` | Const binding (top-level) |
| `Use` | Use import |

## ComponentMember (`declarations.rs:47`)

| Node | Description |
|---|---|
| `State` | State variable |
| `Recipe` | Recipe (event handler / lifecycle) |
| `Attribute` | Attribute (`@application`) |

## Pattern / IsPattern (`patterns.rs:7`, `patterns.rs:14`)

| Node | Description |
|---|---|
| `Literal` (Pattern) | Literal pattern |
| `Ident` (Pattern) | Identifier pattern (`name` or `name: Type`) |
| `DotIdent` (IsPattern only) | Dot identifier pattern (`.entered`) |

## Type (`types.rs:7`)

| Node | Description |
|---|---|
| `Primitive` | Primitive (`String`, `Number`, `Boolean`, `Null`) |
| `Function` | Function type (`(param) => ReturnType`) |
| `List` | List type (`ElementType[]`) |
| `Object` | Object type (`{ key: Type }`) |
| `Named` | Named type (`Name<Arg>`) |

## Sub-nodes

| Node | Definition |
|---|---|
| `CallArg` | Positional / Named |
| `LiteralKind` | String / Number / Boolean / Null / Percent |
| `UnaryOp` | Not |
| `BinaryOp` | Add, Sub, Mul, Div, Eq, NotEq, Lt, Lte, Gt, Gte, And, Or |
| `AssignOp` | Assign / AddAssign |
| `PropertyKey` | Ident / String / Number / Computed |
| `TemplatePart` | String / Expression |
| `UseSpecifier` | Wildcard / Named |
| `Module` | Source file (= `Vec<Declaration>`) |
