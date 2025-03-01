//! Test utilities for ink! analyzer.

use std::cmp;
use std::fs;
use std::path::PathBuf;

pub mod fixtures;

/// Quasi-quotation macro that accepts input like the `quote!` macro
/// but returns a string (`String`) instead of a `TokenStream`.
#[macro_export]
macro_rules! quote_as_string {
    ($($tt:tt)*) => {
        quote::quote!($($tt)*).to_string()
    };
}

/// Quasi-quotation macro that accepts input like the `quote!` macro
/// but returns a string slice (`&str`) instead of a `TokenStream`.
#[macro_export]
macro_rules! quote_as_str {
    ($($tt:tt)*) => {
        $crate::quote_as_string!($($tt)*).as_str()
    };
}

/// Quasi-quotation macro that accepts input like the `quote!` macro
/// but returns a `prettyplease` formatted string (`String`) instead of a `TokenStream`.
#[macro_export]
macro_rules! quote_as_pretty_string {
    ($($tt:tt)*) => {
        prettyplease::unparse(&syn::parse2::<syn::File>(quote::quote!($($tt)*)).unwrap())
    };
}

/// Reads source code from a file in the `test-fixtures` directory as a string.
///
/// `location` is the relative path of the source file minus the `.rs` extension.
pub fn read_source_code(location: &str) -> String {
    fs::read_to_string(format!("../../test-fixtures/{location}.rs"))
        .unwrap()
        .replace("\r\n", "\n")
}

/// Creates an LSP URI for a file in the `test-fixtures` directory.
///
/// `location` is the relative path of the source file minus the `.rs` extension.
pub fn source_uri(location: &str) -> lsp_types::Url {
    lsp_types::Url::from_file_path(
        fs::canonicalize(PathBuf::from(&format!("../../test-fixtures/{location}.rs"))).unwrap(),
    )
    .unwrap()
}

/// Returns the offset of `pat` in `subject`.
///
/// offset is placed at the end of `pat` in `subject` by default,
/// unless `pat` is `Some` substring that starts with `<-`,
/// in which case the offset is placed at the beginning of the substring.
///
/// Additionally, `pat` is searched from the beginning of `subject` by default,
/// unless `pat` is `Some` substring that ends with `->`,
/// in which case `pat` is searched from the end of `subject`
/// (offsets are still calculated from the beginning of `subject` even in this case).
pub fn parse_offset_at(subject: &str, pat: Option<&str>) -> Option<usize> {
    let mut parsed_pat = pat;
    let mut position = Direction::End;
    let mut origin = Direction::Start;

    if let Some(substr) = parsed_pat {
        if substr.starts_with("<-") {
            // Strip the prefix and set offset position the `Start` variant if substring starts with "<-".
            parsed_pat = substr.strip_prefix("<-");
            position = Direction::Start;
        }

        if substr.ends_with("->") {
            // Strip the suffix and set the search origin the `End` variant if substring ends with "->".
            parsed_pat = parsed_pat.and_then(|substr| substr.strip_suffix("->"));
            origin = Direction::End;
        }
    }

    // Retrieve the offset using the structured utility.
    offset_at(subject, parsed_pat, position, origin)
}

/// An origin or placement direction.
enum Direction {
    Start,
    End,
}

/// Returns the offset of `pat` in `subject` using
/// `position` (i.e `Start` or `End`) to determine offset placement around the `pat` substring and
/// `origin` (i.e `Start` or `End`) to determine the search origin for the `pat` in `subject` (i.e beginning or end)
/// (offsets are still calculated from the beginning of `subject` regardless of the search origin/direction).
fn offset_at(
    subject: &str,
    pat: Option<&str>,
    position: Direction,
    origin: Direction,
) -> Option<usize> {
    match pat {
        Some(substr) => {
            // Origin determines how we search `subject` for the substring.
            let offset = match origin {
                Direction::Start => subject.find(substr),
                Direction::End => subject.rfind(substr),
            };
            // Position determines whether the offset is set at the beginning or end of the substring.
            match position {
                Direction::Start => offset,
                Direction::End => offset.map(|idx| cmp::min(idx + substr.len(), subject.len())),
            }
        }
        // No `pat` places offset at the beginning or the end of the subject depending on the desired position.
        None => match position {
            Direction::Start => Some(0),
            Direction::End => Some(subject.len()),
        },
    }
}

