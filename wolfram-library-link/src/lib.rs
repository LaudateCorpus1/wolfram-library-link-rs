//! A safe and convenient wrapper around Wolfram [LibraryLink][library-link-guide].
//!
//! LibraryLink is framework for writing C/Rust programs which can be
//! [loaded][library-function-load] by the Wolfram Language.
//!
//! The primary interface provided by this library is [`#[wolfram_library_function]`][wlf]:
//!
//! ```
//! use wl_expr::Expr;
//! use wolfram_library_link::{wolfram_library_function, WolframEngine};
//!
//! #[wolfram_library_function]
//! pub fn say_hello(engine: &WolframEngine, args: Vec<Expr>) -> Expr {
//!     for arg in args {
//!         engine.evaluate(&Expr! { Print["Hello ", 'arg, "!"] });
//!     }
//!
//!     Expr::null()
//! }
//! ```
//!
//! ## Show backtrace when a panic occurs
//!
//! Functions wrapped using [`wolfram_library_function`][wlf] will automatically catch any
//! Rust panic's which occur in the wrapped code, and return a [`Failure`][failure] object
//! with the panic message and source file/line number. It also can optionally show the
//! backtrace. This is configured by the `"LIBRARY_LINK_RUST_BACKTRACE"` environment
//! variable. Enable it by evaluating:
//!
//! ```wolfram
//! SetEnvironment["LIBRARY_LINK_RUST_BACKTRACE" -> "True"]
//! ```
//!
//! Now the error shown when a panic occurs will include a backtrace.
//!
//! Note that the error message may include more information if the `"nightly"`
//! [feature][cargo-features] of `wolfram-library-link` is enabled.
//!
//! [wlf]: attr.wolfram_library_function.html
//! [library-link-guide]: https://reference.wolfram.com/language/guide/LibraryLink.html
//! [library-function-load]: https://reference.wolfram.com/language/ref/LibraryFunctionLoad.html
//! [failure]: https://reference.wolfram.com/language/ref/Failure.html
//! [cargo-features]: https://doc.rust-lang.org/cargo/reference/features.html

#![cfg_attr(feature = "nightly", feature(panic_info_message))]
#![warn(missing_docs)]

mod args;
mod async_tasks;
/// This module is *semver exempt*. This is not intended to be part of the public API of
/// wolfram-library-link.
///
/// Utility for catching panics, capturing a backtrace, and extracting the panic
/// message.
#[doc(hidden)]
pub mod catch_panic;
mod data_store;
mod library_data;
/// This module is *semver exempt*. This is not intended to be part of the public API of
/// wolfram-library-link.
///
/// Utilities used by code generated by the [`#[wolfram_library_function]`][wlf] macro.
///
/// [wlf]: attr.wolfram_library_function.html
#[doc(hidden)]
pub mod macro_utils;
mod numeric_array;
pub mod rtl;


use wl_expr::{Expr, ExprKind};
use wl_symbol_table as sym;
use wolfram_library_link_sys::{mint, WSLINK};
use wstp::Link;


pub use wolfram_library_link_sys as sys;
pub use wstp;

pub use self::{
    args::{FromArg, IntoArg, NativeFunction},
    async_tasks::{spawn_async_task_with_thread, AsyncTaskObject},
    data_store::DataStore,
    library_data::{get_library_data, initialize, WolframLibraryData},
    numeric_array::{
        NumericArray, NumericArrayDataType, NumericArrayKind, NumericArrayType,
        UninitNumericArray,
    },
};

