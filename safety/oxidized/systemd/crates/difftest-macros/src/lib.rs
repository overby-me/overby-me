//! Proc-macro crate for the `difftest` differential testing framework.
//!
//! Provides the `#[difftest]` attribute macro for registering differential
//! tests that compare systemd and rust-systemd behavior.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Expr, ItemFn, Meta, Token, parse_macro_input, punctuated::Punctuated};

/// Attribute macro for registering differential tests.
///
/// Marks a function as a differential test that will be discovered and executed
/// by the `DiffTestRunner`. The function must return a type implementing
/// `DiffTest`.
///
/// # Attributes
///
/// - `category = "..."` — Test category for filtering and reporting (e.g.
///   `"unit_parsing"`, `"service_lifecycle"`, `"dbus"`).
/// - `timeout_ms = N` — Per-test timeout in milliseconds (default: 30000).
/// - `tags = "tag1,tag2"` — Comma-separated tags for filtering.
/// - `ignore` — Skip this test by default (can be un-ignored with `--include-ignored`).
///
/// # Examples
///
/// ```ignore
/// use difftest::DiffTest;
/// use difftest_macros::difftest;
///
/// #[difftest(category = "unit_parsing")]
/// fn ini_parser_basic() -> impl DiffTest {
///     UnitFileDiffTest::new("corpus/units/basic.service")
/// }
///
/// #[difftest(category = "calendar", timeout_ms = 60000)]
/// fn calendar_minutely() -> impl DiffTest {
///     CalendarDiffTest::new("minutely")
/// }
///
/// #[difftest(category = "dbus", ignore)]
/// fn dbus_manager_list_units() -> impl DiffTest {
///     DbusMethodDiffTest::new("org.freedesktop.systemd1", "ListUnits")
/// }
/// ```
#[proc_macro_attribute]
pub fn difftest(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let fn_vis = &input_fn.vis;
    let fn_block = &input_fn.block;
    let fn_sig = &input_fn.sig;
    let fn_attrs = &input_fn.attrs;

    // Parse attributes
    let mut category = String::from("default");
    let mut timeout_ms: u64 = 30_000;
    let mut tags = Vec::<String>::new();
    let mut ignore = false;

    if !attr.is_empty() {
        let attr2: proc_macro2::TokenStream = attr.into();
        let parsed: Result<Punctuated<Meta, Token![,]>, _> = syn::parse::Parser::parse2(
            Punctuated::<Meta, Token![,]>::parse_terminated,
            attr2.clone(),
        );

        match parsed {
            Ok(metas) => {
                for meta in &metas {
                    match meta {
                        Meta::Path(path) if path.is_ident("ignore") => {
                            ignore = true;
                        }
                        Meta::NameValue(nv) => {
                            if nv.path.is_ident("category") {
                                if let Expr::Lit(expr_lit) = &nv.value
                                    && let syn::Lit::Str(lit) = &expr_lit.lit
                                {
                                    category = lit.value();
                                }
                            } else if nv.path.is_ident("timeout_ms") {
                                if let Expr::Lit(expr_lit) = &nv.value
                                    && let syn::Lit::Int(lit) = &expr_lit.lit
                                {
                                    timeout_ms = lit.base10_parse().unwrap_or(30_000);
                                }
                            } else if nv.path.is_ident("tags")
                                && let Expr::Lit(expr_lit) = &nv.value
                                && let syn::Lit::Str(lit) = &expr_lit.lit
                            {
                                tags = lit
                                    .value()
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {
                // Fallback: try parsing as simple assignment expressions for
                // forward-compatibility
            }
        }
    }

    let tag_tokens: Vec<proc_macro2::TokenStream> = tags
        .iter()
        .map(|t| {
            quote! { #t.to_string() }
        })
        .collect();

    // Generate a test function that can be run via `cargo test` as well
    let test_fn_ident = syn::Ident::new(&format!("difftest_{}", fn_name_str), fn_name.span());

    let ignore_attr = if ignore {
        quote! { #[ignore] }
    } else {
        quote! {}
    };

    let expanded = quote! {
        // Keep the original function (it returns an impl DiffTest)
        #(#fn_attrs)*
        #fn_vis #fn_sig {
            #fn_block
        }

        // Register this test with the inventory so the runner can discover it
        ::difftest::inventory::submit! {
            ::difftest::DiffTestRegistration {
                name: #fn_name_str,
                category: #category,
                timeout_ms: #timeout_ms,
                tags: &[#(#tag_tokens),*],
                ignored: #ignore,
                constructor: || ::std::boxed::Box::new(#fn_name()),
            }
        }

        // Also generate a standard #[test] so `cargo test` picks it up
        #[test]
        #ignore_attr
        fn #test_fn_ident() {
            let test_impl = #fn_name();
            let result = ::difftest::run_single_difftest(&test_impl, #timeout_ms);
            if let ::difftest::DiffResult::Divergent(ref explanation) = result {
                panic!(
                    "Differential test `{}` diverged:\n{}",
                    #fn_name_str, explanation
                );
            }
        }
    };

    TokenStream::from(expanded)
}
