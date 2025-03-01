//! AST item code/intent actions.

use ink_analyzer_ir::ast::HasAttrs;
use ink_analyzer_ir::syntax::{AstNode, SyntaxKind, SyntaxNode, SyntaxToken, TextRange, TextSize};
use ink_analyzer_ir::{
    ast, ChainExtension, Contract, Event, FromInkAttribute, FromSyntax, InkArgKind, InkAttribute,
    InkAttributeKind, InkFile, InkImpl, InkMacroKind, TraitDefinition,
};
use itertools::Itertools;

use super::entity;
use super::{Action, ActionKind};
use crate::analysis::utils;
use crate::TextEdit;

/// Computes AST item-based ink! attribute actions at the given text range.
pub fn actions(results: &mut Vec<Action>, file: &InkFile, range: TextRange) {
    match utils::focused_element(file, range) {
        // Computes actions based on focused element (if it can be determined).
        Some(focused_elem) => {
            // Computes an offset for inserting around the focused element
            // (i.e. insert at the end of the focused element except if it's whitespace,
            // in which case insert based on the passed text range).
            let focused_elem_insert_offset = || -> TextSize {
                if focused_elem.kind() == SyntaxKind::WHITESPACE
                    && focused_elem.text_range().contains_range(range)
                {
                    range
                } else {
                    focused_elem.text_range()
                }
                .end()
            };

            // Only computes actions if the focused element isn't part of an attribute.
            if utils::covering_attribute(file, range).is_none() {
                match utils::parent_ast_item(file, range) {
                    // Computes actions based on the parent AST item.
                    Some(ast_item) => {
                        // Gets the covering struct record field (if any) if the AST item is a struct.
                        let record_field: Option<ast::RecordField> =
                            matches!(&ast_item, ast::Item::Struct(_))
                                .then(|| ink_analyzer_ir::closest_ancestor_ast_type(&focused_elem))
                                .flatten();

                        // Only computes ink! attribute actions if the focus is on either a struct record field or
                        // an AST item's declaration (i.e not on attributes nor rustdoc nor inside the AST item's item list or body) for
                        // an item that can be annotated with ink! attributes.
                        if record_field.is_some()
                            || is_focused_on_item_declaration(&ast_item, range)
                        {
                            // Retrieves the target syntax node as either the covering struct field (if present) or
                            // the parent AST item (for all other cases).
                            let target = record_field
                                .as_ref()
                                .map_or(ast_item.syntax(), AstNode::syntax);

                            // Determines text range for item "declaration" (fallbacks to range of the entire item).
                            let item_declaration_text_range = record_field
                                .as_ref()
                                .map(|it| it.syntax().text_range())
                                .or(utils::ast_item_declaration_range(&ast_item))
                                .unwrap_or(ast_item.syntax().text_range());

                            // Suggests ink! attribute macros based on the context.
                            ink_macro_actions(results, target, item_declaration_text_range);

                            // Suggests ink! attribute arguments based on the context.
                            ink_arg_actions(results, target, item_declaration_text_range);

                            // Suggests actions for "flattening" ink! attributes (if any).
                            flatten_attrs(results, target, item_declaration_text_range);
                        }

                        // Only computes ink! entity actions if the focus is on either
                        // an AST item's "declaration" or body (except for record fields)
                        // (i.e not on meta - attributes/rustdoc) for an item that can can have ink! attribute descendants.
                        let is_focused_on_body = is_focused_on_item_body(&ast_item, range);
                        if is_focused_on_item_declaration(&ast_item, range)
                            || (is_focused_on_body && record_field.is_none())
                        {
                            // Suggests ink! entities based on item context.
                            item_ink_entity_actions(
                                results,
                                &ast_item,
                                is_focused_on_body.then_some(focused_elem_insert_offset()),
                            );
                        }
                    }
                    // Computes root-level ink! entity actions if focused element is whitespace in the root of the file (i.e. has no AST parent).
                    None => {
                        let is_in_file_root = focused_elem
                            .parent()
                            .map_or(false, |it| it.kind() == SyntaxKind::SOURCE_FILE);
                        if is_in_file_root {
                            // Suggests root-level ink! entities based on the context.
                            root_ink_entity_actions(results, file, focused_elem_insert_offset());
                        }
                    }
                }
            }
        }
        // Computes root-level ink! entity actions if file is empty.
        None => {
            if file.syntax().text_range().is_empty()
                && file.syntax().text_range().contains_range(range)
            {
                root_ink_entity_actions(results, file, range.end());
            }
        }
    }
}

/// Computes AST item-based ink! attribute macro actions.
fn ink_macro_actions(results: &mut Vec<Action>, target: &SyntaxNode, range: TextRange) {
    // Only suggest ink! attribute macros if the AST item has no other ink! attributes.
    if ink_analyzer_ir::ink_attrs(target).next().is_none() {
        // Suggests ink! attribute macros based on the context.
        let mut ink_macro_suggestions = utils::valid_ink_macros_by_syntax_kind(target.kind());

        // Filters out duplicate and invalid ink! attribute macro actions based on parent ink! scope (if any).
        utils::remove_duplicate_ink_macro_suggestions(&mut ink_macro_suggestions, target);
        utils::remove_invalid_ink_macro_suggestions_for_parent_ink_scope(
            &mut ink_macro_suggestions,
            target,
        );
        utils::remove_invalid_ink_macro_suggestions_for_parent_cfg_scope(
            &mut ink_macro_suggestions,
            target,
        );

        if !ink_macro_suggestions.is_empty() {
            // Determines the insertion offset and affixes for the action.
            let insert_offset = utils::ink_attribute_insert_offset(target);

            // Add ink! attribute macro actions to accumulator.
            for macro_kind in ink_macro_suggestions {
                results.push(Action {
                    label: format!("Add ink! {macro_kind} attribute macro."),
                    kind: ActionKind::Refactor,
                    range,
                    edits: vec![TextEdit::insert(
                        format!("#[{}]", macro_kind.path_as_str(),),
                        insert_offset,
                    )],
                });
            }
        }
    }
}