/// Attribute to generate a [LibraryLink][library-link]-compatible wrapper around a Rust
/// function.
///
/// The wrapper function generated by this macro must be loaded using
/// [`LibraryFunctionLoad`][library-function-load], with [`LinkObject`][link-object] as
/// the argument and return value types.
///
/// A function written like:
///
/// ```
/// use wl_expr::Expr;
/// use wolfram_library_link::{WolframEngine, wolfram_library_function};
///
/// #[wolfram_library_function]
/// pub fn say_hello(engine: &WolframEngine, args: Vec<Expr>) -> Expr {
///     for arg in args {
///         engine.evaluate(&Expr! { Print["Hello ", 'arg] });
///     }
///
///     Expr::null()
/// }
/// ```
///
/// can be loaded in the Wolfram Language by evaluating:
///
/// ```wolfram
/// LibraryFunctionLoad[
///     "/path/to/target/debug/libmy_crate.dylib",
///     "say_hello_wrapper",
///     LinkObject,
///     LinkObject
/// ]
/// ```
///
/// ## Options
///
/// #### Generated wrapper name
///
/// By default, the generated wrapper function will be the name of the function the
/// attribute it applied to with the fragment `_wrapper` appended. For example, the
/// function `say_hello` has a wrapper named `say_hello_wrapper`.
///
/// This can be controlled via the `name` option of `wolfram_library_function`, which sets
/// the name of generated Wolfram library function:
///
/// ```
/// # use wl_expr::Expr;
/// # use wolfram_library_link::{WolframEngine, wolfram_library_function};
/// #
/// #[wolfram_library_function(name = "WL_greet")]
/// pub fn say_hello(engine: &WolframEngine, args: Vec<Expr>) -> Expr {
///     // ...
/// #   Expr::null()
/// }
/// ```
///
/// The `LibraryFunctionLoad` invocation should change to:
///
/// ```wolfram
/// LibraryFunctionLoad[
///     "/path/to/target/debug/libmy_crate.dylib"
///     "WL_greet",
///     LinkObject,
///     LinkObject
/// ]
/// ```
///
///
/// [library-link]: https://reference.wolfram.com/language/guide/LibraryLink.html
/// [library-function-load]: https://reference.wolfram.com/language/ref/LibraryFunctionLoad.html
/// [link-object]: https://reference.wolfram.com/language/ref/LinkObject.html
#[doc(inline)]
pub use wolfram_library_function_macro::wolfram_library_function;

const BACKTRACE_ENV_VAR: &str = "LIBRARY_LINK_RUST_BACKTRACE";

//======================================
// WolframEngine
//======================================

/// Callbacks to the Wolfram Engine.
#[allow(non_snake_case)]
pub struct WolframEngine {
    wl_lib: sys::WolframLibraryData,

    // TODO: Is this function thread safe? Can it be called from a thread other than the
    //       one the LibraryLink wrapper was originally invoked from?
    AbortQ: unsafe extern "C" fn() -> mint,
    getWSLINK: unsafe extern "C" fn(sys::WolframLibraryData) -> WSLINK,
    processWSLINK: unsafe extern "C" fn(WSLINK) -> i32,
}

impl WolframEngine {
    /// Initialize a `WolframEngine` from the callbacks in a [`WolframLibraryData`]
    /// object.
    unsafe fn from_library_data(libdata: sys::WolframLibraryData) -> Self {
        // TODO(!): Use the library version to verify this is still correct?
        // TODO(!): Audit this
        // NOTE: That these fields are even an Option is likely just bindgen being
        //       conservative with function pointers possibly being null.
        // TODO: Investigate making bindgen treat these as non-null fields?
        let lib = *libdata;
        WolframEngine {
            wl_lib: libdata,

            AbortQ: lib.AbortQ.expect("AbortQ callback is NULL"),
            getWSLINK: lib.getWSLINK.expect("getWSLINK callback is NULL"),
            processWSLINK: lib.processWSLINK.expect("processWSLINK callback is NULL"),
        }
    }

