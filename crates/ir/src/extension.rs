//! ink! extension IR.

use ink_analyzer_macro::{FromInkAttribute, FromSyntax};
use ra_ap_syntax::ast;

use crate::{
    utils, FromInkAttribute, FromSyntax, InkArg, InkArgKind, InkAttrData, InkAttribute, InkFn,
};

/// An ink! extension.
#[derive(Debug, Clone, PartialEq, Eq, FromInkAttribute, FromSyntax)]
pub struct Extension {
    /// ink! attribute IR data.
    #[arg_kind(Extension)]
    ink_attr: InkAttrData<ast::Fn>,
}

impl InkFn for Extension {
    fn fn_item(&self) -> Option<&ast::Fn> {
        self.ink_attr.parent_ast()
    }
}

impl Extension {
    /// Returns the ink! extension argument (if any) for the ink! extension.
    pub fn extension_arg(&self) -> Option<InkArg> {
        utils::ink_arg_by_kind(self.syntax(), InkArgKind::Extension)
    }

    /// Returns the ink! handle_status argument (if any) for the ink! extension.
    pub fn handle_status_arg(&self) -> Option<InkArg> {
        utils::ink_arg_by_kind(self.syntax(), InkArgKind::HandleStatus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote_as_str;
    use crate::test_utils::*;

    #[test]
    fn cast_works() {
        for (code, has_handle_status) in [
            (
                quote_as_str! {
                    #[ink(extension=1)]
                    fn my_extension();
                },
                false,
            ),
            (
                quote_as_str! {
                    #[ink(extension=0x1)]
                    fn my_extension();
                },
                false,
            ),
            (
                quote_as_str! {
                    #[ink(extension=1, handle_status=false)]
                    fn my_extension();
                },
                true,
            ),
            (
                quote_as_str! {
                    #[ink(extension=1)]
                    #[ink(handle_status=true)]
                    fn my_extension();
                },
                true,
            ),
        ] {
            let ink_attr = parse_first_ink_attribute(code);

            let extension = Extension::cast(ink_attr).unwrap();

            // `extension_arg` argument exists.
            assert!(extension.extension_arg().is_some());

            // `handle_status` argument exists.
            assert_eq!(extension.handle_status_arg().is_some(), has_handle_status);

            // `fn` item exists.
            assert!(extension.fn_item().is_some());
        }
    }
}
