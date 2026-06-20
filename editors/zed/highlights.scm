(comment) @comment

(attribute) @attribute

(string) @string
(string_content) @string
(escape_sequence) @string.escape

(percentage) @number
(number) @number

(boolean) @boolean
(null) @constant.builtin

(keyword) @keyword
(operator) @operator
(punctuation) @punctuation

; Component, enum and extension names.
(
  (keyword) @_declaration
  .
  (identifier) @type
  (#match? @_declaration "^(component|enum|extension)$")
  )

; Function and recipe names.
(
  (keyword) @_function_keyword
  .
  (identifier) @function
  (#match? @_function_keyword "^(fn|recipe)$")
  )

; State and local binding names.
(
  (keyword) @_binding_keyword
  .
  (identifier) @variable
  (#match? @_binding_keyword "^(state|let|mut)$")
  )

; Identifiers beginning with an uppercase letter are treated as types.
(
  (identifier) @type
  (#match? @type "^[A-Z]")
  )

; An identifier immediately followed by "(" is a function call.
(
  (identifier) @function
  .
  (punctuation) @_open_paren
  (#eq? @_open_paren "(")
  )

; Dot-prefixed identifiers such as ".accent".
(
  (punctuation) @_dot
  .
  (identifier) @variant
  (#eq? @_dot ".")
  )