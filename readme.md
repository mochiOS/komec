# komec

A compilor for Kome language.

Kome is a programming language for [mochiOS](https://github.com/tas0dev/mochiOS).

## Documentation

The language reference, Task model, component system, C interoperability, and
tooling documentation are indexed in [`docs/README.md`](docs/README.md).

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

The Cargo-like `kome` command resolves `Kome.toml` and passes package sources
and native library paths to `komec`. `komec` only discovers the adjacent
standard library itself. See [`docs/kome-yakuwari.md`](docs/kome-yakuwari.md).
