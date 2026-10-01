# komec

A compilor for Kome language.

Kome is a programming language for [mochiOS](https://github.com/tas0dev/mochiOS).

## Library layout

During development, `komec` loads libraries from `vendor/stdlib` and
`vendor/viewkit`.

An installed compiler uses paths relative to its executable:

```text
~/.kome/
├── bin/komec
├── stdlib/
└── viewkit/
```

External packages such as ViewKit are discovered from their `Kome.toml`.
`KOME_STDLIB_PATH` and `KOME_LIBRARY_PATH` remain available as explicit
overrides.
