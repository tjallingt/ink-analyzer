//! ink! message diagnostics.

use ink_analyzer_ir::ast::AstNode;
use ink_analyzer_ir::{ast, IsInkFn, Message};

use super::utils;
use crate::analysis::text_edit::TextEdit;
use crate::analysis::utils as analysis_utils;
use crate::{Action, ActionKind, Diagnostic, Severity};

const MESSAGE_SCOPE_NAME: &str = "message";

/// Runs all ink! message diagnostics.
///
/// The entry point for finding ink! message semantic rules is the message module of the `ink_ir` crate.
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L201-L216>.
pub fn diagnostics(results: &mut Vec<Diagnostic>, message: &Message) {
    // Runs generic diagnostics, see `utils::run_generic_diagnostics` doc.
    utils::run_generic_diagnostics(results, message);

    // Ensures that ink! message is an `fn` item, see `utils::ensure_fn` doc.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L201>.
    if let Some(diagnostic) = utils::ensure_fn(message, MESSAGE_SCOPE_NAME) {
        results.push(diagnostic);
    }

    if let Some(fn_item) = message.fn_item() {
        // Ensures that ink! message `fn` item satisfies all common invariants of externally callable ink! entities,
        // see `utils::ensure_callable_invariants` doc.
        // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L202>.
        // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/callable.rs#L355-L440>.
        utils::ensure_callable_invariants(results, fn_item, MESSAGE_SCOPE_NAME);

        // Ensures that ink! message `fn` item has a self reference receiver, see `ensure_receiver_is_self_ref` doc.
        if let Some(diagnostic) = ensure_receiver_is_self_ref(fn_item) {
            results.push(diagnostic);
        }

        // Ensures that ink! message `fn` item does not return `Self`, see `ensure_not_return_self` doc.
        if let Some(diagnostic) = ensure_not_return_self(fn_item) {
            results.push(diagnostic);
        }
    }

    // Ensures that ink! message has no ink! descendants, see `utils::ensure_no_ink_descendants` doc.
    utils::ensure_no_ink_descendants(results, message, MESSAGE_SCOPE_NAME);
}

/// Ensures that ink! message `fn` has a self reference receiver (i.e `&self` or `&mut self`).
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L203>.
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L121-L150>.
fn ensure_receiver_is_self_ref(fn_item: &ast::Fn) -> Option<Diagnostic> {
    // Determines if the function has a self reference receiver.
    let has_self_ref_receiver = fn_item
        .param_list()
        .as_ref()
        .and_then(ast::ParamList::self_param)
        .map_or(false, |self_param| self_param.amp_token().is_some());

    // Gets the declaration range for the item.
    let range = analysis_utils::ast_item_declaration_range(&ast::Item::Fn(fn_item.clone()))
        .unwrap_or(fn_item.syntax().text_range());

    (!has_self_ref_receiver).then_some(Diagnostic {
        message: "ink! message must have a self reference receiver (i.e `&self` or `&mut self`)."
            .to_string(),
        range,
        severity: Severity::Error,
        quickfixes: fn_item
            .param_list()
            .and_then(|param_list| param_list.l_paren_token())
            .map(|it| it.text_range().end())
            .map(|insert_offset| {
                let has_more_params = fn_item
                    .param_list()
                    .map_or(false, |param_list| param_list.params().next().is_some());
                let insert_suffix = if has_more_params { ", " } else { "" };
                vec![
                    Action {
                        label: "Add immutable self reference receiver".to_string(),
                        kind: ActionKind::QuickFix,
                        range,
                        edits: vec![TextEdit::insert(
                            format!("&self{insert_suffix}"),
                            insert_offset,
                        )],
                    },
                    Action {
                        label: "Add mutable self reference receiver".to_string(),
                        kind: ActionKind::QuickFix,
                        range,
                        edits: vec![TextEdit::insert(
                            format!("&mut self{insert_suffix}"),
                            insert_offset,
                        )],
                    },
                ]
            }),
    })
}

