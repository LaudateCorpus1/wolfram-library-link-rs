# wl-library-link

This library offers bindings to Rust code from the Wolfram language.

This library is used for writing Rust programs which can be loaded by the Wolfram language
LibraryLink family of functions, specifically by
[`LibraryFunctionLoad[]`][library-function-load].

Features:

  * Call Rust functions from the Wolfram language.
  * Pass general Wolfram language expressions to and from Rust code.
  * Evaluate Wolfram expressions from Rust code.
  * Check for and respond to Wolfram language aborts while in Rust code.
  * TODO: WSTP bindings

## Usage

First, ensure that your project's `Cargo.toml` is correctly configured. This means:

  * Setting `crate-type = ["cdylib"]`
  * Adding `wl-expr` and `wl-library-link` as dependencies

By setting `crate-type` to `cdylib` we tell `cargo` to build a dynamic library, which
will be loadable using Wolfram [LibraryLink][library-link].

The `wl-expr` and `wl-library-link` dependencies provide, respectively, an `Expr` type,
which is a simple Rust representation of a Wolfram expression, and an API for interacting
with the Wolfram language from Rust.

A correctly configured Cargo.toml looks like:

```toml
### Cargo.toml

[package]
# This can be whatever you like, as long as it's a valid crate identifier.
name = "my-package"
version = "0.1.0"
edition = "2018"

[lib]
crate-type = ["cdylib"]

[dependencies]
wl-expr            = { git = "ssh://github.com/ConnorGray/wl-expr.git" }
wl-library-link    = { git = "ssh://github.com/ConnorGray/wl-library-link.git" }
```

See the [Cargo manifest documentation][cargo-manifest-docs] for a complete description of
the Cargo TOML file.

Next

```rust
// ### main.rs

use wl_expr::Expr;
use wl_library_link::generate_wrapper;

generate_wrapper![GET_HEAD # get_head(e: Expr) -> Expr];

// TODO: #[wl_library_link::wrap(wrapper_name = "GET_HEAD")]
fn get_normal_head(expr: Expr) -> Expr {
    match expr.kind() {
        ExprKind::Normal(normal) => normal.head.clone(),
        ExprKind::Symbol(_) | ExprKind::String(_) | ExprKind::Number(_) => wlexpr! {
            Failure["HeadOfAtomic", <|
                "Message" -> "Expected non-atomic expression"
            |>]
        }
    }
}
```

Finally, build the library by executing the following commands in the terminal:

```shell
$ cargo build
```

[library-link]: https://reference.wolfram.com/language/guide/LibraryLink.html
[library-function-load]: https://reference.wolfram.com/language/ref/LibraryFunctionLoad.html
[cargo-manifest-docs]: https://doc.rust-lang.org/cargo/reference/manifest.html

### Creating a library which is usable from Rust and Wolfram

`crate-type = ["rlib", "cdyib"]`