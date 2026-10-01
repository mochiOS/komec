; Component declarations
(
  (keyword) @_keyword
  .
  (identifier) @name
  (#match? @_keyword "^(component|enum|extension)$")
) @item

; Function declarations
(
  (keyword) @_keyword
  .
  (identifier) @name
  (#match? @_keyword "^fn$")
) @item

; Recipe declarations
(
  (keyword) @_keyword
  .
  (identifier) @name
  (#match? @_keyword "^recipe$")
) @item
