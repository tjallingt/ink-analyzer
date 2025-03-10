//! Utilities for generate ink! project files.

use self::snippets::{CARGO_TOML_PLAIN, CARGO_TOML_SNIPPET, CONTRACT_PLAIN, CONTRACT_SNIPPET};
use crate::utils;

pub mod snippets;

/// Code stubs/snippets for creating an ink! project
/// (i.e. code stubs/snippets for `lib.rs` and `Cargo.toml`).
#[derive(Debug, PartialEq, Eq)]
pub struct Project {
    /// The `lib.rs` content.
    pub lib: ProjectFile,
    /// The `Cargo.toml` content.
    pub cargo: ProjectFile,
}

/// Code stubs/snippets for creating a file in an ink! project
/// (e.g. `lib.rs` or `Cargo.toml` for an ink! contract).
#[derive(Debug, PartialEq, Eq)]
pub struct ProjectFile {
    /// A plain text code stub.
    pub plain: String,
    /// A snippet (i.e. with tab stops and/or placeholders).
    pub snippet: Option<String>,
}

/// An ink! project error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Invalid package name.
    ///
    /// Ref: <https://doc.rust-lang.org/cargo/reference/manifest.html#the-name-field>.
    PackageName,
    /// Invalid contract name.
    ///
    /// Ref: <https://github.com/paritytech/cargo-contract/blob/v3.2.0/crates/build/src/new.rs#L34-L52>.
    ContractName,
}

/// Returns code stubs/snippets for creating a new ink! project given a name.
pub fn new_project(name: String) -> Result<Project, Error> {
    // Validates that name is a valid Rust package name.
    // Ref: <https://doc.rust-lang.org/cargo/reference/manifest.html#the-name-field>.
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Error::PackageName);
    }

    // Validates that name is a valid ink! contract name (i.e. contract names must additionally begin with an alphabetic character).
    // Ref: <https://github.com/paritytech/cargo-contract/blob/v3.2.0/crates/build/src/new.rs#L34-L52>.
    if !name.chars().next().map_or(false, char::is_alphabetic) {
        return Err(Error::ContractName);
    }

    // Generates `mod` and storage `struct` names for the contract.
    let module_name = name.replace('-', "_");
    let struct_name = utils::pascal_case(&module_name);

    // Returns project code stubs/snippets.
    Ok(Project {
        // Generates `lib.rs`.
        lib: ProjectFile {
            plain: CONTRACT_PLAIN
                .replace("my_contract", &module_name)
                .replace("MyContract", &struct_name),
            snippet: Some(
                CONTRACT_SNIPPET
                    .replace("my_contract", &module_name)
                    .replace("MyContract", &struct_name),
            ),
        },
        // Generates `Cargo.toml`.
        cargo: ProjectFile {
            plain: CARGO_TOML_PLAIN.replace("my_contract", &name),
            snippet: Some(CARGO_TOML_SNIPPET.replace("my_contract", &name)),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Analysis;

    // Ref: <https://doc.rust-lang.org/cargo/reference/manifest.html#the-name-field>.
    // Ref: <https://github.com/paritytech/cargo-contract/blob/v3.2.0/crates/build/src/new.rs#L34-L52>.
    #[test]
    fn invalid_project_name_fails() {
        for (name, expected_error) in [
            // Empty.
            ("", Error::PackageName),
            // Disallowed characters (i.e not alphanumeric, `-` or `_`).
            ("hello!", Error::PackageName),
            ("hello world", Error::PackageName),
            ("💝", Error::PackageName),
            // Starts with non-alphabetic character.
            ("1hello", Error::ContractName),
            ("-hello", Error::ContractName),
            ("_hello", Error::ContractName),
        ] {
            assert_eq!(new_project(name.to_string()), Err(expected_error));
        }
    }

    #[test]
    fn valid_project_name_works() {
        for name in ["hello", "hello_world", "hello-world"] {
            // Generates an ink! contract project.
            let result = new_project(name.to_string());
            assert!(result.is_ok());

            // Verifies that the generated code stub is a valid contract.
            let contract_code = result.unwrap().lib.plain;
            let analysis = Analysis::new(&contract_code);
            assert_eq!(analysis.diagnostics().len(), 0);
        }
    }
}
