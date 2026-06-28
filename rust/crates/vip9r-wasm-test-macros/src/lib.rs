use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Item, ItemMod, parse_macro_input, parse_quote};

#[proc_macro_attribute]
pub fn wasm_tests(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return compile_error("#[wasm_tests] does not accept arguments");
    }

    let native_module = parse_macro_input!(item as ItemMod);
    let mut wasm_module = native_module.clone();
    let Some((_, items)) = &mut wasm_module.content else {
        return compile_error("#[wasm_tests] must annotate an inline module");
    };

    let mut wrappers = Vec::new();
    for item in items.iter_mut() {
        let Item::Fn(function) = item else {
            continue;
        };
        if !take_test_attr(&mut function.attrs) {
            continue;
        }
        if !function.sig.generics.params.is_empty()
            || function.sig.constness.is_some()
            || function.sig.asyncness.is_some()
            || function.sig.unsafety.is_some()
            || function.sig.abi.is_some()
            || !function.sig.inputs.is_empty()
        {
            let error = syn::Error::new_spanned(
                &function.sig,
                "#[test] functions in #[wasm_tests] modules must be safe, non-async, non-generic functions with no parameters",
            );
            return error.into_compile_error().into();
        }

        let name = &function.sig.ident;
        let wrapper_name = format_ident!("__vip9r_wasm_test_{name}");
        wrappers.push(quote! {
            #[cfg(all(target_arch = "wasm32", feature = "wasm-tests"))]
            #[unsafe(export_name = concat!("vip9r_test__", module_path!(), "::", stringify!(#name)))]
            pub extern "C" fn #wrapper_name() -> i32 {
                __vip9r_wasm_test_run(#name)
            }
        });
    }

    if !wrappers.is_empty() {
        items.push(parse_quote! {
            #[cfg(all(target_arch = "wasm32", feature = "wasm-tests"))]
            trait __Vip9rWasmTestReturn {
                fn __vip9r_wasm_test_finish(self) -> i32;
            }
        });
        items.push(parse_quote! {
            #[cfg(all(target_arch = "wasm32", feature = "wasm-tests"))]
            impl __Vip9rWasmTestReturn for () {
                fn __vip9r_wasm_test_finish(self) -> i32 {
                    0
                }
            }
        });
        items.push(parse_quote! {
            #[cfg(all(target_arch = "wasm32", feature = "wasm-tests"))]
            impl<E: core::fmt::Debug> __Vip9rWasmTestReturn for Result<(), E> {
                fn __vip9r_wasm_test_finish(self) -> i32 {
                    match self {
                        Ok(()) => 0,
                        Err(_) => 1,
                    }
                }
            }
        });
        items.push(parse_quote! {
            #[cfg(all(target_arch = "wasm32", feature = "wasm-tests"))]
            fn __vip9r_wasm_test_run<T>(test: fn() -> T) -> i32
            where
                T: __Vip9rWasmTestReturn,
            {
                test().__vip9r_wasm_test_finish()
            }
        });
        for wrapper in wrappers {
            items.push(parse_quote!(#wrapper));
        }
    }

    quote! {
        #[cfg(test)]
        #native_module

        #[cfg(all(not(test), target_arch = "wasm32", feature = "wasm-tests"))]
        #wasm_module
    }
    .into()
}

fn take_test_attr(attrs: &mut Vec<Attribute>) -> bool {
    let mut found = false;
    attrs.retain(|attr| {
        if attr.path().is_ident("test") {
            found = true;
            false
        } else {
            true
        }
    });
    found
}

fn compile_error(message: &str) -> TokenStream {
    format!("compile_error!({message:?});")
        .parse()
        .expect("compile_error is valid Rust")
}
