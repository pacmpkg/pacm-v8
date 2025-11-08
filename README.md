# pacm-v8

High-level Rust bindings for the V8 JavaScript engine tailored for the pacm toolkit. The crate exposes a safe, ergonomic wrapper around the C++ shims that ship with this repository so you can embed V8 without having to manage isolates, contexts, and lifetime tracking yourself.

## Features

- Safe and minimal Rust API on top of the V8 C++ embedding interface
- Prebuilt Windows V8 artifacts ready to use out of the box
- Utilities for executing scripts, evaluating expressions, and binding host functions
- Support for injecting numbers, strings, and Rust callbacks into a V8 context
- Example crate demonstrating integration in a pacm project

## Getting Started

### Prerequisites

- Rust 1.85 or newer (`rustup update` to stay current)
- Windows 10+ with the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Python 3.11+ if you plan to rebuild V8 using the helper scripts

The repository already includes prebuilt V8 binaries via the releases tab. If you only need to consume the crate, no additional setup is required.

### Installation

Add the crate to your project with Cargo:

```sh
cargo add pacm-v8
```

Or edit your `Cargo.toml` manually:

```toml
[dependencies]
pacm-v8 = "14.4.78"
```

### Quick Start

The snippet below demonstrates how to spin up an isolate, evaluate some JavaScript, and expose a Rust callback to V8:

```rust
use pacm_v8::{Context, Isolate, JsValue, Result};

fn main() -> Result<()> {
    let isolate = Isolate::new()?;
    let mut ctx = isolate.create_context()?;

    ctx.add_function("double", |args| {
        let value = args.first().map(|v| v.as_str().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);
        Ok(Some(JsValue::from((value * 2.0).to_string())))
    })?;

    ctx.set_global_number("forty_two", 42.0)?;

    let result = ctx.eval("double(forty_two).toString()");
    assert_eq!(result?.as_str(), "84");

    Ok(())
}
```

If you built the crate yourself, the ICU data file is embedded by default. To load an external ICU data file, set the `PACM_V8_ICU_DATA_PATH` environment variable to an absolute path.

### Examples

The `example/` workspace member shows how to wire the bindings into a binary crate. You can run it locally:

```ps1
cd example
cargo run
```

## Developing

- Format the code with `cargo fmt` before submitting patches.
- Run `cargo clippy --all-targets --all-features` to catch common pitfalls.
- To refresh the bundled V8 binaries, execute `python scripts/build_v8.py` and follow the prompts.

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines, local development tips, and release steps.

## Documentation

- API Docs: <https://docs.rs/pacm-v8>
- Repository: <https://github.com/pacmpkg/pacm-v8>

## License

`pacm-v8` is distributed under the terms of the license listed in [LICENSE](LICENSE).

## Support and Security

- For questions, start a discussion or open an issue at [GitHub Issues](https://github.com/pacmpkg/pacm-v8/issues).
- For security disclosures, follow the instructions in [SECURITY.md](SECURITY.md).

## Contributing

We welcome contributions of all sizes. Please review [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.