    /// Returns `true` if the user has requested that the current evaluation be aborted.
    ///
    /// Programs should finish what they are doing and return control of this thread to
    /// to the kernel as quickly as possible. They should not exit the process or
    /// otherwise terminate execution, simply return up the call stack.
    ///
    /// Within Rust code reached through a `#[wolfram_library_function]` wrapper,
    /// `panic!()` can be used to quickly unwind the call stack to the appropriate place.
    /// Note that this will not work if the current library is built with
    /// `panic = "abort"`. See the [`panic`][panic-option] profile configuration option
    /// for more information.
    ///
    /// [panic-option]: https://doc.rust-lang.org/cargo/reference/profiles.html#panic
    pub fn aborted(&self) -> bool {
        let val: mint = unsafe { (self.AbortQ)() };
        val == 1
    }

    /// Evaluate `expr` by calling back into the Wolfram Kernel.
    ///
    /// TODO: Specify and document what happens if the evaluation of `expr` triggers a
    ///       kernel abort (such as a `Throw[]` in the code).
    pub fn evaluate(&self, expr: &Expr) -> Expr {
        match self.try_evaluate(expr) {
            Ok(returned) => returned,
            Err(msg) => panic!(
                "WolframEngine::evaluate: evaluation of expression failed: \
                {}: \n\texpression: {}",
                msg, expr
            ),
        }
    }

