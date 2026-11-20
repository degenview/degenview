//! Script names double as file names inside the library folder, so anything that could escape
//! it or produce an unusable file is rejected before any disk access. Port of
//! `ScriptNameValidator.swift`; the messages are the Swift ones verbatim.

/// The 255-byte file-name limit minus room for the `.pine` extension.
pub const MAX_NAME_BYTES: usize = 250;
pub const FILE_EXTENSION: &str = "pine";

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ScriptNameError {
    #[error("Enter a name for the script.")]
    Empty,
    #[error("Script names cannot start with a period.")]
    Hidden,
    #[error("Script names cannot contain “{0}”.")]
    ForbiddenCharacter(char),
    #[error("Script names cannot contain control characters.")]
    ControlCharacter,
    #[error("Script names must be {MAX_NAME_BYTES} bytes or fewer.")]
    TooLong,
}

/// The name as it will be stored: trimmed, without a typed `.pine` extension.
pub fn validate(raw: &str) -> Result<String, ScriptNameError> {
    let mut name = raw.trim();
    let suffix = format!(".{FILE_EXTENSION}");
    if name.chars().count() > suffix.chars().count() && name.to_lowercase().ends_with(&suffix) {
        name = name[..name.len() - suffix.len()].trim();
    }
    if name.is_empty() {
        return Err(ScriptNameError::Empty);
    }
    if name.starts_with('.') {
        return Err(ScriptNameError::Hidden);
    }
    if let Some(bad) = name.chars().find(|c| matches!(c, '/' | '\\' | ':')) {
        return Err(ScriptNameError::ForbiddenCharacter(bad));
    }
    if name.chars().any(char::is_control) {
        return Err(ScriptNameError::ControlCharacter);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(ScriptNameError::TooLong);
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_strips_extension() {
        assert_eq!(validate("  My Script.PINE ").unwrap(), "My Script");
        assert_eq!(validate(".pine"), Err(ScriptNameError::Hidden));
    }

    #[test]
    fn rejects_bad_names_with_swift_messages() {
        assert_eq!(
            validate("   ").unwrap_err().to_string(),
            "Enter a name for the script."
        );
        assert_eq!(
            validate(".hidden").unwrap_err().to_string(),
            "Script names cannot start with a period."
        );
        assert_eq!(
            validate("a/b").unwrap_err().to_string(),
            "Script names cannot contain “/”."
        );
        assert_eq!(
            validate("a:b"),
            Err(ScriptNameError::ForbiddenCharacter(':'))
        );
        assert_eq!(validate("a\tb"), Err(ScriptNameError::ControlCharacter));
        assert_eq!(validate(&"x".repeat(251)), Err(ScriptNameError::TooLong));
        assert!(validate(&"x".repeat(250)).is_ok());
    }
}
