//! ink! attribute IR.

use ink_analyzer_macro::FromAST;
use itertools::Itertools;
use ra_ap_syntax::{ast, AstNode, Direction, SyntaxNode};
use std::cmp::Ordering;
use std::fmt;

use crate::traits::{FromAST, FromSyntax};

use crate::meta::MetaName;
pub use arg::{InkArg, InkArgKind, InkArgValueKind, InkArgValuePathKind, InkArgValueStringKind};

mod arg;
pub mod meta;
pub mod utils;

/// An ink! specific attribute.
#[derive(Debug, Clone, PartialEq, Eq, FromAST)]
pub struct InkAttribute {
    /// The kind of the ink! attribute e.g attribute macro like `#[ink::contract]`
    /// or attribute argument like `#[ink(storage)]`.
    kind: InkAttributeKind,
    /// ink! attribute arguments e.g message, payable, selector = 1
    /// for `#[ink(message, payable, selector = 1)]`
    args: Vec<InkArg>,
    /// AST Node for ink! attribute.
    ast: ast::Attr,
    /// ink! path segment node.
    ink: ast::PathSegment,
    /// ink! macro path segment node (if any) from which the attribute macro kind is derived.
    ink_macro: Option<ast::PathSegment>,
    /// ink! argument name (if any) from which the attribute argument kind is derived.
    ink_arg_name: Option<MetaName>,
}

impl InkAttribute {
    /// Converts an AST attribute (`Attr`) into an `InkAttribute` IR type.
    pub fn cast(attr: ast::Attr) -> Option<Self> {
        // Get attribute path segments.
        let mut path_segments = attr.path()?.segments();

        let ink_crate_segment = path_segments.next()?;
        let ink_crate_name = ink_crate_segment.to_string();

        (matches!(ink_crate_name.as_str(), "ink" | "ink_e2e")).then(|| {
            let args = utils::parse_ink_args(&attr);
            let possible_ink_macro_segment = path_segments.next();
            let mut possible_ink_arg_name: Option<MetaName> = None;

            let ink_attr_kind = match &possible_ink_macro_segment {
                Some(ink_macro_segment) => {
                    // More than one path segment means an ink! attribute macro e.g `#[ink::contract]` or `#[ink_e2e::test]`.
                    match path_segments.next() {
                        // Any more path segments means an unknown attribute macro e.g `#[ink::abc::xyz]` or `#[ink_e2e::abc::xyz]`.
                        Some(_) => InkAttributeKind::Macro(InkMacroKind::Unknown),
                        // Otherwise we parse the ink! macro kind from the macro path segment.
                        None => InkAttributeKind::Macro(InkMacroKind::from((
                            ink_crate_name.as_str(),
                            ink_macro_segment.to_string().as_str(),
                        ))),
                    }
                }
                None => {
                    // No additional path segments means either an ink! attribute argument (e.g `#[ink(storage)]`) or an unknown attribute.
                    if args.is_empty() {
                        match attr.token_tree() {
                            // A token tree means an unknown ink! attribute argument.
                            Some(_) => InkAttributeKind::Arg(InkArgKind::Unknown),
                            // No token tree means an unknown ink! attribute macro.
                            None => InkAttributeKind::Macro(InkMacroKind::Unknown),
                        }
                    } else {
                        // Sort arguments so that we choose the "primary" `InkArgKind` for the attribute.
                        // See [`utils::ink_arg_kind_sort_order`] doc.
                        // Returns a new list so we don't change the original order for later analysis.
                        let primary_arg = args.iter().sorted().next().unwrap();
                        possible_ink_arg_name = primary_arg.name().cloned();
                        InkAttributeKind::Arg(*primary_arg.kind())
                    }
                }
            };

            Self {
                ast: attr,
                kind: ink_attr_kind,
                args,
                ink: ink_crate_segment,
                ink_macro: possible_ink_macro_segment,
                ink_arg_name: possible_ink_arg_name,
            }
        })
    }

    /// Returns the ink! attribute kind.
    ///
    /// Differentiates ink! attribute macros (e.g `#[ink::contract]`)
    /// from ink! attribute arguments (e.g `#[ink(storage)]`).
    pub fn kind(&self) -> &InkAttributeKind {
        &self.kind
    }

