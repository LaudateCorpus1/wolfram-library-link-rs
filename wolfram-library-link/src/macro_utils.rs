use std::os::raw::{c_int, c_uint};

use wstp::{self, Link};

use crate::{
    catch_panic::{call_and_catch_panic, CaughtPanic},
    expr::{Expr, Symbol},
    sys::{self, MArgument, LIBRARY_NO_ERROR},
    NativeFunction, WstpFunction,
};

/// Error codes returned by macro-generated wrapper code.
///
/// If no error occured, [`sys::LIBRARY_NO_ERROR`] is returned.
///
/// Using separate error codes for macro-generated code makes the source of the error
/// clearer when something goes wrong in wrapper code.
//
// TODO: Make this module public somewhere and document these error code in export!,
//       export_wstp!, and Overview.md.
mod error_code {
    use std::os::raw::c_uint;

    // Chosen arbitrarily. Avoids clashing with `LIBRARY_FUNCTION_ERROR` and related
    // error codes.
    const OFFSET: c_uint = 1000;

    /// A call to [initialize()][crate::initialize] failed.
    pub const FAILED_TO_INIT: c_uint = OFFSET + 1;

    /// The library code panicked.
    //
    // TODO: Wherever this code is set, also set a $LastError-like variable.
    pub const FAILED_WITH_PANIC: c_uint = OFFSET + 2;
}

//==================
// WSTP helpers
//==================

unsafe fn call_wstp_link_wolfram_library_function<
    F: FnOnce(&mut Link) + std::panic::UnwindSafe,
>(
    libdata: sys::WolframLibraryData,
    mut unsafe_link: wstp::sys::WSLINK,
    function: F,
) -> c_uint {
    // Initialize the library.
    if crate::initialize(libdata).is_err() {
        return error_code::FAILED_TO_INIT;
    }

    let link = Link::unchecked_ref_cast_mut(&mut unsafe_link);

    let result: Result<(), CaughtPanic> =
        call_and_catch_panic(std::panic::AssertUnwindSafe(|| {
            let _: () = function(link);
        }));

    match result {
        Ok(()) => LIBRARY_NO_ERROR,
        // Try to fail gracefully by writing the panic message as a Failure[..] object to
        // be returned, but if that fails, just return LIBRARY_FUNCTION_ERROR.
        Err(panic) => match write_panic_failure_to_link(link, panic) {
            Ok(()) => LIBRARY_NO_ERROR,
            Err(_wstp_err) => {
                // println!("PANIC ERROR: {}", _wstp_err);
                sys::LIBRARY_FUNCTION_ERROR // +1
            },
        },
    }
}

fn write_panic_failure_to_link(
    link: &mut Link,
    caught_panic: CaughtPanic,
) -> Result<(), wstp::Error> {
    // Clear the last error on the link, if any.
    //
    // This is necessary because the panic we caught might have been caused by
    // code like:
    //
    //     link.do_something(...).unwrap()
    //
    // where `do_something()` fails, which will have "poisoned" the link, and would cause
    // our attempt to write the panic message to the link to fail if we didn't clear the
    // error.
    //
    // If there is no error condition set on the link, this is a no-op.
    //
    // TODO: If an error *is* set, mention that in the Failure message? That might help
    //       users debug link issues more quickly.
    link.clear_error();

    // Skip whatever data is still stored in the link, if any.
    if link.is_ready() {
        link.raw_get_next()?;
        link.new_packet()?;
    }

    link.put_expr(&caught_panic.to_pretty_expr())
}

//======================================
// export! (NativeFunction) and export_wstp! (WstpFunction) helpers
//======================================

