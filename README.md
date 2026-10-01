# idn

[![Build status](https://github.com/djc/idn/workflows/CI/badge.svg)](https://github.com/djc/idn/actions?query=workflow%3ACI)
[![codecov](https://codecov.io/gh/djc/idn/branch/main/graph/badge.svg)](https://codecov.io/gh/djc/idn)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

Internationalized Domain Names in Applications (IDNA) for Rust, implementing
[UTS #46](https://www.unicode.org/reports/tr46/) (Nontransitional Processing) with Unicode 18.0.

```rust
let config = idn::Config::new(); // The options used by the WHATWG URL Standard
assert_eq!(config.to_ascii("Bücher.example").unwrap(), "xn--bcher-kva.example");
assert_eq!(config.to_unicode("xn--bcher-kva.example").unwrap(), "bücher.example");

let strict = idn::Config::strict(); // UseSTD3ASCIIRules, CheckHyphens, VerifyDnsLength
assert!(strict.to_ascii("_dmarc.example").is_err());
```

- Performance on par with or better than `idna` 1.1 with ICU4X (see below)
- Passes all UTS #46 conformance tests for Unicode 18.0
- No dependencies; `#![no_std]` (requires `alloc`); `#![forbid(unsafe_code)]`
- Builds from scratch in about 0.3 seconds; 34% smaller binary sizes than idna 1.1

## Comparison

| | `to_ascii` | `to_unicode` | build time (debug / release) | dependencies | binary size | MSRV |
|---|---:|---:|---:|---:|---:|---:|
| `idn` | 75 ns | 63 ns | 0.3 s / 0.4 s | 0 | 105 KiB | 1.81 |
| `idna` 1.1 with the `idn` back end (`idna_adapter` 1.3.0, unreleased) | 102 ns | 93 ns | 0.4 s / 0.7 s | 4 | 120 KiB | 1.81 |
| `idna` 1.1 with `idna_adapter` 1.2.2 (ICU4X 2.3) | 110 ns | 96 ns | 2.7 s / 3.3 s | 28 | 160 KiB | 1.88 |
| `idna` 1.1 with `idna_adapter` 1.1.0 (unicode-rs) | 148 ns | 136 ns | 0.9 s / 1.3 s | 8 | 286 KiB | 1.57 |

Measured on an Apple M4 Max with Rust 1.98:

- `idna` back end: selected by pinning `idna_adapter` in `Cargo.lock`
  (`cargo update -p idna_adapter --precise <version>`). The `idn` back end is the `idn` branch of
  [`djc/idna_adapter`](https://github.com/djc/idna_adapter/tree/idn): with a checkout in
  `../idna_adapter`, pass `--config 'patch.crates-io.idna_adapter.path="../idna_adapter"'` and
  `--config 'patch.crates-io.idn.path="idn"'` to `cargo update -p idna_adapter` and `cargo bench`
- `to_ascii` and `to_unicode`: geometric mean of the time per call over the inputs in the
  benchmark below, with the options used by the WHATWG URL Standard
- Build time: clean build of the crate and its dependencies (`cargo build -p <crate>`)
- Dependencies: all crates the crate depends on directly or indirectly, including proc macros
- Binary size: increase in the total size of the sections of a stripped release binary with LTO
  and a single codegen unit that converts its arguments with `to_ascii` and `to_unicode`
  (`domain_to_ascii` and `domain_to_unicode` for `idna`), compared to one that only prints them
- MSRV: the highest `rust-version` declared by the crate and its dependencies

## Updating Unicode data

Replace the files in `idn/data/` with those for the new Unicode version from
`https://www.unicode.org/Public/<version>/` (`idna/` and `ucd/`, including `ucd/extracted/`), then
run `cargo test --release`. The `codegen` test regenerates `idn/src/tables.rs` and fails if it changed,
so run the tests again to check the new data.
