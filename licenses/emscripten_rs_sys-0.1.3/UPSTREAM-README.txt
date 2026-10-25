# emscripten-rs-sys

[![Crates.io](https://img.shields.io/crates/v/emscripten-rs-sys.svg)](https://crates.io/crates/emscripten-rs-sys)
[![Docs.rs](https://img.shields.io/docsrs/emscripten_rs_sys)](https://img.shields.io/docsrs/emscripten_rs_sys)
![License](https://img.shields.io/crates/l/emscripten-rs-sys.svg)

Low-level Rust FFI bindings to the [Emscripten](https://emscripten.org/) C API, generated using [bindgen](https://github.com/rust-lang/rust-bindgen).

This crate provides raw, unsafe bindings to the C API of Emscripten, allowing Rust code to interface directly with the Emscripten runtime.

⚠️ The only supported target is `wasm32-unknown-emscripten`.

## Prerequisites

You must have the **Emscripten SDK (emsdk)** installed and activated on your system before building.

Follow the official installation instructions here: [Emscripten SDK Installation Guide](https://emscripten.org/docs/getting_started/downloads.html)

After installing, ensure the `emcc` tool is on your PATH, because the create relies on it to locate the SDK.

## Linker setup

Some APIs of Emscripten only work at runtime if they are explicitly enabled at link time.

See the [settings Reference](https://emscripten.org/docs/tools_reference/settings_reference.html) for all `-s` options.

Ways to set linker settings:
- With the RUSTFLAG environment variable: `RUSTFLAGS='-C link-args=-sALLOW_MEMORY_GROWTH=1'`
- In the build script: `println!("cargo:rustc-link-arg=-sALLOW_MEMORY_GROWTH=1");`
- In the `.cargo/config.toml` with the `rustflags` field.

## Highlights

- The complete C API is covered.
- `EM_JS` support for inline JS functions.

## Example

```rust
js! {
    fn compute_sum(n: c_int) -> c_int,
    {
        let sum = 0;
        for(let i=1; i<n; i++)
        {
            sum += i;
        }
        return sum;
    }
}

fn test_sum() {
    assert_eq!(unsafe { compute_sum(100) }, 4950)
}
```