pub unsafe fn call_native_wolfram_library_function<'a, F: NativeFunction<'a>>(
    lib_data: sys::WolframLibraryData,
    args: *mut MArgument,
    argc: sys::mint,
    res: MArgument,
    func: F,
) -> c_uint {
    use std::panic::AssertUnwindSafe;

    // Initialize the library.
    if crate::initialize(lib_data).is_err() {
        return error_code::FAILED_TO_INIT;
    }

    let argc = match usize::try_from(argc) {
        Ok(argc) => argc,
        Err(_) => return sys::LIBRARY_FUNCTION_ERROR,
    };

    // FIXME: This isn't safe! 'a could be 'static, and then the user could store the
    //        `&mut Link` reference beyond the lifetime of this function.
    //        E.g. `fn foo(link: &'static mut str) { ... }`
    let args: &[MArgument] = std::slice::from_raw_parts(args, argc);

    if call_and_catch_panic(AssertUnwindSafe(move || func.call(args, res))).is_err() {
        // TODO: Store the panic into a "LAST_ERROR" static, and provide an accessor to
        //       get it from WL? E.g. RustLink`GetLastError[<optional func name>].
        return error_code::FAILED_WITH_PANIC;
    };

    sys::LIBRARY_NO_ERROR
}

pub unsafe fn call_wstp_wolfram_library_function<
    F: WstpFunction + std::panic::UnwindSafe,
>(
    libdata: sys::WolframLibraryData,
    unsafe_link: wstp::sys::WSLINK,
    func: F,
) -> c_uint {
    call_wstp_link_wolfram_library_function(
        libdata,
        unsafe_link,
        move |link: &mut Link| {
            let _: () = func.call(link);
        },
    )
}

//======================================
// Automatic Loader
//======================================

pub enum LibraryLinkFunction {
    Native {
        name: &'static str,
        /// # Implementation note on the type of this field
        ///
        /// In an ideal world, the type of this field would be something like
        /// `ty: Box<dyn NativeFunction>`.
        ///
        /// Using `fn() -> _` as the type of this field is necessary to work around
        /// the following constraints :
        ///
        /// * Instances of `LibraryLinkFunction` are constructed within a `static` context,
        ///   so only operations that are allowed in a `static` context can be used.
        ///
        /// * Can't be `&'static dyn for<'a> NativeFunction<'a>>`
        ///   - Doesn't work because it would require an intermediate `&'static fn(..)`
        ///     value, which can only be derived from an explicit `static FUNC: fn(..)`,
        ///     which in turn needs to be declared using explicit types for the function
        ///     parameter and return types (`static FUNC: fn(_, _) -> _` is not allowed,
        ///     because type inferrence doesn't work on static variables).
        ///
        /// * Can't be `Box<dyn for<'a> NativeFunction<'a>>`.
        ///   - Doesn't work because `Box::new()` can't be used in a `static` context.
        ///
        /// * Can't be `fn() -> Box<dyn NativeFunction<'a>>` because the `'a` lifetime
        ///   parameter can't be declared in any way.
        ///
        /// So in the end, we just call `NativeFunction::signature()` within `fn()`
        /// that is constructed in the macro-generated code (and where the concrete
        /// function type is still available) to avoid trying and failing to box up or
        /// return the `NativeFunction` trait object.
        signature: fn() -> Result<(Vec<Expr>, Expr), String>,
    },
    Wstp {
        name: &'static str,
    },
}

inventory::collect!(LibraryLinkFunction);

pub unsafe fn load_library_functions_impl(
    lib_data: sys::WolframLibraryData,
    raw_link: wstp::sys::WSLINK,
) -> c_uint {
    call_wstp_link_wolfram_library_function(lib_data, raw_link, |link: &mut Link| {
        let arg_count: usize =
            link.test_head("List").expect("expected 'List' expression");

        if arg_count != 1 {
            panic!(
                "expected 1 argument: the name of or file path to the dynamic library"
            );
        }

        let path = {
            let path = match link.get_string_ref() {
                Ok(value) => value,
                Err(err) => panic!("expected String argument (error: {})", err),
            };
            std::path::PathBuf::from(path.to_str())
        };

        let expr = library_function_load_expr(path);

        link.put_expr(&expr)
            .expect("failed to write loader Association");
    })
}