    /// Returns the ink! attribute arguments.
    pub fn args(&self) -> &[InkArg] {
        &self.args
    }

    /// Returns the ink! path segment node.
    pub fn ink(&self) -> &ast::PathSegment {
        &self.ink
    }

    /// Returns the ink! macro path segment node (if any) from which the attribute macro kind is derived.
    pub fn ink_macro(&self) -> Option<&ast::PathSegment> {
        self.ink_macro.as_ref()
    }

    /// Returns the ink! argument name (if any) from which the attribute argument kind is derived.
    pub fn ink_arg_name(&self) -> Option<&MetaName> {
        self.ink_arg_name.as_ref()
    }

    /// Returns sibling ink! attributes (if any).
    pub fn siblings(&self) -> impl Iterator<Item = Self> + '_ {
        self.syntax()
            .siblings(Direction::Prev)
            .chain(self.syntax().siblings(Direction::Next))
            .filter(|it| it.text_range() != self.syntax().text_range())
            .filter_map(ast::Attr::cast)
            .filter_map(Self::cast)
    }
}

impl Ord for InkAttribute {
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self.kind(), other.kind())
    }
}

impl PartialOrd for InkAttribute {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The ink! attribute kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InkAttributeKind {
    /// ink! attribute macros e.g `#[ink::contract]`.
    Macro(InkMacroKind),
    /// ink! attributes arguments e.g `#[ink(storage)]`.
    Arg(InkArgKind),
}

impl Ord for InkAttributeKind {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            // Macros have same priority.
            (InkAttributeKind::Macro(_), InkAttributeKind::Macro(_)) => Ordering::Equal,
            // Macros have higher priority (:- less since ascending order) than arguments.
            (InkAttributeKind::Macro(_), InkAttributeKind::Arg(_)) => Ordering::Less,
            (InkAttributeKind::Arg(_), InkAttributeKind::Macro(_)) => Ordering::Greater,
            // Arguments have defined priorities.
            (InkAttributeKind::Arg(lhs), InkAttributeKind::Arg(rhs)) => Ord::cmp(lhs, rhs),
        }
    }
}

impl PartialOrd for InkAttributeKind {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl InkAttributeKind {
    /// Returns true if an ink! attribute's "primary" kind is unknown.
    ///
    /// (i.e. either an unknown macro with or without known arguments
    /// - e.g. `#[ink::xyz]` or `#[ink::xyz(payable)]` -
    /// or an unknown argument - e.g. `#[ink(xyz)]`).
    pub fn is_unknown(&self) -> bool {
        matches!(
            self,
            InkAttributeKind::Macro(InkMacroKind::Unknown)
                | InkAttributeKind::Arg(InkArgKind::Unknown)
        )
    }
}

/// The ink! attribute macro kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InkMacroKind {
    /// `#[ink::chain_extension]`
    ChainExtension,
    /// `#[ink::contract]`
    Contract,
    /// `#[ink::storage_item]`
    StorageItem,
    /// `#[ink::test]`
    Test,
    /// `#[ink::trait_definition]`
    TraitDefinition,
    /// `#[ink_e2e::test]`
    E2ETest,
    /// Unknown ink! attribute macro.
    Unknown,
}

impl From<(&str, &str)> for InkMacroKind {
    /// Converts a string slice tuple representing an ink! attribute macro into an ink! attribute macro kind.
    fn from(path_segments: (&str, &str)) -> Self {
        match path_segments {
            ("ink", ink_macro) => match ink_macro {
                // `#[ink::chain_extension]`
                "chain_extension" => InkMacroKind::ChainExtension,
                // `#[ink::contract]`
                "contract" => InkMacroKind::Contract,
                // `#[ink::storage_item]`
                "storage_item" => InkMacroKind::StorageItem,
                // `#[ink::test]`
                "test" => InkMacroKind::Test,
                // `#[ink::trait_definition]`
                "trait_definition" => InkMacroKind::TraitDefinition,
                // unknown ink! attribute path (i.e unknown ink! attribute macro).
                _ => InkMacroKind::Unknown,
            },
            // `#[ink_e2e::test]`
            ("ink_e2e", "test") => InkMacroKind::E2ETest,
            // unknown ink! attribute path (i.e unknown ink! attribute macro).
            _ => InkMacroKind::Unknown,
        }
    }
}