/// Computes AST item-based ink! attribute argument actions.
fn ink_arg_actions(results: &mut Vec<Action>, target: &SyntaxNode, range: TextRange) {
    // Gets the primary ink! attribute candidate (if any).
    let primary_ink_attr_candidate =
        utils::primary_ink_attribute_candidate(ink_analyzer_ir::ink_attrs(target))
            .map(|(attr, ..)| attr);

    // Suggests ink! attribute arguments based on the context.
    let mut ink_arg_suggestions = match primary_ink_attr_candidate.as_ref() {
        // Make suggestions based on the "primary" valid ink! attribute (if any).
        Some(ink_attr) => utils::valid_sibling_ink_args(*ink_attr.kind()),
        // Otherwise make suggestions based on the AST item's syntax kind.
        None => utils::valid_ink_args_by_syntax_kind(target.kind()),
    };

    // Filters out duplicate ink! attribute argument actions.
    utils::remove_duplicate_ink_arg_suggestions(&mut ink_arg_suggestions, target);
    // Filters out conflicting ink! attribute argument actions.
    utils::remove_conflicting_ink_arg_suggestions(&mut ink_arg_suggestions, target);
    // Filters out invalid ink! arguments from suggestions based on parent item's invariants.
    utils::remove_invalid_ink_arg_suggestions_for_parent_item(&mut ink_arg_suggestions, target);
    // Filters out invalid ink! attribute argument actions based on parent ink! scope
    // if there's either no valid ink! attribute macro or only ink! attribute arguments applied to the item.
    if primary_ink_attr_candidate.is_none()
        || !matches!(
            primary_ink_attr_candidate.as_ref().map(InkAttribute::kind),
            Some(InkAttributeKind::Macro(_))
        )
    {
        utils::remove_invalid_ink_arg_suggestions_for_parent_ink_scope(
            &mut ink_arg_suggestions,
            target,
        );
    }

    if !ink_arg_suggestions.is_empty() {
        // Add ink! attribute argument actions to accumulator.
        for arg_kind in ink_arg_suggestions {
            // Determines the insertion offset and affixes for the action and whether or not an existing attribute can be extended.
            let ((insert_offset, insert_prefix, insert_suffix), is_extending) =
                primary_ink_attr_candidate
                    .as_ref()
                    .and_then(|ink_attr| {
                        // Try to extend an existing attribute (if possible).
                        utils::ink_arg_insert_offset_and_affixes(ink_attr, Some(arg_kind)).map(
                            |(insert_offset, insert_prefix, insert_suffix)| {
                                ((insert_offset, insert_prefix, insert_suffix), true)
                            },
                        )
                    })
                    .unwrap_or((
                        // Fallback to inserting a new attribute.
                        (utils::ink_attribute_insert_offset(target), None, None),
                        false,
                    ));

            // Adds ink! attribute argument action to accumulator.
            let (edit, snippet) = utils::ink_arg_insert_text(
                arg_kind,
                Some(insert_offset),
                is_extending
                    .then(|| {
                        primary_ink_attr_candidate
                            .as_ref()
                            .map(InkAttribute::syntax)
                    })
                    .flatten(),
            );
            results.push(Action {
                label: format!("Add ink! {arg_kind} attribute argument."),
                kind: ActionKind::Refactor,
                range: is_extending
                    .then(|| {
                        primary_ink_attr_candidate
                            .as_ref()
                            .map(|it| it.syntax().text_range())
                    })
                    .flatten()
                    .unwrap_or(range),
                edits: vec![TextEdit::insert_with_snippet(
                    format!(
                        "{}{}{}",
                        insert_prefix.unwrap_or_default(),
                        if is_extending {
                            edit
                        } else {
                            format!("#[ink({edit})]")
                        },
                        insert_suffix.unwrap_or_default(),
                    ),
                    insert_offset,
                    snippet.map(|snippet| {
                        format!(
                            "{}{}{}",
                            insert_prefix.unwrap_or_default(),
                            if is_extending {
                                snippet
                            } else {
                                format!("#[ink({snippet})]")
                            },
                            insert_suffix.unwrap_or_default(),
                        )
                    }),
                )],
            });
        }
    }
}