    /// Attempt to evaluate `expr`, returning an error if a WSTP transport error occurred
    /// or evaluation failed.
    pub fn try_evaluate(&self, expr: &Expr) -> Result<Expr, String> {
        let mut link = self.get_wstp_link();

        // Send an EvaluatePacket['expr].
        let _: () = link
            .put_expr(&Expr! { EvaluatePacket['expr] })
            .map_err(|e| e.to_string())?;

        let _: () = self.process_wstp_link(&link)?;

        let return_packet: Expr = link.get_expr().map_err(|e| e.to_string())?;

        let returned_expr = match return_packet.kind() {
            ExprKind::Normal(normal) => {
                debug_assert!(normal.has_head(&*sym::ReturnPacket));
                debug_assert!(normal.contents.len() == 1);
                normal.contents[0].clone()
            },
            _ => return Err(format!(
                "WolframEngine::try_evaluate: returned expression was not ReturnPacket: {}",
                return_packet
            )),
        };

        Ok(returned_expr)
    }

    fn get_wstp_link(&self) -> Link {
        unsafe {
            let unsafe_link = (self.getWSLINK)(self.wl_lib);
            // Go from *mut MLINK -> *mut WSLINK
            Link::unchecked_new(unsafe_link as *mut _)
        }
    }

    fn process_wstp_link(&self, link: &Link) -> Result<(), String> {
        let raw_link = unsafe { link.raw_link() };

        // Process the packet on the link.
        let code: i32 = unsafe { (self.processWSLINK)(raw_link as *mut _) };

        if code == 0 {
            let error_message = link
                .error_message()
                .unwrap_or_else(|| "unknown error occurred on WSTP Link".into());

            return Err(error_message);
        }

        Ok(())
    }
}

/// Export the specified functions as native LibraryLink functions.
///
/// [`NativeFunction`] must be implemented by the functions
/// exported by this macro.
///
/// Functions exported using this macro will automatically:
///
/// * Call [`initialize()`] to initialize this library.
/// * Catch any panics that occur.
///   - If a panic does occur, the function will return
///     [`LIBRARY_FUNCTION_ERROR`][crate::sys::LIBRARY_FUNCTION_ERROR].
///
// * Extract the function arguments from the raw [`MArgument`] array.
// * Store the function return value in the raw [`MArgument`] return value field.
///
/// # Syntax
///
/// Export a function with a single argument.
///
/// ```no_run
/// export![square(_)];
/// ```
///
/// Export a function using the specified low-level shared library symbol name.
///
/// ```
/// export![square(_) as WL_square];
/// ```
///
/// Export multiple functions with one `export!` invocation. This is purely for convenience.
///
/// ```
/// export![
///     square(_);
///     add_two(_, _) as AddTwo;
/// ];
/// ```
///
// TODO: Remove this feature? If someone wants to export the low-level function, they
//       should do `pub use square::square as ...` instead of exposing the hidden module
//       (which is just an implementation detail of `export![]` anyway).
// Make public the `mod` module that contains the low-level wrapper function.
//
// ```
// export![pub square(_)];
// ```
///
/// # Examples
///
/// ### Primitive data types
///
/// Export a native function with a single argument:
///
/// ```
/// fn square(x: i64) -> i64 {
///     x * x
/// }
///
/// export![square(_)]
/// ```
///
/// ```wolfram
/// LibraryFunctionLoad["...", "square", {Integer}, Integer]
/// ```
///
/// Export a native function with multiple arguments:
///
/// ```
/// fn reverse_string(string: String) -> String {
///     string.chars().rev().collect()
/// }
/// ```
///
/// ### Numeric arrays
///
/// Export a native function with a [`NumericArray`] argument:
///
/// ```
/// fn total_i64(list: &NumericArray<i64>) -> i64 {
///     list.as_slice().into_iter().sum()
/// }
/// ```
///
/// ```wolfram
/// LibraryFunctionLoad[
///     "...", "total_i64",
///     {LibraryDataType[NumericArray, "Integer64"]}
///     Integer
/// ]
/// ```
///
///
// TODO: Add a "Memory Management" section to this comment and discuss "Constant".
//
// ```wolfram
// LibraryFunctionLoad[
//     "...", "total_i64",
//     {
//         {LibraryDataType[NumericArray, "Integer64"], "Constant"}
//     },
//     Integer
// ]
// ```
///
/// # Parameter types
///
/// The following table describes the relationship between Rust types that implement
/// [`FromArg`] and the compatible Wolfram LibraryLink function parameter type(s).
///
/// <h4 style="border-bottom: none; margin-bottom: 4px"> ⚠️ Warning! ⚠️ </h4>
///
/// Calling a LibraryLink function from the Wolfram Language that was loaded using the
/// wrong parameter type may lead to undefined behavior! Ensure that the function
/// parameter type declared in your Wolfram Language code matches the Rust function
/// parameter type.
///
/// Rust parameter type                | Wolfram library function parameter type
/// -----------------------------------|---------------------------------------
/// [`bool`]                           | `"Boolean"`
/// [`mint`]                           | `Integer`
/// [`mreal`][crate::sys::mreal]       | `Real`
/// [`mcomplex`][crate::sys::mcomplex] | `Complex`
/// [`String`]                         | `String`
/// [`CString`][std::ffi::CString]     | `String`
/// [`&NumericArray<T>`][NumericArray] | a. `LibraryDataType[NumericArray, `[`"..."`][ref/NumericArray]`]`[^1] <br/> b. `{LibraryDataType[NumericArray, "..."], "Constant"}`[^1]
/// [`NumericArray<T>`]                | a. `{LibraryDataType[NumericArray, "..."], "Manual"}`[^1] <br/> b. `{LibraryDataType[NumericArray, "..."], "Shared"}`[^1]
/// [`DataStore`]                      | `"DataStore"`
///
/// # Return types
///
/// The following table describes the relationship between Rust types that implement
/// [`IntoArg`] and the compatible Wolfram LibraryLink function return type.
///
/// Rust return type                   | Wolfram library function return type
/// -----------------------------------|---------------------------------------
/// [`bool`]                           | `"Boolean"`
/// [`mint`]                           | `Integer`
/// [`mreal`][crate::sys::mreal]       | `Real`
/// [`i8`], [`i16`], [`i32`]           | `Integer`
/// [`u8`], [`u16`], [`u32`]           | `Integer`
/// [`f32`]                            | `Real`
/// [`mcomplex`][crate::sys::mcomplex] | `Complex`
/// [`String`]                         | `String`
/// [`NumericArray<T>`]                | `LibraryDataType[NumericArray, `[`"..."`][ref/NumericArray][^1]`]`
/// [`DataStore`]                      | `"DataStore"`
///
/// [^1]: The Details and Options section of the Wolfram Language
///       [`NumericArray` reference page][ref/NumericArray] lists the available element
///       types.
///
/// [ref/NumericArray]: https://reference.wolfram.com/language/ref/NumericArray.html

// # Design constraints
//
// The current design of this macro is intended to accommodate the following constraints:
//
// 1. Support automatic generation of wrapper functions without using procedural macros,
//    and with minimal code duplication. Procedural macros require external dependencies,
//    and can significantly increase compile times.
//
//      1a. Don't depend on the entire function definition to be contained within the
//          macro invocation, which leads to unergonomic rightward drift. E.g. don't
//          require something like:
//
//          export![
//              fn foo(x: i64) { ... }
//          ]
//
//      1b. Don't depend on the entire function declaration to be repeated in the
//          macro invocation. E.g. don't require:
//
//              fn foo(x: i64) -> i64 {...}
//
//              export![
//                  fn foo(x: i64) -> i64;
//              ]
//
// 2. The name of the function in Rust should match the name of the function that appears
//    in the WL LibraryFunctionLoad call. E.g. needing different `foo` and `foo__wrapper`
//    named must be avoided.
//
// To satisfy constraint 1, it's necessary to depend on the type system rather than
// clever macro operations. This leads naturally to the creation of the `NativeFunction`
// trait, which is implemented for all suitable `Fn(..) -> _` types.
//
// Constraint 1b is unable to be met completely by the current implementation due to
// limitations with Rust's coercion from `fn(A, B, ..) -> C` to `Fn(A, B, ..) -> C`. The
// coercion requires that the number of parameters (`foo(_, _)`) be made explicit, even
// if their types can be elided. If eliding the number of Fn(..) arguments were permitted,
// `export![foo]` could work.
//
// To satisfy constraint 2, this implementation creates a private module with the same
// name as the function that is being wrapped. This is required because in Rust (as in
// many languages), it's illegal for two different functions with the same name to exist
// within the same module:
//
// ```
// fn foo { ... }
//
// #[no_mangle]
// pub extern "C" fn foo { ... } // Error: conflicts with the other foo()
// ```
//
// This means that the export![] macro cannot simply generate a wrapper function
// with the same name as the wrapped function, because they would conflict.
//
// However, it *is* legal for a module to contain a function and a child module that
// have the same name. Because `#[no_mangle]` functions are exported from the crate no
// matter where they appear in the module heirarchy, this offers an effective workaround
// for the name clash issue, while satisfy constraint 2's requirement that the original
// function and the wrapper function have the same name:
//
// ```
// fn foo() { ... } // This does not conflict with the `foo` module.
//
// mod foo {
//     #[no_mangle]
//     pub extern "C" fn foo(..) { ... } // This does not conflict with super::foo().
// }
// ```
#[macro_export]
macro_rules! export {
    ($vis:vis $name:ident($($argc:ty),*) as $exported:ident) => {
        $vis mod $name {
            #[no_mangle]
            pub unsafe extern "C" fn $exported(
                lib: $crate::sys::WolframLibraryData,
                argc: $crate::sys::mint,
                args: *mut $crate::sys::MArgument,
                res: $crate::sys::MArgument,
            ) -> std::os::raw::c_uint {
                // The number of `$argc` is required for type inference of the variadic
                // `&dyn Fn(..) -> _` type to work. See constraint 2a.
                let func: &dyn Fn($($argc),*) -> _ = &super::$name;

                $crate::macro_utils::call_native_wolfram_library_function(
                    lib,
                    args,
                    argc,
                    res,
                    func
                )
            }
        }
    };

    // Convert export![name(..)] to export![name(..) as name].
    ($vis:vis $name:ident($($argc:ty),*)) => {
        $crate::export![$vis $name($($argc),*) as $name];
    };

    ($($vis:vis $name:ident($($argc:ty),*) $(as $exported:ident)?);* $(;)?) => {
        $(
            $crate::export![$vis $name($($argc),*) $(as $exported)?];
        )*
    };
}

// TODO: Allow any type which implements FromExpr in wrapper parameter lists?