impl fmt::Display for InkMacroKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                // `#[ink::chain_extension]`
                InkMacroKind::ChainExtension => "chain_extension",
                // `#[ink::contract]`
                InkMacroKind::Contract => "contract",
                // `#[ink::storage_item]`
                InkMacroKind::StorageItem => "storage_item",
                // `#[ink::test]`
                InkMacroKind::Test => "test",
                // `#[ink::trait_definition]`
                InkMacroKind::TraitDefinition => "trait_definition",
                // `#[ink_e2e::test]`
                InkMacroKind::E2ETest => "e2e test",
                // unknown ink! attribute path (i.e unknown ink! attribute macro).
                InkMacroKind::Unknown => "unknown",
            }
        )
    }
}

impl InkMacroKind {
    /// Returns the full path of the ink! attribute macro as a string slice (`&str`)
    ///
    /// (e.g `ink::contract` for `#[ink::contract]`).
    pub fn path_as_str(&self) -> &str {
        match self {
            // `#[ink::chain_extension]`
            InkMacroKind::ChainExtension => "ink::chain_extension",
            // `#[ink::contract]`
            InkMacroKind::Contract => "ink::contract",
            // `#[ink::storage_item]`
            InkMacroKind::StorageItem => "ink::storage_item",
            // `#[ink::test]`
            InkMacroKind::Test => "ink::test",
            // `#[ink::trait_definition]`
            InkMacroKind::TraitDefinition => "ink::trait_definition",
            // `#[ink_e2e::test]`
            InkMacroKind::E2ETest => "ink_e2e::test",
            // unknown ink! attribute path (i.e unknown ink! attribute macro).
            _ => "",
        }
    }

    /// Returns the name of the ink! attribute macro as a string slice (`&str`)
    ///
    /// (e.g `contract` for `#[ink::contract]`).
    pub fn macro_name(&self) -> &str {
        match self {
            // `#[ink::chain_extension]`
            InkMacroKind::ChainExtension => "chain_extension",
            // `#[ink::contract]`
            InkMacroKind::Contract => "contract",
            // `#[ink::storage_item]`
            InkMacroKind::StorageItem => "storage_item",
            // `#[ink::test]`
            InkMacroKind::Test => "test",
            // `#[ink::trait_definition]`
            InkMacroKind::TraitDefinition => "trait_definition",
            // `#[ink_e2e::test]`
            InkMacroKind::E2ETest => "test",
            // unknown ink! attribute path (i.e unknown ink! attribute macro).
            _ => "",
        }
    }

    /// Returns the name of the source crate of the ink! attribute macro as a string slice (`&str`)
    ///
    /// (e.g `ink` for `#[ink::contract]` or `ink_e2e` for `#[ink_e2e::test]`).
    pub fn crate_name(&self) -> &str {
        match self {
            // `#[ink::chain_extension]`
            // `#[ink::contract]`
            // `#[ink::storage_item]`
            // `#[ink::test]`
            // `#[ink::trait_definition]`
            InkMacroKind::ChainExtension
            | InkMacroKind::Contract
            | InkMacroKind::StorageItem
            | InkMacroKind::Test
            | InkMacroKind::TraitDefinition => "ink",
            // `#[ink_e2e::test]`
            InkMacroKind::E2ETest => "ink_e2e",
            // unknown ink! attribute path (i.e unknown ink! attribute macro).
            _ => "",
        }
    }
}

/// Standard data for an IR item derived from an ink! attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InkAttrData<T: AstNode> {
    /// ink! contract attributes.
    attr: InkAttribute,
    /// Annotated module (if any).
    ast: Option<T>,
    /// Syntax node for ink! contract.
    syntax: SyntaxNode,
}

impl<T: AstNode> From<InkAttribute> for InkAttrData<T> {
    fn from(attr: InkAttribute) -> Self {
        Self {
            ast: attr.syntax().parent().and_then(T::cast),
            syntax: attr
                .syntax()
                .parent()
                .expect("An attribute should always have a parent."),
            attr,
        }
    }
}

impl<T: AstNode> InkAttrData<T> {
    /// Returns the ink! attribute.
    pub fn attr(&self) -> &InkAttribute {
        &self.attr
    }

    /// Returns the ink! attribute's parent `ASTNode`.
    pub fn parent_ast(&self) -> Option<&T> {
        self.ast.as_ref()
    }