fn library_function_load_expr(library: std::path::PathBuf) -> Expr {
    let mut fields = Vec::new();
    let rule = Symbol::new("System`Rule");

    for func in inventory::iter::<LibraryLinkFunction> {
        let code = match func.loading_code(&library) {
            Ok(code) => code,
            // TODO: Generate a message? Return a Failure[..]? Doing nothing seems
            //       reasonable too. This only currently fails for
            //       `fn(&[MArgument], MArgument)` functions.
            Err(_) => continue,
        };

        fields.push(Expr::normal(&rule, vec![Expr::string(func.name()), code]));
    }

    Expr::normal(Symbol::new("System`Association"), fields)
}

impl LibraryLinkFunction {
    fn name(&self) -> &str {
        match self {
            LibraryLinkFunction::Native { name, .. } => name,
            LibraryLinkFunction::Wstp { name } => name,
        }
    }

    fn loading_code(&self, library: &std::path::PathBuf) -> Result<Expr, String> {
        fn sys(name: &str) -> Symbol {
            Symbol::new(&format!("System`{}", name))
        }

        let lib_func_load = sys("LibraryFunctionLoad");
        let link_object = Expr::from(sys("LinkObject"));
        let library = Expr::string(
            library
                .to_str()
                .expect("unable to convert library file path to str"),
        );

        let code = match self {
            LibraryLinkFunction::Native { name, signature } => {
                let (args, ret) = signature()?;

                Expr::normal(&lib_func_load, vec![
                    library.clone(),
                    Expr::string(*name),
                    Expr::normal(sys("List"), args),
                    ret,
                ])
            },
            /*
                With[{
                    var = LibraryFunctionLoad[...]
                },
                    Function[
                        (* Note:
                            Set $Context and $ContextPath to force symbols sent across
                            the LinkObject to contain the symbol context explicitly.
                        *)
                        Block[{$Context = "RustLinkWSTPPrivateContext`", $ContextPath = {}},
                            var[##]
                        ]
                    ]
                ]
            */
            LibraryLinkFunction::Wstp { name } => {
                let load_call = Expr::normal(&lib_func_load, vec![
                    library.clone(),
                    Expr::string(*name),
                    link_object.clone(),
                    link_object,
                ]);

                let var = Expr::from(Symbol::new("RustLink`Private`wstpFunc"));

                Expr::normal(sys("With"), vec![
                    Expr::normal(sys("List"), vec![Expr::normal(sys("Set"), vec![
                        var.clone(),
                        load_call,
                    ])]),
                    Expr::normal(sys("Function"), vec![Expr::normal(
                        sys("Block"),
                        vec![
                            Expr::normal(sys("List"), vec![
                                // $Context = "RustLinkWSTPPrivateContext`"
                                Expr::normal(sys("Set"), vec![
                                    Expr::from(sys("$Context")),
                                    Expr::string("RustLinkWSTPPrivateContext`"),
                                ]),
                                // $ContextPath = {}
                                Expr::normal(sys("Set"), vec![
                                    Expr::from(sys("$ContextPath")),
                                    Expr::normal(sys("List"), vec![]),
                                ]),
                            ]),
                            // var[##]
                            Expr::normal(var, vec![Expr::normal(
                                sys("SlotSequence"),
                                vec![Expr::from(1)],
                            )]),
                        ],
                    )]),
                ])
            },
        };

        Ok(code)
    }
}

//======================================
// Initialization
//======================================

pub unsafe fn init_with_user_function(
    lib: sys::WolframLibraryData,
    user_init_func: fn(),
) -> c_int {
    if let Err(()) = crate::initialize(lib) {
        return error_code::FAILED_TO_INIT as c_int;
    }

    if let Err(_) = call_and_catch_panic(user_init_func) {
        error_code::FAILED_WITH_PANIC as c_int
    } else {
        sys::LIBRARY_NO_ERROR as c_int
    }
}
