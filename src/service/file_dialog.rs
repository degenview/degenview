//! Open and save panels: `osascript` on macOS (no extra crate, and the system's own panels) and the
//! XDG file chooser portal on Linux (through `rfd`), which inside a Flatpak is the only way to
//! reach a file outside the sandbox. Windows has none yet. Every call blocks until the user
//! answers, so run them on a worker thread. A cancelled panel is `None`, not an error.

// Transient: the portfolio import/export flow calls these.
#![allow(dead_code)]

use std::path::PathBuf;
#[cfg(not(target_os = "linux"))]
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

/// File name extensions the open panels offer, without the dot. The AppleScript panels ask for
/// the matching uniform type identifiers instead.
const CSV_EXTENSIONS: [&str; 3] = ["csv", "txt", "text"];
const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];
const JSON_EXTENSIONS: [&str; 1] = ["json"];

#[cfg(target_os = "linux")]
mod portal {
    use std::path::PathBuf;

    /// `rfd` reports a cancelled dialog and one that could not open (no portal, no `zenity`) the
    /// same way, as `None`; both read as "the user chose nothing".
    pub fn open(title: &str, filter: &str, extensions: &[&str]) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter, extensions)
            .pick_file()
    }

    pub fn save(title: &str, default_name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(default_name)
            .save_file()
    }
}

#[cfg(not(target_os = "linux"))]
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
    #[cfg(target_os = "linux")]
    {
        Ok(portal::open(prompt, "CSV or text", &CSV_EXTENSIONS))
    }
    #[cfg(not(target_os = "linux"))]
    {
        run(&open_script(
            prompt,
            &["public.comma-separated-values-text", "public.plain-text"],
        ))
    }
}

/// Asks for a JSON file to read.
pub fn choose_json(prompt: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(portal::open(prompt, "JSON", &JSON_EXTENSIONS))
    }
    #[cfg(not(target_os = "linux"))]
    {
        run(&open_script(prompt, &["public.json"]))
    }
}

/// Asks for a PNG or JPEG picture to read.
pub fn choose_image(prompt: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(portal::open(prompt, "Pictures", &IMAGE_EXTENSIONS))
    }
    #[cfg(not(target_os = "linux"))]
    {
        run(&open_script(prompt, &["public.png", "public.jpeg"]))
    }
}

/// Asks where to save a file.
pub fn choose_save_path(prompt: &str, default_name: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(portal::save(prompt, default_name))
    }
    #[cfg(not(target_os = "linux"))]
    {
        run(&save_script(prompt, default_name))
    }
}

/// Reads a chosen file as UTF-8 text.
pub fn read_text(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))
}

/// Reads a chosen file as UTF-8 text, refusing one longer than `limit` bytes without reading it.
pub fn read_text_up_to(path: &std::path::Path, limit: usize) -> Result<String, String> {
    let length = std::fs::metadata(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?
        .len();
    if length > limit as u64 {
        return Err(format!(
            "{} is {} KB, which is too large to be what was asked for (the limit is {} KB).",
            path.display(),
            length / 1024,
            limit / 1024
        ));
    }
    read_text(path)
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
    fn the_extension_filters_are_bare_lowercase_names() {
        for extension in CSV_EXTENSIONS.iter().chain(&IMAGE_EXTENSIONS) {
            assert!(!extension.starts_with('.') && !extension.starts_with('*'));
            assert_eq!(*extension, extension.to_lowercase());
        }
        assert!(CSV_EXTENSIONS.contains(&"csv"));
        assert!(IMAGE_EXTENSIONS.contains(&"png") && IMAGE_EXTENSIONS.contains(&"jpg"));
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
    fn a_file_over_the_limit_is_refused_unread() {
        let path =
            std::env::temp_dir().join(format!("degenview-limit-{}.json", std::process::id()));
        write_text(&path, &"x".repeat(2048)).unwrap();
        assert!(read_text_up_to(&path, 4096).is_ok());
        let error = read_text_up_to(&path, 1024).unwrap_err();
        assert!(error.contains("too large"), "{error}");
        let _ = std::fs::remove_file(&path);
        assert!(
            read_text_up_to(&path, 4096)
                .unwrap_err()
                .starts_with("Could not read")
        );
    }

    #[test]
    fn the_json_panel_asks_for_json_only() {
        assert!(open_script("Import", &["public.json"]).contains("{\"public.json\"}"));
        assert_eq!(JSON_EXTENSIONS, ["json"]);
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