    /// Returns the ink! attribute's parent `SyntaxNode`.
    pub fn parent_syntax(&self) -> &SyntaxNode {
        &self.syntax
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;
    use ra_ap_syntax::SyntaxKind;
    use test_utils::quote_as_str;

    #[test]
    fn cast_ink_attribute_works() {
        for (code, expected_ink_attr) in [
            // Macro with no arguments.
            (
                quote_as_str! {
                    #[ink::chain_extension]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::ChainExtension),
                    vec![],
                )),
            ),
            (
                quote_as_str! {
                    #[ink::contract]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::Contract), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::storage_item]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::StorageItem), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::test]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::Test), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::trait_definition]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::TraitDefinition),
                    vec![],
                )),
            ),
            (
                quote_as_str! {
                    #[ink_e2e::test]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::E2ETest), vec![])),
            ),
            // Macro with arguments.
            (
                quote_as_str! {
                    #[ink::contract(env=my::env::Types, keep_attr="foo,bar")]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::Contract),
                    vec![
                        (InkArgKind::Env, Some(SyntaxKind::PATH)),
                        (InkArgKind::KeepAttr, Some(SyntaxKind::STRING)),
                    ],
                )),
            ),
            (
                quote_as_str! {
                    #[ink::storage_item(derive=true)]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::StorageItem),
                    vec![(InkArgKind::Derive, Some(SyntaxKind::TRUE_KW))],
                )),
            ),
            (
                quote_as_str! {
                    #[ink::trait_definition(namespace="my_namespace", keep_attr="foo,bar")]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::TraitDefinition),
                    vec![
                        (InkArgKind::Namespace, Some(SyntaxKind::STRING)),
                        (InkArgKind::KeepAttr, Some(SyntaxKind::STRING)),
                    ],
                )),
            ),
            (
                quote_as_str! {
                    #[ink_e2e::test(additional_contracts="adder/Cargo.toml flipper/Cargo.toml", environment=my::env::Types, keep_attr="foo,bar")]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::E2ETest),
                    vec![
                        (InkArgKind::AdditionalContracts, Some(SyntaxKind::STRING)),
                        (InkArgKind::Environment, Some(SyntaxKind::PATH)),
                        (InkArgKind::KeepAttr, Some(SyntaxKind::STRING)),
                    ],
                )),
            ),
            // Argument with no value.
            (
                quote_as_str! {
                    #[ink(storage)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Storage),
                    vec![(InkArgKind::Storage, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(anonymous)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Anonymous),
                    vec![(InkArgKind::Anonymous, None)],
                )),
            ),
            // Compound arguments with no value.
            // NOTE: Required and/or root-level/unambiguous arguments always have the highest priority,
            // so they become the attribute kind even when they're not the first attribute.
            (
                quote_as_str! {
                    #[ink(event, anonymous)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Event),
                    vec![(InkArgKind::Event, None), (InkArgKind::Anonymous, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(anonymous, event)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Event),
                    vec![(InkArgKind::Anonymous, None), (InkArgKind::Event, None)],
                )),
            ),
            // Argument with integer value.
            (
                quote_as_str! {
                    #[ink(selector=1)] // Decimal.
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Selector),
                    vec![(InkArgKind::Selector, Some(SyntaxKind::INT_NUMBER))],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(extension=0x1)] // Hexadecimal.
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Extension),
                    vec![(InkArgKind::Extension, Some(SyntaxKind::INT_NUMBER))],
                )),
            ),
            // Argument with wildcard/underscore value.
            (
                quote_as_str! {
                    #[ink(selector=_)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Selector),
                    vec![(InkArgKind::Selector, Some(SyntaxKind::UNDERSCORE))],
                )),
            ),
            // Argument with string value.
            (
                quote_as_str! {
                    #[ink(namespace="my_namespace")]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Namespace),
                    vec![(InkArgKind::Namespace, Some(SyntaxKind::STRING))],
                )),
            ),
            // Argument with boolean value.
            (
                quote_as_str! {
                    #[ink(handle_status=true)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::HandleStatus),
                    vec![(InkArgKind::HandleStatus, Some(SyntaxKind::TRUE_KW))],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(derive=false)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Derive),
                    vec![(InkArgKind::Derive, Some(SyntaxKind::FALSE_KW))],
                )),
            ),
            // Argument with path value.
            (
                quote_as_str! {
                    #[ink(env=my::env::Types)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Env),
                    vec![(InkArgKind::Env, Some(SyntaxKind::PATH))],
                )),
            ),
            // Compound arguments of different kinds.
            // NOTE: Required and/or root-level/unambiguous arguments always have the highest priority,
            // so they become the attribute kind even when they're not the first attribute.
            (
                quote_as_str! {
                    #[ink(message, payable, selector=1)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Message),
                    vec![
                        (InkArgKind::Message, None),
                        (InkArgKind::Payable, None),
                        (InkArgKind::Selector, Some(SyntaxKind::INT_NUMBER)),
                    ],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(selector=1, payable, message)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Message),
                    vec![
                        (InkArgKind::Selector, Some(SyntaxKind::INT_NUMBER)),
                        (InkArgKind::Payable, None),
                        (InkArgKind::Message, None),
                    ],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(event, anonymous)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Event),
                    vec![(InkArgKind::Event, None), (InkArgKind::Anonymous, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(anonymous, event)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Event),
                    vec![(InkArgKind::Anonymous, None), (InkArgKind::Event, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(extension=1, handle_status=false)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Extension),
                    vec![
                        (InkArgKind::Extension, Some(SyntaxKind::INT_NUMBER)),
                        (InkArgKind::HandleStatus, Some(SyntaxKind::FALSE_KW)),
                    ],
                )),
            ),
            // Unknown ink! macro.
            // NOTE: Macros always have the highest priority, even the unknown variety.
            (
                quote_as_str! {
                    #[ink]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::Unknown), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::Unknown), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::unknown]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::Unknown), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::xyz]
                },
                Some((InkAttributeKind::Macro(InkMacroKind::Unknown), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink::unknown(message)]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::Unknown),
                    vec![(InkArgKind::Message, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink::unknown(selector=1)]
                },
                Some((
                    InkAttributeKind::Macro(InkMacroKind::Unknown),
                    vec![(InkArgKind::Selector, Some(SyntaxKind::INT_NUMBER))],
                )),
            ),
            // Unknown ink! argument.
            // NOTE: Unknown arguments always have the lowest priority.
            (
                quote_as_str! {
                    #[ink()]
                },
                Some((InkAttributeKind::Arg(InkArgKind::Unknown), vec![])),
            ),
            (
                quote_as_str! {
                    #[ink(unknown)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Unknown),
                    vec![(InkArgKind::Unknown, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(xyz)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Unknown),
                    vec![(InkArgKind::Unknown, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(xyz="abc")]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Unknown),
                    vec![(InkArgKind::Unknown, Some(SyntaxKind::STRING))],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(message, unknown)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Message),
                    vec![(InkArgKind::Message, None), (InkArgKind::Unknown, None)],
                )),
            ),
            (
                quote_as_str! {
                    #[ink(unknown, message)]
                },
                Some((
                    InkAttributeKind::Arg(InkArgKind::Message),
                    vec![(InkArgKind::Unknown, None), (InkArgKind::Message, None)],
                )),
            ),
            // Non-ink attributes.
            // These simply return none.
            (
                quote_as_str! {
                    #[cfg_attr(not(feature = "std"), no_std)]
                },
                None,
            ),
        ] {
            // Parse attribute.
            let attr = parse_first_attribute(code);

            // Converts an attribute to an ink! attribute (if possible).
            let possible_ink_attr = InkAttribute::cast(attr);

            // Converts the ink! attribute to an array of tuples with
            // ink! attribute argument kind and an inner array of tuples with
            // ink! attribute argument kind and meta value syntax kind for easy comparisons.
            let actual_ink_attr: Option<(InkAttributeKind, Vec<(InkArgKind, Option<SyntaxKind>)>)> =
                possible_ink_attr.map(|ink_attr| {
                    (
                        // ink! attribute kind.
                        *ink_attr.kind(),
                        // array tuples of ink! attribute argument kind and meta value syntax kind.
                        ink_attr
                            .args()
                            .iter()
                            .map(|arg| (*arg.kind(), arg.value().map(|value| value.kind())))
                            .collect(),
                    )
                });

            // actual arguments should match expected arguments.
            assert_eq!(actual_ink_attr, expected_ink_attr);
        }
    }
}