/// Returns client capabilities with support for UTF-8 position encoding.
pub fn simple_client_config() -> lsp_types::ClientCapabilities {
    lsp_types::ClientCapabilities {
        general: Some(lsp_types::GeneralClientCapabilities {
            position_encodings: Some(vec![
                lsp_types::PositionEncodingKind::UTF8,
                lsp_types::PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        text_document: Some(lsp_types::TextDocumentClientCapabilities {
            signature_help: Some(lsp_types::SignatureHelpClientCapabilities {
                signature_information: Some(lsp_types::SignatureInformationSettings {
                    parameter_information: Some(lsp_types::ParameterInformationSettings {
                        label_offset_support: Some(true),
                    }),
                    active_parameter_support: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        workspace: Some(lsp_types::WorkspaceClientCapabilities {
            apply_edit: Some(true),
            workspace_edit: Some(lsp_types::WorkspaceEditClientCapabilities {
                document_changes: Some(true),
                resource_operations: Some(vec![
                    lsp_types::ResourceOperationKind::Create,
                    lsp_types::ResourceOperationKind::Delete,
                    lsp_types::ResourceOperationKind::Rename,
                ]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Describes a group of tests to run on a smart contract code from a source file.
#[derive(Debug)]
pub struct TestGroup {
    /// Location of the smart code (e.g. in the `test-fixtures` directory in the project root).
    pub source: &'static str,
    /// List of test cases.
    pub test_cases: Vec<TestCase>,
}

/// Describes a single test case in a [`TestGroup`].
#[derive(Debug)]
pub struct TestCase {
    /// List of modifications to perform on the original source before running the test.
    pub modifications: Option<Vec<TestCaseModification>>,
    /// Parameters used by the test case.
    pub params: Option<TestCaseParams>,
    /// Expected results for the test case.
    pub results: TestCaseResults,
}

/// Describes a modification to perform on the original smart contract code.
#[derive(Debug)]
pub struct TestCaseModification {
    /// Substring used to find the start offset for the code snippet to replace (see [`parse_offset_at`] doc).
    pub start_pat: Option<&'static str>,
    /// Substring used to find the end offset for the code snippet to replace (see [`parse_offset_at`] doc).
    pub end_pat: Option<&'static str>,
    /// Replacement snippet to be inserted in place of the code snippet covering the `start_pat` and `end_pat` offsets defined above.
    pub replacement: &'static str,
}

/// Variants for [`TestCase`] parameters.
#[derive(Debug)]
pub enum TestCaseParams {
    Action(TestParamsOffsetOnly),
    Completion(TestParamsOffsetOnly),
    Hover(TestParamsRangeOnly),
    InlayHints(Option<TestParamsRangeOnly>),
    SignatureHelp(TestParamsOffsetOnly),
}

/// Variants for [`TestCase`] results.
#[derive(Debug)]
pub enum TestCaseResults {
    Action(Vec<TestResultAction>),
    Completion(Vec<TestResultTextRange>),
    // Expected number of diagnostic errors/warnings.
    Diagnostic {
        n: usize,
        // Vec<Vec<Vec because we iterate over diagnostics > quickfixes > text edits.
        quickfixes: Vec<Vec<Vec<TestResultTextRange>>>,
    },
    Hover(Option<TestResultTextRange>),
    InlayHints(Vec<TestResultTextOffsetRange>),
    SignatureHelp(Vec<TestResultSignatureHelp>),
}

/// Test parameters for offset-based tests.
#[derive(Debug)]
pub struct TestParamsOffsetOnly {
    /// Substring used to find the cursor offset parameter for the test case (see [`parse_offset_at`] doc).
    pub pat: Option<&'static str>,
}

/// Test parameters for text range based tests.
#[derive(Debug)]
pub struct TestParamsRangeOnly {
    /// Substring used to find the start offset for the focus range (see [`parse_offset_at`] doc).
    pub start_pat: Option<&'static str>,
    /// Substring used to find the end offset for the focus range (see [`parse_offset_at`] doc).
    pub end_pat: Option<&'static str>,
}

/// Describes the expected text and range result.
#[derive(Debug)]
pub struct TestResultTextRange {
    /// Expected text.
    pub text: &'static str,
    /// Substring used to find the start of the offset of the expected result (see [`parse_offset_at`] doc).
    pub start_pat: Option<&'static str>,
    /// Substring used to find the end of the offset of the expected result (see [`parse_offset_at`] doc).
    pub end_pat: Option<&'static str>,
}

/// Describes the expected text, offset and range result.
#[derive(Debug)]
pub struct TestResultTextOffsetRange {
    /// Expected text.
    pub text: &'static str,
    /// Substring used to find the offset of the expected result (see [`parse_offset_at`] doc).
    pub pos_pat: Option<&'static str>,
    /// Substring used to find the start of the offset of the expected result (see [`parse_offset_at`] doc).
    pub range_start_pat: Option<&'static str>,
    /// Substring used to find the end of the offset of the expected result (see [`parse_offset_at`] doc).
    pub range_end_pat: Option<&'static str>,
}

/// Describes the expected action label and text edits.
#[derive(Debug)]
pub struct TestResultAction {
    /// Expected label.
    pub label: &'static str,
    /// Expected edits.
    pub edits: Vec<TestResultTextRange>,
}

/// Describes the expected signature help.
#[derive(Debug)]
pub struct TestResultSignatureHelp {
    /// Expected label.
    pub label: &'static str,
    /// Substring used to find the start of the offset of the expected result (see [`parse_offset_at`] doc).
    pub start_pat: Option<&'static str>,
    /// Substring used to find the end of the offset of the expected result (see [`parse_offset_at`] doc).
    pub end_pat: Option<&'static str>,
    /// Expected parameters.
    pub params: Vec<TestResultSignatureParam>,
    /// Expected active parameter.
    pub active_param: Option<usize>,
}

/// Describes the expected signature parameter.
#[derive(Debug)]
pub struct TestResultSignatureParam {
    /// Substring used to find the start of the offset of the expected result (see [`parse_offset_at`] doc).
    pub start_pat: Option<&'static str>,
    /// Substring used to find the end of the offset of the expected result (see [`parse_offset_at`] doc).
    pub end_pat: Option<&'static str>,
}

/// Applies the test case modifications to the source code.
pub fn apply_test_modifications(source_code: &mut String, modifications: &[TestCaseModification]) {
    for modification in modifications {
        let start_offset = parse_offset_at(source_code, modification.start_pat).unwrap();
        let end_offset = parse_offset_at(source_code, modification.end_pat).unwrap();
        source_code.replace_range(start_offset..end_offset, modification.replacement);
    }
}

/// Sends an LSP `DidOpenTextDocument` or `DidChangeTextDocument` notification depending on the value of `version`.
pub fn versioned_document_sync_notification(
    uri: lsp_types::Url,
    test_code: String,
    version: i32,
    sender: &crossbeam_channel::Sender<lsp_server::Message>,
) {
    // Creates `DidOpenTextDocument` or `DidChangeTextDocument` notification depending on the current value of `version`.
    use lsp_types::notification::Notification;
    let not = match version {
        // Creates `DidOpenTextDocument` notification if version is zero.
        0 => lsp_server::Notification {
            method: lsp_types::notification::DidOpenTextDocument::METHOD.to_string(),
            params: serde_json::to_value(lsp_types::DidOpenTextDocumentParams {
                text_document: lsp_types::TextDocumentItem {
                    uri,
                    language_id: "rust".to_string(),
                    version,
                    text: test_code,
                },
            })
            .unwrap(),
        },
        // Creates `DidChangeTextDocument` notification if version is greater than zero.
        _ => lsp_server::Notification {
            method: lsp_types::notification::DidChangeTextDocument::METHOD.to_string(),
            params: serde_json::to_value(lsp_types::DidChangeTextDocumentParams {
                text_document: lsp_types::VersionedTextDocumentIdentifier { uri, version },
                content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: test_code,
                }],
            })
            .unwrap(),
        },
    };
    // Sends the `DidOpenTextDocument` or `DidChangeTextDocument` notification.
    sender.send(not.into()).unwrap();
}

/// Removes whitespace from a string (e.g to simplify text comparisons by ignoring whitespace formatting).
pub fn remove_whitespace(mut text: String) -> String {
    text.retain(|it| !it.is_whitespace());
    text
}

/// A custom string type used in test comparisons where we only want a partial match
/// (i.e. either both strings are empty or the RHS is a substring of the LHS in comparisons)
#[derive(Debug, Clone, Eq)]
pub struct PartialMatchStr<'a>(&'a str);

impl<'a> PartialEq for PartialMatchStr<'a> {
    fn eq(&self, other: &Self) -> bool {
        if self.0.is_empty() {
            other.0.is_empty()
        } else {
            self.0.contains(other.0)
        }
    }
}

impl<'a> From<&'a str> for PartialMatchStr<'a> {
    fn from(value: &'a str) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_at_variants_works() {
        for (subject, pat, start_offset, end_offset, r_start_offset, r_end_offset) in [
            // subject - the subject being searched.
            // pat = substring used to find the cursor offset,
            // start_offset = the expected offset (calculated from the start of `subject`) when `subject` is searched
            //                from the beginning and the cursor is placed at the beginning of `pat`.
            // end_offset = the expected offset (calculated from the start of `subject`) when `subject` is searched
            //              from the beginning and the cursor is placed at the end of `pat`.
            // r_start_offset = the expected offset (calculated from the start of `subject`) when `subject` is searched
            //                  from the end and the cursor is placed at the beginning of `pat`.
            // r_end_offset = the expected offset (calculated from the start of `subject`) when `subject` is searched
            //                from the end and the cursor is placed at the end of `pat`.
            ("", None, Some(0), Some(0), Some(0), Some(0)),
            ("", Some(""), Some(0), Some(0), Some(0), Some(0)),
            ("", Some("a"), None, None, None, None),
            ("hello", None, Some(0), Some(5), Some(0), Some(5)),
            ("hello", Some(""), Some(0), Some(0), Some(5), Some(5)),
            ("hello", Some("e"), Some(1), Some(2), Some(1), Some(2)),
            ("hello", Some("l"), Some(2), Some(3), Some(3), Some(4)),
            ("hello", Some("lo"), Some(3), Some(5), Some(3), Some(5)),
            (
                "hello, world",
                Some("d"),
                Some(11),
                Some(12),
                Some(11),
                Some(12),
            ),
            (
                "hello, world",
                Some("o"),
                Some(4),
                Some(5),
                Some(8),
                Some(9),
            ),
            (
                "hello, world",
                Some(","),
                Some(5),
                Some(6),
                Some(5),
                Some(6),
            ),
            (
                "hello, world",
                Some("o,"),
                Some(4),
                Some(6),
                Some(4),
                Some(6),
            ),
            (
                "hello, world",
                Some(", "),
                Some(5),
                Some(7),
                Some(5),
                Some(7),
            ),
            // Ref: <https://doc.rust-lang.org/std/primitive.str.html#method.find>.
            // Ref: <https://doc.rust-lang.org/std/primitive.str.html#method.rfind>.
            (
                "Löwe 老虎 Léopard Gepardi",
                Some("ö"),
                Some(1),
                Some(3),
                Some(1),
                Some(3),
            ),
            (
                "Löwe 老虎 Léopard Gepardi",
                Some("老"),
                Some(6),
                Some(9),
                Some(6),
                Some(9),
            ),
            (
                "Löwe 老虎 Léopard Gepardi",
                Some("é"),
                Some(14),
                Some(16),
                Some(14),
                Some(16),
            ),
            (
                "Löwe 老虎 Léopard Gepardi",
                Some("éo"),
                Some(14),
                Some(17),
                Some(14),
                Some(17),
            ),
            (
                "Löwe 老虎 Léopard Gepardi",
                Some("老虎"),
                Some(6),
                Some(12),
                Some(6),
                Some(12),
            ),
            (
                "Löwe 老虎 Léopard Gepardi",
                Some("pard"),
                Some(17),
                Some(21),
                Some(24),
                Some(28),
            ),
        ] {
            // Start offset from beginning.
            assert_eq!(
                offset_at(subject, pat, Direction::Start, Direction::Start),
                start_offset
            );
            // End offset from beginning.
            assert_eq!(
                offset_at(subject, pat, Direction::End, Direction::Start),
                end_offset
            );

            // Start offset from end.
            assert_eq!(
                offset_at(subject, pat, Direction::Start, Direction::End),
                r_start_offset
            );
            // End offset from end.
            assert_eq!(
                offset_at(subject, pat, Direction::End, Direction::End),
                r_end_offset
            );

            if pat.is_some() {
                // Parse start offset from beginning, `<-` prefix only works if `pat` is Some.
                assert_eq!(
                    parse_offset_at(subject, pat.map(|substr| format!("<-{substr}")).as_deref()),
                    start_offset
                );
            }
            // Parse end offset from beginning.
            assert_eq!(parse_offset_at(subject, pat), end_offset);

            if pat.is_some() {
                // Parse start offset from end, `<-` and `->` affixes only work if `pat` is Some.
                assert_eq!(
                    parse_offset_at(
                        subject,
                        pat.map(|substr| format!("<-{substr}->")).as_deref()
                    ),
                    r_start_offset
                );

                // Parse end offset from end, `->` suffix only works if `pat` is Some.
                assert_eq!(
                    parse_offset_at(subject, pat.map(|substr| format!("{substr}->")).as_deref()),
                    r_end_offset
                );
            }
        }
    }
}