/// Computes AST item-based ink! entity macro actions.
fn item_ink_entity_actions(
    results: &mut Vec<Action>,
    item: &ast::Item,
    insert_offset_option: Option<TextSize>,
) {
    let mut add_result = |action_option: Option<Action>| {
        // Add action to accumulator (if any).
        if let Some(action) = action_option {
            results.push(action);
        }
    };
    match item {
        ast::Item::Module(module) => {
            match ink_analyzer_ir::ink_attrs(module.syntax())
                .find(|attr| *attr.kind() == InkAttributeKind::Macro(InkMacroKind::Contract))
                .and_then(Contract::cast)
            {
                Some(contract) => {
                    // Adds ink! storage if it doesn't exist.
                    if contract.storage().is_none() {
                        add_result(entity::add_storage(
                            &contract,
                            ActionKind::Refactor,
                            insert_offset_option,
                        ));
                    }

                    // Adds ink! event.
                    add_result(entity::add_event(
                        &contract,
                        ActionKind::Refactor,
                        insert_offset_option,
                    ));

                    // Adds ink! constructor.
                    add_result(entity::add_constructor_to_contract(
                        &contract,
                        ActionKind::Refactor,
                        insert_offset_option,
                    ));

                    // Adds ink! message.
                    add_result(entity::add_message_to_contract(
                        &contract,
                        ActionKind::Refactor,
                        insert_offset_option,
                    ));
                }
                None => {
                    let is_cfg_test = module.attrs().any(|attr| utils::is_cfg_test_attr(&attr));
                    if is_cfg_test {
                        // Adds ink! test.
                        add_result(entity::add_ink_test(
                            module,
                            ActionKind::Refactor,
                            insert_offset_option,
                        ));
                    }

                    let is_cfg_e2e_tests = module
                        .attrs()
                        .any(|attr| utils::is_cfg_e2e_tests_attr(&attr));
                    if is_cfg_e2e_tests {
                        // Adds ink! e2e test.
                        add_result(entity::add_ink_e2e_test(
                            module,
                            ActionKind::Refactor,
                            insert_offset_option,
                        ));
                    }
                }
            }
        }
        ast::Item::Impl(impl_item) => {
            // Only computes ink! entities if impl item is not a trait `impl` and additionally either:
            // - has an ink! `impl` attribute.
            // - contains at least one ink! constructor or ink! message.
            // - has an ink! contract as the direct parent.
            if impl_item.trait_().is_none()
                && (InkImpl::can_cast(impl_item.syntax())
                    || ink_analyzer_ir::ink_parent::<Contract>(impl_item.syntax()).is_some())
            {
                // Adds ink! constructor.
                add_result(entity::add_constructor_to_impl(
                    impl_item,
                    ActionKind::Refactor,
                    insert_offset_option,
                ));

                // Adds ink! message.
                add_result(entity::add_message_to_impl(
                    impl_item,
                    ActionKind::Refactor,
                    insert_offset_option,
                ));
            }
        }
        ast::Item::Trait(trait_item) => {
            if let Some((attr, _)) = utils::primary_ink_attribute_candidate(
                ink_analyzer_ir::ink_attrs(trait_item.syntax()),
            ) {
                if let InkAttributeKind::Macro(macro_kind) = attr.kind() {
                    match macro_kind {
                        InkMacroKind::ChainExtension => {
                            if let Some(chain_extension) = ChainExtension::cast(attr) {
                                // Add `ErrorCode` if it doesn't exist.
                                if chain_extension.error_code().is_none() {
                                    add_result(entity::add_error_code(
                                        &chain_extension,
                                        ActionKind::Refactor,
                                        insert_offset_option,
                                    ));
                                }

                                // Adds ink! extension.
                                add_result(entity::add_extension(
                                    &chain_extension,
                                    ActionKind::Refactor,
                                    insert_offset_option,
                                ));
                            }
                        }
                        InkMacroKind::TraitDefinition => {
                            if let Some(trait_definition) = TraitDefinition::cast(attr) {
                                // Adds ink! message declaration.
                                add_result(entity::add_message_to_trait_definition(
                                    &trait_definition,
                                    ActionKind::Refactor,
                                    insert_offset_option,
                                ));
                            }
                        }
                        // Ignores other macros.
                        _ => (),
                    }
                }
            }
        }
        ast::Item::Struct(struct_item) => {
            if let Some(event) = ink_analyzer_ir::ink_attrs(struct_item.syntax())
                .find(|attr| *attr.kind() == InkAttributeKind::Arg(InkArgKind::Event))
                .and_then(Event::cast)
            {
                // Adds ink! topic.
                add_result(entity::add_topic(
                    &event,
                    ActionKind::Refactor,
                    insert_offset_option,
                ));
            }
        }
        // Ignores other items.
        _ => (),
    }
}

/// Computes root-level ink! entity macro actions.
fn root_ink_entity_actions(results: &mut Vec<Action>, file: &InkFile, offset: TextSize) {
    if file.contracts().is_empty() {
        // Adds ink! contract.
        results.push(entity::add_contract(offset, ActionKind::Refactor, None));
    }

    // Adds ink! trait definition.
    results.push(entity::add_trait_definition(
        offset,
        ActionKind::Refactor,
        None,
    ));

    // Adds ink! chain extension.
    results.push(entity::add_chain_extension(
        offset,
        ActionKind::Refactor,
        None,
    ));

    // Adds ink! storage item.
    results.push(entity::add_storage_item(offset, ActionKind::Refactor, None));
}

/// Computes actions for "flattening" ink! attributes for the target syntax node.
fn flatten_attrs(results: &mut Vec<Action>, target: &SyntaxNode, range: TextRange) {
    let mut attrs = ink_analyzer_ir::ink_attrs(target).sorted();
    if let Some(primary_candidate) = attrs.next() {
        // Only computes flattening actions if the item has other argument-based ink! attributes.
        let other_arg_attrs = attrs.filter(|attr| matches!(attr.kind(), InkAttributeKind::Arg(_)));
        if other_arg_attrs.clone().next().is_some() {
            results.push(Action {
                label: "Flatten ink! attribute arguments.".to_string(),
                kind: ActionKind::Refactor,
                range,
                edits: [TextEdit::replace(
                    format!(
                        "#[{}({})]",
                        match primary_candidate.kind() {
                            InkAttributeKind::Macro(macro_kind) => macro_kind.path_as_str(),
                            InkAttributeKind::Arg(_) => "ink",
                        },
                        // All ink! attribute arguments sorted by priority.
                        primary_candidate
                            .args()
                            .iter()
                            .cloned()
                            .chain(
                                other_arg_attrs
                                    .clone()
                                    .flat_map(|attr| attr.args().to_vec())
                            )
                            .sorted()
                            .map(|arg| arg.to_string())
                            .join(", ")
                    ),
                    primary_candidate.syntax().text_range(),
                )]
                .into_iter()
                // Removes other argument-based ink! attributes.
                .chain(other_arg_attrs.map(|attr| TextEdit::delete(attr.syntax().text_range())))
                .collect(),
            });
        }
    }
}