/// Ensures that ink! message does not return `Self`.
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L204>.
///
/// Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L152-L174>.
fn ensure_not_return_self(fn_item: &ast::Fn) -> Option<Diagnostic> {
    let return_type = fn_item.ret_type()?.ty()?;
    // Edit range for quickfix.
    let range = analysis_utils::node_and_trivia_range(fn_item.ret_type()?.syntax());
    (return_type.to_string() == "Self").then_some(Diagnostic {
        message: "ink! message must not return `Self`.".to_string(),
        range: return_type.syntax().text_range(),
        severity: Severity::Error,
        quickfixes: Some(vec![Action {
            label: "Remove `Self` return type.".to_string(),
            kind: ActionKind::QuickFix,
            range,
            edits: vec![TextEdit::delete(range)],
        }]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::verify_actions;
    use ink_analyzer_ir::{FromInkAttribute, InkArgKind, InkAttributeKind, InkFile, IsInkEntity};
    use quote::quote;
    use test_utils::{quote_as_pretty_string, quote_as_str, TestResultAction, TestResultTextRange};

    fn parse_first_message(code: &str) -> Message {
        Message::cast(
            InkFile::parse(code)
                .tree()
                .ink_attrs_in_scope()
                .find(|attr| *attr.kind() == InkAttributeKind::Arg(InkArgKind::Message))
                .unwrap(),
        )
        .unwrap()
    }

    // List of valid minimal ink! messages used for positive(`works`) tests for ink! message verifying utilities.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L545-L584>.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L389-L412>.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L341-L364>.
    macro_rules! valid_messages {
        () => {
            [
                // &self + inherited visibility
                quote! {
                    fn my_message(&self) {}
                },
                // &self + pub
                quote! {
                    pub fn my_message(&self) {}
                },
                // &mut self + inherited visibility
                quote! {
                    fn my_message(&mut self) {}
                },
                // &mut self + pub
                quote! {
                    pub fn my_message(&mut self) {}
                },
                // &self + single input + output
                quote! {
                    fn my_message(&self, a: i32) -> bool {}
                },
                // &mut self + single input + output
                quote! {
                    fn my_message(&mut self, a: i32) -> bool {}
                },
                // &self + single input + tuple output
                quote! {
                    fn my_message(&self, a: i32) -> (i32, u64, bool) {}
                },
                // &mut self + single input + tuple output
                quote! {
                    fn my_message(&mut self, a: i32) -> (i32, u64, bool) {}
                },
                // &self + many inputs + output
                quote! {
                    fn my_message(&self, a: i32, b: u64, c: [u8; 32]) -> bool {}
                },
                // &mut self + many inputs + output
                quote! {
                    fn my_message(&mut self, a: i32, b: u64, c: [u8; 32]) -> bool {}
                },
                // &self + many inputs + tuple output
                quote! {
                    fn my_message(&self, a: i32, b: u64, c: [u8; 32]) -> (i32, u64, bool) {}
                },
                // &mut self + many inputs + tuple output
                quote! {
                    fn my_message(&mut self, a: i32, b: u64, c: [u8; 32]) -> (i32, u64, bool) {}
                },
            ]
            .iter()
            .flat_map(|code| {
                [
                    // Simple.
                    quote! {
                        #[ink(message)]
                        #code
                    },
                    // Payable.
                    quote! {
                        #[ink(message, payable)]
                        #code
                    },
                    // Selector.
                    quote! {
                        #[ink(message, selector=1)]
                        #code
                    },
                    quote! {
                        #[ink(message, selector=0x1)]
                        #code
                    },
                    quote! {
                        #[ink(message, selector=_)]
                        #code
                    },
                    // Compound.
                    quote! {
                        #[ink(message, payable, default, selector=1)]
                        #code
                    },
                    quote! {
                        #[ink(message)]
                        #[ink(payable, default, selector=1)]
                        #code
                    },
                    quote! {
                        #[ink(message)]
                        #[ink(payable)]
                        #[ink(default)]
                        #[ink(selector=1)]
                        #code
                    },
                ]
            })
        };
    }

    #[test]
    fn valid_callable_works() {
        for code in valid_messages!() {
            let message = parse_first_message(quote_as_str! {
                #code
            });

            let mut results = Vec::new();
            utils::ensure_callable_invariants(
                &mut results,
                message.fn_item().unwrap(),
                MESSAGE_SCOPE_NAME,
            );
            assert!(results.is_empty(), "message: {code}");
        }
    }

    #[test]
    fn invalid_callable_fails() {
        for (code, expected_quickfixes) in [
            // Bad visibility.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L751-L781>.
            (
                quote! {
                    pub(crate) fn my_message(&self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(crate)"),
                            end_pat: Some("pub(crate)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(crate)"),
                            end_pat: Some("pub(crate) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(crate) fn my_message(&mut self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(crate)"),
                            end_pat: Some("pub(crate)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(crate)"),
                            end_pat: Some("pub(crate) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(self) fn my_message(&self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(self)"),
                            end_pat: Some("pub(self)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(self)"),
                            end_pat: Some("pub(self) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(self) fn my_message(&mut self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(self)"),
                            end_pat: Some("pub(self)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(self)"),
                            end_pat: Some("pub(self) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(super) fn my_message(&self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(super)"),
                            end_pat: Some("pub(super)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(super)"),
                            end_pat: Some("pub(super) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(super) fn my_message(&mut self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(super)"),
                            end_pat: Some("pub(super)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(super)"),
                            end_pat: Some("pub(super) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(in my::path) fn my_message(&self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(in my::path)"),
                            end_pat: Some("pub(in my::path)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(in my::path)"),
                            end_pat: Some("pub(in my::path) "),
                        }],
                    },
                ],
            ),
            (
                quote! {
                    pub(in my::path) fn my_message(&mut self) {}
                },
                vec![
                    TestResultAction {
                        label: "`pub`",
                        edits: vec![TestResultTextRange {
                            text: "pub",
                            start_pat: Some("<-pub(in my::path)"),
                            end_pat: Some("pub(in my::path)"),
                        }],
                    },
                    TestResultAction {
                        label: "Remove",
                        edits: vec![TestResultTextRange {
                            text: "",
                            start_pat: Some("<-pub(in my::path)"),
                            end_pat: Some("pub(in my::path) "),
                        }],
                    },
                ],
            ),
            // Generic params fails.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L599-L622>.
            (
                quote! {
                    fn my_message<T>(&self) {}
                },
                vec![TestResultAction {
                    label: "Remove generic",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-<T>"),
                        end_pat: Some("<T>"),
                    }],
                }],
            ),
            (
                quote! {
                    pub fn my_message<T>(&self) {}
                },
                vec![TestResultAction {
                    label: "Remove generic",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-<T>"),
                        end_pat: Some("<T>"),
                    }],
                }],
            ),
            (
                quote! {
                    fn my_message<T>(&mut self) {}
                },
                vec![TestResultAction {
                    label: "Remove generic",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-<T>"),
                        end_pat: Some("<T>"),
                    }],
                }],
            ),
            (
                quote! {
                    pub fn my_message<T>(&mut self) {}
                },
                vec![TestResultAction {
                    label: "Remove generic",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-<T>"),
                        end_pat: Some("<T>"),
                    }],
                }],
            ),
            // Const fails.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L656-L673>.
            (
                quote! {
                    const fn my_message(&self) {}
                },
                vec![TestResultAction {
                    label: "Remove `const`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-const"),
                        end_pat: Some("const "),
                    }],
                }],
            ),
            (
                quote! {
                    const fn my_message(&mut self) {}
                },
                vec![TestResultAction {
                    label: "Remove `const`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-const"),
                        end_pat: Some("const "),
                    }],
                }],
            ),
            // Async fails.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L675-L692>.
            (
                quote! {
                    async fn my_message(&self) {}
                },
                vec![TestResultAction {
                    label: "Remove `async`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-async"),
                        end_pat: Some("async "),
                    }],
                }],
            ),
            (
                quote! {
                    async fn my_message(&mut self) {}
                },
                vec![TestResultAction {
                    label: "Remove `async`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-async"),
                        end_pat: Some("async "),
                    }],
                }],
            ),
            // Unsafe fails.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L694-L711>.
            (
                quote! {
                    unsafe fn my_message(&self) {}
                },
                vec![TestResultAction {
                    label: "Remove `unsafe`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-unsafe"),
                        end_pat: Some("unsafe "),
                    }],
                }],
            ),
            (
                quote! {
                    unsafe fn my_message(&mut self) {}
                },
                vec![TestResultAction {
                    label: "Remove `unsafe`",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-unsafe"),
                        end_pat: Some("unsafe "),
                    }],
                }],
            ),
            // Explicit ABI fails.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L713-L730>.
            (
                quote! {
                    extern "C" fn my_message(&self) {}
                },
                vec![TestResultAction {
                    label: "Remove explicit ABI",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some(r#"<-extern "C""#),
                        end_pat: Some(r#"extern "C" "#),
                    }],
                }],
            ),
            (
                quote! {
                    extern "C" fn my_message(&mut self) {}
                },
                vec![TestResultAction {
                    label: "Remove explicit ABI",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some(r#"<-extern "C""#),
                        end_pat: Some(r#"extern "C" "#),
                    }],
                }],
            ),
            // Variadic fails.
            // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L732-L749>.
            (
                quote! {
                    fn my_message(&self, ...) {}
                },
                vec![TestResultAction {
                    label: "un-variadic",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-, ..."),
                        end_pat: Some("..."),
                    }],
                }],
            ),
            (
                quote! {
                    fn my_message(&mut self, ...) {}
                },
                vec![TestResultAction {
                    label: "un-variadic",
                    edits: vec![TestResultTextRange {
                        text: "",
                        start_pat: Some("<-, ..."),
                        end_pat: Some("..."),
                    }],
                }],
            ),
        ] {
            let code = quote_as_pretty_string! {
                #[ink(message)]
                #code
            };
            let message = parse_first_message(&code);

            let mut results = Vec::new();
            utils::ensure_callable_invariants(
                &mut results,
                message.fn_item().unwrap(),
                MESSAGE_SCOPE_NAME,
            );

            // Verifies diagnostics.
            assert_eq!(results.len(), 1, "message: {code}");
            assert_eq!(results[0].severity, Severity::Error, "message: {code}");
            // Verifies quickfixes.
            verify_actions(
                &code,
                results[0].quickfixes.as_ref().unwrap(),
                &expected_quickfixes,
            );
        }
    }

    #[test]
    fn self_ref_receiver_works() {
        for code in valid_messages!() {
            let message = parse_first_message(quote_as_str! {
                #code
            });

            let result = ensure_receiver_is_self_ref(message.fn_item().unwrap());
            assert!(result.is_none(), "message: {code}");
        }
    }

    #[test]
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L624-L654>.
    fn non_self_ref_receiver_fails() {
        for code in [
            quote! {
                fn my_message() {}
            },
            quote! {
                fn my_message(self) {}
            },
            quote! {
                fn my_message(mut self) {}
            },
            quote! {
                fn my_message(this: &Self) {}
            },
            quote! {
                fn my_message(this: &mut Self) {}
            },
        ] {
            let code = quote_as_pretty_string! {
                #[ink(message)]
                #code
            };
            let message = parse_first_message(&code);

            let result = ensure_receiver_is_self_ref(message.fn_item().unwrap());

            // Verifies diagnostics.
            assert!(result.is_some(), "message: {code}");
            assert_eq!(
                result.as_ref().unwrap().severity,
                Severity::Error,
                "message: {code}"
            );
            // Verifies quickfixes.
            let expected_quickfixes = vec![
                TestResultAction {
                    label: "self reference receiver",
                    edits: vec![TestResultTextRange {
                        text: "&self",
                        start_pat: Some("fn my_message("),
                        end_pat: Some("fn my_message("),
                    }],
                },
                TestResultAction {
                    label: "self reference receiver",
                    edits: vec![TestResultTextRange {
                        text: "&mut self",
                        start_pat: Some("fn my_message("),
                        end_pat: Some("fn my_message("),
                    }],
                },
            ];
            let quickfixes = result.as_ref().unwrap().quickfixes.as_ref().unwrap();
            verify_actions(&code, quickfixes, &expected_quickfixes);
        }
    }

    #[test]
    fn non_self_return_type_works() {
        for code in valid_messages!() {
            let message = parse_first_message(quote_as_str! {
                #code
            });

            let result = ensure_not_return_self(message.fn_item().unwrap());
            assert!(result.is_none(), "message: {code}");
        }
    }

    #[test]
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L624-L654>.
    fn self_return_type_fails() {
        for code in [
            quote! {
                fn my_message(&self) -> Self {}
            },
            quote! {
                fn my_message(&mut self) -> Self {}
            },
            quote! {
                pub fn my_message(&self) -> Self {}
            },
            quote! {
                pub fn my_message(&mut self) -> Self {}
            },
        ] {
            let code = quote_as_pretty_string! {
                #[ink(message)]
                #code
            };
            let message = parse_first_message(&code);

            let result = ensure_not_return_self(message.fn_item().unwrap());

            // Verifies diagnostics.
            assert!(result.is_some(), "message: {code}");
            assert_eq!(
                result.as_ref().unwrap().severity,
                Severity::Error,
                "message: {code}"
            );
            // Verifies quickfixes.
            let expected_quickfixes = vec![TestResultAction {
                label: "Remove `Self`",
                edits: vec![TestResultTextRange {
                    text: "",
                    start_pat: Some("<--> Self"),
                    end_pat: Some("-> Self "),
                }],
            }];
            let quickfixes = result.as_ref().unwrap().quickfixes.as_ref().unwrap();
            verify_actions(&code, quickfixes, &expected_quickfixes);
        }
    }

    #[test]
    fn no_ink_descendants_works() {
        for code in valid_messages!() {
            let message = parse_first_message(quote_as_str! {
                #code
            });

            let mut results = Vec::new();
            utils::ensure_no_ink_descendants(&mut results, &message, MESSAGE_SCOPE_NAME);
            assert!(results.is_empty(), "message: {code}");
        }
    }

    #[test]
    fn ink_descendants_fails() {
        let code = quote_as_pretty_string! {
            #[ink(message)]
            pub fn my_message(&mut self) {
                #[ink(event)]
                struct MyEvent {
                    #[ink(topic)]
                    value: bool,
                }
            }
        };
        let message = parse_first_message(&code);

        let mut results = Vec::new();
        utils::ensure_no_ink_descendants(&mut results, &message, MESSAGE_SCOPE_NAME);

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
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L545-L584>.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L389-L412>.
    // Ref: <https://github.com/paritytech/ink/blob/v4.1.0/crates/ink/ir/src/ir/item_impl/message.rs#L341-L364>.
    fn compound_diagnostic_works() {
        for code in valid_messages!() {
            let message = parse_first_message(quote_as_str! {
                #code
            });

            let mut results = Vec::new();
            diagnostics(&mut results, &message);
            assert!(results.is_empty(), "message: {code}");
        }
    }
}
