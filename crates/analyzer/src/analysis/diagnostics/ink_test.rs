//! ink! test diagnostics.

use ink_analyzer_ir::InkTest;

use super::utils;
use crate::Diagnostic;

const TEST_SCOPE_NAME: &str = "test";

/// Runs all ink! test diagnostics.
///
/// The entry point for finding ink! test semantic rules is the `ink_test` module of the `ink_ir` crate.
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/ink_test.rs#L34-L44>.
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/ink_test.rs#L27-L30>.
pub fn diagnostics(results: &mut Vec<Diagnostic>, ink_test: &InkTest) {
    // Runs generic diagnostics, see `utils::run_generic_diagnostics` doc.
    utils::run_generic_diagnostics(results, ink_test);

    // Ensures that ink! test is an `fn` item, see `utils::ensure_fn` doc.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/ink_test.rs#L27>.
    if let Some(diagnostic) = utils::ensure_fn(ink_test, TEST_SCOPE_NAME) {
        results.push(diagnostic);
    }

    // Ensures that ink! test has no ink! descendants, see `utils::ensure_no_ink_descendants` doc.
    utils::ensure_no_ink_descendants(results, ink_test, TEST_SCOPE_NAME);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::verify_actions;
    use crate::Severity;
    use ink_analyzer_ir::syntax::{TextRange, TextSize};
    use ink_analyzer_ir::{FromInkAttribute, InkAttributeKind, InkFile, InkMacroKind, IsInkEntity};
    use quote::quote;
    use test_utils::{
        parse_offset_at, quote_as_pretty_string, quote_as_str, TestResultAction,
        TestResultTextRange,
    };

    fn parse_first_ink_test(code: &str) -> InkTest {
        InkTest::cast(
            InkFile::parse(code)
                .tree()
                .ink_attrs_in_scope()
                .find(|attr| *attr.kind() == InkAttributeKind::Macro(InkMacroKind::Test))
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn fn_works() {
        let ink_test = parse_first_ink_test(quote_as_str! {
            #[ink::test]
            fn it_works() {
            }
        });

        let result = utils::ensure_fn(&ink_test, TEST_SCOPE_NAME);
        assert!(result.is_none());
    }

    #[test]
    fn non_fn_fails() {
        for code in [
            quote! {
                struct MyTest {
                }
            },
            quote! {
                enum MyTest {
                }
            },
            quote! {
                mod my_test;
            },
            quote! {
                trait MyTest {
                }
            },
        ] {
            let code = quote_as_pretty_string! {
                #[ink::test]
                #code
            };
            let ink_test = parse_first_ink_test(&code);

            let result = utils::ensure_fn(&ink_test, TEST_SCOPE_NAME);

            // Verifies diagnostics.
            assert!(result.is_some(), "ink test: {code}");
            assert_eq!(
                result.as_ref().unwrap().severity,
                Severity::Error,
                "ink test: {code}"
            );
            // Verifies quickfixes.
            let fix = &result.as_ref().unwrap().quickfixes.as_ref().unwrap()[0];
            assert!(fix.label.contains("Remove `#[ink::test]`"));
            assert!(fix.edits[0].text.is_empty());
            assert_eq!(
                fix.edits[0].range,
                TextRange::new(
                    TextSize::from(parse_offset_at(&code, Some("<-#[ink::test]")).unwrap() as u32),
                    TextSize::from(parse_offset_at(&code, Some("#[ink::test]")).unwrap() as u32)
                )
            );
        }
    }

    #[test]
    fn no_ink_descendants_works() {
        let ink_test = parse_first_ink_test(quote_as_str! {
            #[ink::test]
            fn it_works() {
            }
        });

        let mut results = Vec::new();
        utils::ensure_no_ink_descendants(&mut results, &ink_test, TEST_SCOPE_NAME);
        assert!(results.is_empty());
    }

    #[test]
    fn ink_descendants_fails() {
        let code = quote_as_pretty_string! {
            #[ink::test]
            fn it_works() {
                #[ink(event)]
                struct MyEvent {
                    #[ink(topic)]
                    value: bool,
                }
            }
        };
        let ink_test = parse_first_ink_test(&code);

        let mut results = Vec::new();
        utils::ensure_no_ink_descendants(&mut results, &ink_test, TEST_SCOPE_NAME);

        // 2 diagnostics for `event` and `topic`.
        assert_eq!(results.len(), 2);
        // All diagnostics should be errors.
        assert_eq!(
            results
                .iter()
                .filter(|item| item.severity == Severity::Error)
                .count(),
            2
        );
        // Verifies quickfixes.
        let expected_quickfixes = vec![
            vec![
                TestResultAction {
                    label: "Remove `#[ink(event)]`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-#[ink(event)]"),
                        end_pat: Some("#[ink(event)]"),
                    }],
                },
                TestResultAction {
                    label: "Remove item",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-#[ink(event)]"),
                        end_pat: Some("}"),
                    }],
                },
            ],
            vec![TestResultAction {
                label: "Remove `#[ink(topic)]`",
                edits: vec![TestResultTextRange {
                    text: "",
                    start_pat: Some("<-#[ink(topic)]"),
                    end_pat: Some("#[ink(topic)]"),
                }],
            }],
        ];
        for (idx, item) in results.iter().enumerate() {
            let quickfixes = item.quickfixes.as_ref().unwrap();
            verify_actions(&code, quickfixes, &expected_quickfixes[idx]);
        }
    }

    #[test]
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/macro/src/lib.rs#L824-L841>.
    fn compound_diagnostic_works() {
        for code in [
            quote_as_str! {
                // Conventional unit test that works with assertions.
                #[ink::test]
                fn test1() {
                   // test code comes here as usual
                }
            },
            quote_as_str! {
                // Conventional unit test that returns some Result.
                // The test code can make use of operator-`?`.
                #[ink::test]
                fn test2() -> Result<(), ink_env::Error> {
                    // test code that returns a Rust Result type
                }
            },
        ] {
            let ink_test = parse_first_ink_test(code);

            let mut results = Vec::new();
            diagnostics(&mut results, &ink_test);
            assert!(results.is_empty(), "ink test: {code}");
        }
    }
}
