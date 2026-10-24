//! Open and save panels, through `osascript`: no extra crate, and the system's own panels. Every
//! call blocks until the user answers, so run them on a worker thread. A cancelled panel is
//! `None`, not an error.

// Transient: the portfolio import/export flow calls these.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

/// A string as an AppleScript literal.
pub fn applescript_string(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' | '\r' => quoted.push(' '),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// "choose file" restricted to uniform type identifiers.
pub fn open_script(prompt: &str, type_identifiers: &[&str]) -> String {
    let types: Vec<String> = type_identifiers
        .iter()
        .map(|identifier| applescript_string(identifier))
        .collect();
    format!(
        "POSIX path of (choose file with prompt {} of type {{{}}})",
        applescript_string(prompt),
        types.join(", ")
    )
}

/// "choose file name": the user types where to save and what to call it.
pub fn save_script(prompt: &str, default_name: &str) -> String {
    format!(
        "POSIX path of (choose file name with prompt {} default name {})",
        applescript_string(prompt),
        applescript_string(default_name)
    )
}

/// What the script printed, as a path; empty output (a cancel) is None.
pub fn parse_path(output: &str) -> Option<PathBuf> {
    let trimmed = output.trim_end_matches(['\n', '\r']);
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn run(script: &str) -> Result<Option<PathBuf>, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| format!("Could not open the file dialog: {error}"))?;
    if output.status.success() {
        return Ok(parse_path(&String::from_utf8_lossy(&output.stdout)));
    }
    let message = String::from_utf8_lossy(&output.stderr);
    // -128 is "User canceled".
    if message.contains("-128") {
        Ok(None)
    } else {
        Err(format!("The file dialog failed: {}", message.trim()))
    }
}

/// Asks for a CSV or text file to read.
pub fn choose_csv(prompt: &str) -> Result<Option<PathBuf>, String> {
    run(&open_script(
        prompt,
        &["public.comma-separated-values-text", "public.plain-text"],
    ))
}

/// Asks for a PNG or JPEG picture to read.
pub fn choose_image(prompt: &str) -> Result<Option<PathBuf>, String> {
    run(&open_script(prompt, &["public.png", "public.jpeg"]))
}

/// Asks where to save a file.
pub fn choose_save_path(prompt: &str, default_name: &str) -> Result<Option<PathBuf>, String> {
    run(&save_script(prompt, default_name))
}

/// Reads a chosen file as UTF-8 text.
pub fn read_text(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))
}

/// Writes `text` to `path`.
pub fn write_text(path: &std::path::Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_quoted_for_applescript() {
        assert_eq!(applescript_string("Import"), "\"Import\"");
        assert_eq!(applescript_string("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(applescript_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(applescript_string("two\nlines"), "\"two lines\"");
    }

    #[test]
    fn the_picture_panel_asks_for_png_and_jpeg_only() {
        let script = open_script("Choose a picture", &["public.png", "public.jpeg"]);
        assert!(script.contains("\"public.png\", \"public.jpeg\""));
        assert!(!script.contains("text"));
    }

    #[test]
    fn the_scripts_ask_for_the_right_panels() {
        assert_eq!(
            open_script("Choose a CSV", &["public.plain-text"]),
            "POSIX path of (choose file with prompt \"Choose a CSV\" of type {\"public.plain-text\"})"
        );
        assert_eq!(
            save_script("Export", "degenview-portfolio-transactions.csv"),
            "POSIX path of (choose file name with prompt \"Export\" default name \"degenview-portfolio-transactions.csv\")"
        );
    }

    #[test]
    fn paths_come_back_without_the_trailing_newline() {
        assert_eq!(
            parse_path("/Users/me/a b.csv\n"),
            Some(PathBuf::from("/Users/me/a b.csv"))
        );
        assert_eq!(parse_path("\n"), None);
        assert_eq!(parse_path(""), None);
    }

    #[test]
    fn text_round_trips_through_a_file() {
        let path =
            std::env::temp_dir().join(format!("degenview-dialog-{}.csv", std::process::id()));
        write_text(&path, "a,b\n1,2\n").unwrap();
        assert_eq!(read_text(&path).unwrap(), "a,b\n1,2\n");
        let _ = std::fs::remove_file(&path);
        assert!(read_text(&path).unwrap_err().starts_with("Could not read"));
    }
}
