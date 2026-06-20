(
  (punctuation) @open
  (#eq? @open "(")
  )

(
  (punctuation) @close
  (#eq? @close ")")
  )

(
  (punctuation) @open
  (#eq? @open "{")
  )

(
  (punctuation) @close
  (#eq? @close "}")
  )

(
  (punctuation) @open
  (#eq? @open "[")
  )

(
  (punctuation) @close
  (#eq? @close "]")
  )

(
  (string
    "\"" @open
    "\"" @close)
  (#set! rainbow.exclude)
  )