/// Determines if the selection range is in an AST item's declaration
/// (i.e not on meta - attributes/rustdoc - nor inside the AST item's item list or body)
/// for an item that can be annotated with ink! attributes or can have ink! attribute descendants.
fn is_focused_on_item_declaration(item: &ast::Item, range: TextRange) -> bool {
    // Returns false for "unsupported" item types (see [`utils::ast_item_declaration_range`] doc and implementation).
    utils::ast_item_declaration_range(item).map_or(false, |declaration_range| {
        declaration_range.contains_range(range)
    }) || utils::ast_item_terminal_token(item)
        .map_or(false, |token| token.text_range().contains_range(range))
}

/// Determines if the selection range is in an AST item's body (i.e inside the AST item's item list or body)
/// for an item that can be annotated with ink! attributes or can have ink! attribute descendants.
fn is_focused_on_item_body(item: &ast::Item, range: TextRange) -> bool {
    // Returns false for "unsupported" item types (see [`utils::ast_item_declaration_range`] doc and implementation).
    utils::ast_item_declaration_range(item)
        .zip(
            utils::ast_item_terminal_token(item)
                .as_ref()
                .map(SyntaxToken::text_range),
        )
        .map_or(false, |(declaration_range, terminal_range)| {
            // Verifies that
            declaration_range.end() < terminal_range.start()
                && TextRange::new(declaration_range.end(), terminal_range.start())
                    .contains_range(range)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::verify_actions;
    use ink_analyzer_ir::syntax::TextSize;
    use ink_analyzer_ir::FromSyntax;
    use test_utils::{parse_offset_at, TestResultAction, TestResultTextRange};

    #[test]
    fn actions_works() {
        for (code, pat, expected_results) in [
            // (code, pat, Vec<(label, Vec<(text, start_pat, end_pat)>)>) where:
            // code = source code,
            // pat = substring used to find the cursor offset (see `test_utils::parse_offset_at` doc),
            // label = the label text (of a substring of it) for the action,
            // edit = the text (of a substring of it) that will inserted,
            // start_pat = substring used to find the start of the edit offset (see `test_utils::parse_offset_at` doc),
            // end_pat = substring used to find the end of the edit offset (see `test_utils::parse_offset_at` doc).

            // No AST item in focus.
            (
                "",
                None,
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::contract]",
                            start_pat: None,
                            end_pat: None,
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::trait_definition]",
                            start_pat: None,
                            end_pat: None,
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::chain_extension]",
                            start_pat: None,
                            end_pat: None,
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::storage_item]",
                            start_pat: None,
                            end_pat: None,
                        }],
                    },
                ],
            ),
            (
                " ",
                None,
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::contract]",
                            start_pat: Some(" "),
                            end_pat: Some(" "),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::trait_definition]",
                            start_pat: Some(" "),
                            end_pat: Some(" "),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::chain_extension]",
                            start_pat: Some(" "),
                            end_pat: Some(" "),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::storage_item]",
                            start_pat: Some(" "),
                            end_pat: Some(" "),
                        }],
                    },
                ],
            ),
            (
                "\n\n",
                Some("\n"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::contract]",
                            start_pat: Some("\n"),
                            end_pat: Some("\n"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::trait_definition]",
                            start_pat: Some("\n"),
                            end_pat: Some("\n"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::chain_extension]",
                            start_pat: Some("\n"),
                            end_pat: Some("\n"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::storage_item]",
                            start_pat: Some("\n"),
                            end_pat: Some("\n"),
                        }],
                    },
                ],
            ),
            (
                "// A comment in focus.",
                None,
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::contract]",
                            start_pat: Some("// A comment in focus."),
                            end_pat: Some("// A comment in focus."),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::trait_definition]",
                            start_pat: Some("// A comment in focus."),
                            end_pat: Some("// A comment in focus."),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::chain_extension]",
                            start_pat: Some("// A comment in focus."),
                            end_pat: Some("// A comment in focus."),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::storage_item]",
                            start_pat: Some("// A comment in focus."),
                            end_pat: Some("// A comment in focus."),
                        }],
                    },
                ],
            ),
            // Module focus.
            (
                r#"
                    mod my_module {

                    }
                "#,
                Some("<-\n                    }"),
                vec![],
            ),
            (
                r#"
                    mod my_module {
                        // The module declaration is out of focus when this comment is in focus.
                    }
                "#,
                Some("<-//"),
                vec![],
            ),
            (
                r#"
                    mod my_contract {
                    }
                "#,
                Some("<-mod"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    mod my_contract {
                    }
                "#,
                Some("my_con"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    mod my_contract {
                    }
                "#,
                Some("<-{"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    mod my_contract {
                    }
                "#,
                Some("{"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    mod my_contract {
                    }
                "#,
                Some("}"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    mod my_contract {
                    }
                "#,
                Some("<-}"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    #[foo]
                    mod my_contract {
                    }
                "#,
                Some("<-mod"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::contract]",
                        start_pat: Some("<-mod"),
                        end_pat: Some("<-mod"),
                    }],
                }],
            ),
            (
                r#"
                    #[ink::contract]
                    mod my_contract {
                    }
                "#,
                Some("<-mod"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "(env = crate::)",
                            start_pat: Some("#[ink::contract"),
                            end_pat: Some("#[ink::contract"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"(keep_attr = "")"#,
                            start_pat: Some("#[ink::contract"),
                            end_pat: Some("#[ink::contract"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(storage)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(event)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink::contract]
                    mod my_contract {

                    }
                "#,
                Some("<-\n                    }"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(storage)]",
                            start_pat: Some("<-\n                    }"),
                            end_pat: Some("<-\n                    }"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(event)]",
                            start_pat: Some("<-\n                    }"),
                            end_pat: Some("<-\n                    }"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("<-\n                    }"),
                            end_pat: Some("<-\n                    }"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("<-\n                    }"),
                            end_pat: Some("<-\n                    }"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink::contract]
                    #[ink(env=crate::Environment)]
                    #[ink(keep_attr="foo,bar")]
                    mod my_contract {
                    }
                "#,
                Some("<-mod"),
                vec![
                    TestResultAction {
                        label: "Flatten",
                        edits: vec![
                            TestResultTextRange {
                                text: r#"#[ink::contract(env = crate::Environment, keep_attr = "foo,bar")]"#,
                                start_pat: Some("<-#[ink::contract]"),
                                end_pat: Some("#[ink::contract]"),
                            },
                            TestResultTextRange {
                                text: "",
                                start_pat: Some(r#"<-#[ink(env=crate::Environment)]"#),
                                end_pat: Some(r#"#[ink(env=crate::Environment)]"#),
                            },
                            TestResultTextRange {
                                text: "",
                                start_pat: Some(r#"<-#[ink(keep_attr="foo,bar")]"#),
                                end_pat: Some(r#"#[ink(keep_attr="foo,bar")]"#),
                            },
                        ],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(storage)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(event)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("mod my_contract {"),
                            end_pat: Some("mod my_contract {"),
                        }],
                    },
                ],
            ),
            // Trait focus.
            (
                r#"
                    pub trait MyTrait {
                    }
                "#,
                Some("<-pub"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::chain_extension]",
                            start_pat: Some("<-pub"),
                            end_pat: Some("<-pub"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::trait_definition]",
                            start_pat: Some("<-pub"),
                            end_pat: Some("<-pub"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink::chain_extension]
                    pub trait MyTrait {
                    }
                "#,
                Some("<-pub"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "type ErrorCode",
                            start_pat: Some("pub trait MyTrait {"),
                            end_pat: Some("pub trait MyTrait {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(extension = 1)]",
                            start_pat: Some("pub trait MyTrait {"),
                            end_pat: Some("pub trait MyTrait {"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink::trait_definition]
                    pub trait MyTrait {
                    }
                "#,
                Some("<-pub"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"(keep_attr = "")"#,
                            start_pat: Some("#[ink::trait_definition"),
                            end_pat: Some("#[ink::trait_definition"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"(namespace = "my_namespace")"#,
                            start_pat: Some("#[ink::trait_definition"),
                            end_pat: Some("#[ink::trait_definition"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("pub trait MyTrait {"),
                            end_pat: Some("pub trait MyTrait {"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink::trait_definition]
                    #[ink(namespace="my_namespace")]
                    #[ink(keep_attr="foo,bar")]
                    pub trait MyTrait {
                    }
                "#,
                Some("<-pub"),
                vec![
                    TestResultAction {
                        label: "Flatten",
                        edits: vec![
                            TestResultTextRange {
                                text: r#"#[ink::trait_definition(namespace = "my_namespace", keep_attr = "foo,bar")]"#,
                                start_pat: Some("<-#[ink::trait_definition]"),
                                end_pat: Some("#[ink::trait_definition]"),
                            },
                            TestResultTextRange {
                                text: "",
                                start_pat: Some(r#"<-#[ink(namespace="my_namespace")]"#),
                                end_pat: Some(r#"#[ink(namespace="my_namespace")]"#),
                            },
                            TestResultTextRange {
                                text: "",
                                start_pat: Some(r#"<-#[ink(keep_attr="foo,bar")]"#),
                                end_pat: Some(r#"#[ink(keep_attr="foo,bar")]"#),
                            },
                        ],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("pub trait MyTrait {"),
                            end_pat: Some("pub trait MyTrait {"),
                        }],
                    },
                ],
            ),
            // ADT focus.
            (
                r#"
                    enum MyEnum {
                    }
                "#,
                Some("<-enum"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::storage_item]",
                        start_pat: Some("<-enum"),
                        end_pat: Some("<-enum"),
                    }],
                }],
            ),
            (
                r#"
                    struct MyStruct {
                    }
                "#,
                Some("<-struct"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::storage_item]",
                            start_pat: Some("<-struct"),
                            end_pat: Some("<-struct"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(anonymous)]",
                            start_pat: Some("<-struct"),
                            end_pat: Some("<-struct"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(event)]",
                            start_pat: Some("<-struct"),
                            end_pat: Some("<-struct"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(storage)]",
                            start_pat: Some("<-struct"),
                            end_pat: Some("<-struct"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    union MyUnion {
                    }
                "#,
                Some("<-union"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink::storage_item]",
                        start_pat: Some("<-union"),
                        end_pat: Some("<-union"),
                    }],
                }],
            ),
            (
                r#"
                    #[ink::storage_item]
                    #[ink(derive=true)]
                    struct MyStruct {
                    }
                "#,
                Some("<-struct"),
                vec![TestResultAction {
                    label: "Flatten",
                    edits: vec![
                        TestResultTextRange {
                            text: "#[ink::storage_item(derive = true)]",
                            start_pat: Some("<-#[ink::storage_item]"),
                            end_pat: Some("#[ink::storage_item]"),
                        },
                        TestResultTextRange {
                            text: "",
                            start_pat: Some("<-#[ink(derive=true)]"),
                            end_pat: Some("#[ink(derive=true)]"),
                        },
                    ],
                }],
            ),
            (
                r#"
                    #[ink(event)]
                    struct MyEvent {
                    }
                "#,
                Some("<-struct"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: ", anonymous",
                            start_pat: Some("#[ink(event"),
                            end_pat: Some("#[ink(event"),
                        }],
                    },
                    // Adds ink! topic `field`.
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(topic)]",
                            start_pat: Some("struct MyEvent {"),
                            end_pat: Some("struct MyEvent {"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink(anonymous)]
                    struct MyEvent {
                    }
                "#,
                Some("<-struct"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "event, ",
                        start_pat: Some("#[ink("),
                        end_pat: Some("#[ink("),
                    }],
                }],
            ),
            (
                r#"
                    #[ink(event, anonymous)]
                    struct MyEvent {
                        my_field: u8,
                    }
                "#,
                Some("<-struct"),
                vec![
                    // Adds ink! topic `field`.
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(topic)]",
                            start_pat: Some("my_field: u8,"),
                            end_pat: Some("my_field: u8,"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink(anonymous)]
                    #[ink(event)]
                    struct MyEvent {
                        my_field: u8,
                    }
                "#,
                Some("<-struct"),
                vec![
                    TestResultAction {
                        label: "Flatten",
                        edits: vec![
                            TestResultTextRange {
                                text: "#[ink(event, anonymous)]",
                                start_pat: Some("<-#[ink(event)]"),
                                end_pat: Some("#[ink(event)]"),
                            },
                            TestResultTextRange {
                                text: "",
                                start_pat: Some("<-#[ink(anonymous)]"),
                                end_pat: Some("#[ink(anonymous)]"),
                            },
                        ],
                    },
                    // Adds ink! topic `field`.
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(topic)]",
                            start_pat: Some("my_field: u8,"),
                            end_pat: Some("my_field: u8,"),
                        }],
                    },
                ],
            ),
            // Struct field focus.
            (
                r#"
                    struct MyStruct {
                        value: bool,
                    }
                "#,
                Some("<-value"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink(topic)]",
                        start_pat: Some("<-value"),
                        end_pat: Some("<-value"),
                    }],
                }],
            ),
            (
                r#"
                    #[ink(event, anonymous)]
                    struct MyEvent {
                        my_field: u8,
                    }
                "#,
                Some("<-my_field"),
                vec![
                    // Adds ink! topic attribute argument.
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(topic)]",
                            start_pat: Some("<-my_field"),
                            end_pat: Some("<-my_field"),
                        }],
                    },
                ],
            ),
            // Fn focus.
            (
                r#"
                    fn my_fn() {
                    }
                "#,
                Some("<-fn"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(default)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(extension = 1)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(handle_status = true)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(payable)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(selector = 1)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[cfg(test)]
                    mod my_mod {
                        fn my_fn() {
                        }
                    }
                "#,
                Some("<-fn"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::test]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(default)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(extension = 1)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(handle_status = true)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(payable)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(selector = 1)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[cfg(all(test, feature="e2e-tests"))]
                    mod my_mod {
                        fn my_fn() {
                        }
                    }
                "#,
                Some("<-fn"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink::test]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink_e2e::test]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(default)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(extension = 1)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(handle_status = true)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(payable)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(selector = 1)]",
                            start_pat: Some("<-fn"),
                            end_pat: Some("<-fn"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink::test]
                    fn my_fn() {
                    }
                "#,
                Some("<-fn"),
                vec![],
            ),
            (
                r#"
                    #[ink_e2e::test]
                    fn my_fn() {
                    }
                "#,
                Some("<-fn"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"(additional_contracts = "")"#,
                            start_pat: Some("#[ink_e2e::test"),
                            end_pat: Some("#[ink_e2e::test"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "(environment = crate::)",
                            start_pat: Some("#[ink_e2e::test"),
                            end_pat: Some("#[ink_e2e::test"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"(keep_attr = "")"#,
                            start_pat: Some("#[ink_e2e::test"),
                            end_pat: Some("#[ink_e2e::test"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink_e2e::test]
                    #[ink(additional_contracts="")]
                    #[ink(environment=crate::)]
                    #[ink(keep_attr="")]
                    fn my_fn() {
                    }
                "#,
                Some("<-fn"),
                vec![TestResultAction {
                    label: "Flatten",
                    edits: vec![
                        TestResultTextRange {
                            text: r#"#[ink_e2e::test(additional_contracts = "", environment = crate::, keep_attr = "")]"#,
                            start_pat: Some("<-#[ink_e2e::test]"),
                            end_pat: Some("#[ink_e2e::test]"),
                        },
                        TestResultTextRange {
                            text: "",
                            start_pat: Some(r#"<-#[ink(additional_contracts="")]"#),
                            end_pat: Some(r#"#[ink(additional_contracts="")]"#),
                        },
                        TestResultTextRange {
                            text: "",
                            start_pat: Some(r#"<-#[ink(environment=crate::)]"#),
                            end_pat: Some(r#"#[ink(environment=crate::)]"#),
                        },
                        TestResultTextRange {
                            text: "",
                            start_pat: Some(r#"<-#[ink(keep_attr="")]"#),
                            end_pat: Some(r#"#[ink(keep_attr="")]"#),
                        },
                    ],
                }],
            ),
            (
                r#"
                    #[ink(constructor)]
                    fn my_fn() {
                    }
                "#,
                Some("<-fn"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: ", default",
                            start_pat: Some("#[ink(constructor"),
                            end_pat: Some("#[ink(constructor"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: ", payable",
                            start_pat: Some("#[ink(constructor"),
                            end_pat: Some("#[ink(constructor"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: ", selector = 1",
                            start_pat: Some("#[ink(constructor"),
                            end_pat: Some("#[ink(constructor"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink(constructor)]
                    #[ink(selector=1)]
                    #[ink(default, payable)]
                    fn my_fn() {
                    }
                "#,
                Some("<-fn"),
                vec![TestResultAction {
                    label: "Flatten",
                    edits: vec![
                        TestResultTextRange {
                            text: "#[ink(constructor, selector = 1, default, payable)]",
                            start_pat: Some("<-#[ink(constructor)]"),
                            end_pat: Some("#[ink(constructor)]"),
                        },
                        TestResultTextRange {
                            text: "",
                            start_pat: Some("<-#[ink(selector=1)]"),
                            end_pat: Some("#[ink(selector=1)]"),
                        },
                        TestResultTextRange {
                            text: "",
                            start_pat: Some("<-#[ink(default, payable)]"),
                            end_pat: Some("#[ink(default, payable)]"),
                        },
                    ],
                }],
            ),
            // impl focus.
            (
                r#"
                    impl MyContract {
                    }
                "#,
                Some("<-impl MyContract {"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(impl)]",
                            start_pat: Some("<-impl MyContract {"),
                            end_pat: Some("<-impl MyContract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"#[ink(namespace = "my_namespace")]"#,
                            start_pat: Some("<-impl MyContract {"),
                            end_pat: Some("<-impl MyContract {"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    #[ink(impl)]
                    impl MyContract {
                    }
                "#,
                Some("<-impl MyContract {"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#", namespace = "my_namespace""#,
                            start_pat: Some("#[ink(impl"),
                            end_pat: Some("#[ink(impl"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("impl MyContract {"),
                            end_pat: Some("impl MyContract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("impl MyContract {"),
                            end_pat: Some("impl MyContract {"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    impl MyContract {
                        #[ink(constructor)]
                        pub fn new() -> Self {}
                    }
                "#,
                Some("<-impl MyContract {"),
                vec![
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(impl)]",
                            start_pat: Some("<-impl MyContract {"),
                            end_pat: Some("<-impl MyContract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: r#"#[ink(namespace = "my_namespace")]"#,
                            start_pat: Some("<-impl MyContract {"),
                            end_pat: Some("<-impl MyContract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("pub fn new() -> Self {}"),
                            end_pat: Some("pub fn new() -> Self {}"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("pub fn new() -> Self {}"),
                            end_pat: Some("pub fn new() -> Self {}"),
                        }],
                    },
                ],
            ),
            (
                r#"
                    impl MyTrait for MyContract {
                        #[ink(constructor)]
                        pub fn new() -> Self {}
                    }
                "#,
                Some("<-impl MyTrait for MyContract {"),
                vec![TestResultAction {
                    label: "Add",
                    edits: vec![TestResultTextRange {
                        text: "#[ink(impl)]",
                        start_pat: Some("<-impl MyTrait for MyContract {"),
                        end_pat: Some("<-impl MyTrait for MyContract {"),
                    }],
                }],
            ),
            (
                r#"
                    #[ink(impl)]
                    #[ink(namespace="my_namespace")]
                    impl MyContract {
                    }
                "#,
                Some("<-impl MyContract {"),
                vec![
                    TestResultAction {
                        label: "Flatten",
                        edits: vec![
                            TestResultTextRange {
                                text: r#"#[ink(impl, namespace = "my_namespace")]"#,
                                start_pat: Some("<-#[ink(impl)]"),
                                end_pat: Some("#[ink(impl)]"),
                            },
                            TestResultTextRange {
                                text: "",
                                start_pat: Some(r#"<-#[ink(namespace="my_namespace")]"#),
                                end_pat: Some(r#"#[ink(namespace="my_namespace")]"#),
                            },
                        ],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(constructor)]",
                            start_pat: Some("impl MyContract {"),
                            end_pat: Some("impl MyContract {"),
                        }],
                    },
                    TestResultAction {
                        label: "Add",
                        edits: vec![TestResultTextRange {
                            text: "#[ink(message)]",
                            start_pat: Some("impl MyContract {"),
                            end_pat: Some("impl MyContract {"),
                        }],
                    },
                ],
            ),
        ] {
            let offset = TextSize::from(parse_offset_at(code, pat).unwrap() as u32);
            let range = TextRange::new(offset, offset);

            let mut results = Vec::new();
            actions(&mut results, &InkFile::parse(code), range);

            // Verifies actions.
            verify_actions(code, &results, &expected_results);
        }
    }

    #[test]
    fn is_focused_on_item_declaration_and_body_works() {
        for (code, test_cases) in [
            // (code, [(pat, declaration_result, body_result)]) where:
            // code = source code,
            // pat = substring used to find the cursor offset (see `test_utils::parse_offset_at` doc),
            // result = expected result from calling `is_focused_on_ast_item_declaration` (i.e whether or not an AST item's declaration is in focus),

            // Module.
            (
                r#"
                    #[abc]
                    #[ink::contract]
                    mod my_module {
                        // The module declaration is out of focus when this comment is in focus.

                    }
                "#,
                vec![
                    (Some("<-#[a"), false, false),
                    (Some("#[ab"), false, false),
                    (Some("abc]"), false, false),
                    (Some("<-#[ink"), false, false),
                    (Some("#[in"), false, false),
                    (Some("ink::"), false, false),
                    (Some("::con"), false, false),
                    (Some("contract]"), false, false),
                    (Some("<-mod"), true, false),
                    (Some("mo"), true, false),
                    (Some("mod"), true, false),
                    (Some("<-my_module"), true, false),
                    (Some("my_"), true, false),
                    (Some("<-my_module"), true, false),
                    (Some("<-{"), true, false),
                    (Some("{"), true, true),
                    (Some("<-//"), false, true),
                    (Some("<-\n                    }"), false, true),
                    (Some("<-}"), true, true),
                    (Some("}"), true, false),
                ],
            ),
            // Trait.
            (
                r#"
                    #[abc]
                    #[ink::trait_definition]
                    pub trait MyTrait {
                        // The trait declaration is out of focus when this comment is in focus.
                    }
                "#,
                vec![
                    (Some("<-#[a"), false, false),
                    (Some("#[ab"), false, false),
                    (Some("abc]"), false, false),
                    (Some("<-#[ink"), false, false),
                    (Some("#[in"), false, false),
                    (Some("ink::"), false, false),
                    (Some("::trait"), false, false),
                    (Some("definition]"), false, false),
                    (Some("<-pub"), true, false),
                    (Some("pu"), true, false),
                    (Some("pub"), true, false),
                    (Some("<-trait MyTrait"), true, false),
                    (Some("pub tr"), true, false),
                    (Some("pub trait"), true, false),
                    (Some("<-MyTrait"), true, false),
                    (Some("My"), true, false),
                    (Some("<-MyTrait"), true, false),
                    (Some("<-{"), true, false),
                    (Some("{"), true, true),
                    (Some("<-//"), false, true),
                    (Some("<-}"), true, true),
                    (Some("}"), true, false),
                ],
            ),
            // Enum.
            (
                r#"
                    #[abc]
                    #[ink::storage_item]
                    pub enum MyEnum {
                        // The enum declaration is out of focus when this comment is in focus.
                    }
                "#,
                vec![
                    (Some("<-#[a"), false, false),
                    (Some("#[ab"), false, false),
                    (Some("abc]"), false, false),
                    (Some("<-#[ink"), false, false),
                    (Some("#[in"), false, false),
                    (Some("ink::"), false, false),
                    (Some("::storage"), false, false),
                    (Some("storage_item]"), false, false),
                    (Some("<-pub"), true, false),
                    (Some("pu"), true, false),
                    (Some("pub"), true, false),
                    (Some("<-enum"), true, false),
                    (Some("en"), true, false),
                    (Some("enum"), true, false),
                    (Some("<-MyEnum"), true, false),
                    (Some("My"), true, false),
                    (Some("<-MyEnum"), true, false),
                    (Some("<-{"), true, false),
                    (Some("{"), true, true),
                    (Some("<-//"), false, true),
                    (Some("<-}"), true, true),
                    (Some("}"), true, false),
                ],
            ),
            // Struct.
            (
                r#"
                    #[abc]
                    #[ink(event, anonymous)]
                    pub struct MyStruct {
                        // The struct declaration is out of focus when this comment is in focus.
                    }
                "#,
                vec![
                    (Some("<-#[a"), false, false),
                    (Some("#[ab"), false, false),
                    (Some("abc]"), false, false),
                    (Some("<-#[ink"), false, false),
                    (Some("#[in"), false, false),
                    (Some("ink("), false, false),
                    (Some("(eve"), false, false),
                    (Some("(event,"), false, false),
                    (Some(", anon"), false, false),
                    (Some("anonymous)]"), false, false),
                    (Some("<-pub"), true, false),
                    (Some("pu"), true, false),
                    (Some("pub"), true, false),
                    (Some("<-struct"), true, false),
                    (Some("st"), true, false),
                    (Some("struct"), true, false),
                    (Some("<-MyStruct"), true, false),
                    (Some("My"), true, false),
                    (Some("<-MyStruct"), true, false),
                    (Some("<-{"), true, false),
                    (Some("{"), true, true),
                    (Some("<-//"), false, true),
                    (Some("<-}"), true, true),
                    (Some("}"), true, false),
                ],
            ),
            // Union.
            (
                r#"
                    #[abc]
                    #[ink::storage_item]
                    pub union MyUnion {
                        // The union declaration is out of focus when this comment is in focus.
                    }
                "#,
                vec![
                    (Some("<-#[a"), false, false),
                    (Some("#[ab"), false, false),
                    (Some("abc]"), false, false),
                    (Some("<-#[ink"), false, false),
                    (Some("#[in"), false, false),
                    (Some("ink::"), false, false),
                    (Some("::storage"), false, false),
                    (Some("storage_item]"), false, false),
                    (Some("<-pub"), true, false),
                    (Some("pu"), true, false),
                    (Some("pub"), true, false),
                    (Some("<-union"), true, false),
                    (Some("un"), true, false),
                    (Some("union"), true, false),
                    (Some("<-MyUnion"), true, false),
                    (Some("My"), true, false),
                    (Some("<-MyUnion"), true, false),
                    (Some("<-{"), true, false),
                    (Some("{"), true, true),
                    (Some("<-//"), false, true),
                    (Some("<-}"), true, true),
                    (Some("}"), true, false),
                ],
            ),
            // Fn.
            (
                r#"
                    #[abc]
                    #[ink(constructor, selector=1)]
                    #[ink(payable)]
                    pub fn my_fn() {
                        // The fn declaration is out of focus when this comment is in focus.
                    }
                "#,
                vec![
                    (Some("<-#[a"), false, false),
                    (Some("#[ab"), false, false),
                    (Some("abc]"), false, false),
                    (Some("<-#[ink"), false, false),
                    (Some("#[in"), false, false),
                    (Some("ink("), false, false),
                    (Some("(con"), false, false),
                    (Some("(constructor,"), false, false),
                    (Some(", select"), false, false),
                    (Some("selector=1)]"), false, false),
                    (Some("(pay"), false, false),
                    (Some("payable)]"), false, false),
                    (Some("<-pub"), true, false),
                    (Some("pu"), true, false),
                    (Some("pub"), true, false),
                    (Some("<-fn"), true, false),
                    (Some("f"), true, false),
                    (Some("fn"), true, false),
                    (Some("<-my_fn"), true, false),
                    (Some("my_"), true, false),
                    (Some("<-my_fn"), true, false),
                    (Some("<-{"), true, false),
                    (Some("{"), true, true),
                    (Some("<-//"), false, true),
                    (Some("<-}"), true, true),
                    (Some("}"), true, false),
                ],
            ),
        ] {
            for (pat, expected_declaration_result, expected_body_result) in test_cases {
                let offset = TextSize::from(parse_offset_at(code, pat).unwrap() as u32);
                let range = TextRange::new(offset, offset);

                let ast_item = InkFile::parse(code)
                    .syntax()
                    .descendants()
                    .filter_map(ast::Item::cast)
                    .next()
                    .unwrap();
                assert_eq!(
                    is_focused_on_item_declaration(&ast_item, range),
                    expected_declaration_result,
                    "code: {code} | {:#?}",
                    pat
                );
                assert_eq!(
                    is_focused_on_item_body(&ast_item, range),
                    expected_body_result,
                    "code: {code} | {:#?}",
                    pat
                );
            }
        }
    }
}
