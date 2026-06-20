; Component declarations
(
  (keyword) @_keyword
  .
  (identifier) @name
  (#match? @_keyword "^(component|enum|extension)$")
)

; Function declarations
(
  (keyword) @_keyword
  .
  (identifier) @name
  (#match? @_keyword "^fn$")
)

; Recipe declarations
(
  (keyword) @_keyword
  .
  (identifier) @name
  (#match? @_keyword "^recipe$")
)
