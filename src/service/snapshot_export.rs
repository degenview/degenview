//! A snapshot as a file: PNG encoding, where it is saved and writing it so a failure never
//! leaves half a file behind.

use crate::model::snapshot;
use eframe::egui::ColorImage;
use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Eq, PartialEq)]
pub enum ExportError {
    /// The image could not be encoded.
    Encode(String),
    /// The file could not be written; the message says what the system said.
    Write(String),
    /// There is nowhere to save to.
    NoFolder,
    /// The file dialog failed (not: was cancelled).
    Dialog(String),
}

impl ExportError {
    pub fn message(&self) -> String {
        match self {
            Self::Encode(why) => format!("Could not make the image: {why}"),
            Self::Write(why) => format!("Could not save the image: {why}"),
            Self::NoFolder => "Could not find a folder to save the image in.".to_owned(),
            Self::Dialog(why) => why.clone(),
        }
    }
}

/// `image` as PNG bytes. The image is opaque, so premultiplied and plain colour are the same.
pub fn encode_png(image: &ColorImage) -> Result<Vec<u8>, ExportError> {
    let [width, height] = image.size;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
    }
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Default, FilterType::Adaptive)
        .write_image(
            &rgba,
            u32::try_from(width).map_err(|e| ExportError::Encode(e.to_string()))?,
            u32::try_from(height).map_err(|e| ExportError::Encode(e.to_string()))?,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| ExportError::Encode(error.to_string()))?;
    Ok(png)
}

fn describe(error: &std::io::Error, path: &Path) -> String {
    use std::io::ErrorKind;
    let folder = path.parent().unwrap_or(path).display();
    match error.kind() {
        ErrorKind::PermissionDenied => format!("no permission to write in {folder}"),
        ErrorKind::StorageFull => "the disk is full".to_owned(),
        ErrorKind::NotFound => format!("{folder} does not exist"),
        _ => error.to_string(),
    }
}

/// Writes `bytes` to `path` by writing a file beside it and renaming that over `path`, so the
/// path holds either what was there or all of the new file. The beside-file is removed when
/// anything fails.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let name = path.file_name().ok_or(ExportError::NoFolder)?;
    let mut partial_name = OsString::from(".");
    partial_name.push(name);
    partial_name.push(format!(".{}.part", std::process::id()));
    let partial = path.with_file_name(partial_name);
    let result = (|| {
        let mut file = std::fs::File::create_new(&partial)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&partial, path)
    })();
    result.map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        ExportError::Write(describe(&error, path))
    })
}

/// The user's downloads folder, from the environment `home` (`HOME`) and `profile`
/// (`USERPROFILE`) came from.
pub fn downloads_folder(home: Option<OsString>, profile: Option<OsString>) -> Option<PathBuf> {
    let base = home.or(profile).filter(|base| !base.is_empty())?;
    Some(PathBuf::from(base).join("Downloads"))
}

/// Where a snapshot with this name goes when no dialog asks: the downloads folder, under a name
/// no file there has.
pub fn downloads_path(name: &str) -> Result<PathBuf, ExportError> {
    let folder = downloads_folder(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
        .ok_or(ExportError::NoFolder)?;
    std::fs::create_dir_all(&folder)
        .map_err(|error| ExportError::Write(describe(&error, &folder)))?;
    Ok(folder.join(snapshot::unused_name(name, |candidate| {
        folder.join(candidate).exists()
    })))
}

/// Where the user wants the file: the system's save panel on macOS and Linux (`None` if they
/// cancel), the downloads folder elsewhere.
pub fn choose_destination(name: &str) -> Result<Option<PathBuf>, ExportError> {
    if crate::platform::has_file_dialogs() {
        let chosen = crate::service::file_dialog::choose_save_path("Save chart image", name)
            .map_err(ExportError::Dialog)?;
        Ok(
            chosen.map(|path| match path.file_name().and_then(|n| n.to_str()) {
                Some(file) => path.with_file_name(snapshot::with_png_extension(file)),
                None => path,
            }),
        )
    } else {
        downloads_path(name).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("degenview-export-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn picture() -> ColorImage {
        let mut image = ColorImage::filled([5, 3], Color32::from_rgb(10, 20, 30));
        image.pixels[7] = Color32::from_rgb(200, 100, 50);
        image
    }

    #[test]
    fn a_png_decodes_to_the_same_pixels() {
        let png = encode_png(&picture()).expect("encodes");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let decoded = image::load_from_memory(&png)
            .expect("a valid png")
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (5, 3));
        assert_eq!(decoded.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(decoded.get_pixel(2, 1).0, [200, 100, 50, 255]);
    }

    #[test]
    fn a_file_is_written_whole_or_not_at_all() {
        let dir = scratch("atomic");
        let path = dir.join("chart.png");
        write_atomic(&path, b"first").expect("written");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second, longer").expect("replaced");
        assert_eq!(std::fs::read(&path).unwrap(), b"second, longer");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [OsString::from("chart.png")], "no part file is left");
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_and_no_part_file() {
        let dir = scratch("failed");
        let path = dir.join("chart.png");
        write_atomic(&path, b"keep me").unwrap();
        // The rename cannot replace a directory with a file.
        let blocked = dir.join("blocked.png");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("inside"), b"x").unwrap();
        let error = write_atomic(&blocked, b"new").unwrap_err();
        assert!(matches!(error, ExportError::Write(_)), "{error:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_error_not_a_panic() {
        let dir = scratch("missing");
        let error = write_atomic(&dir.join("nope").join("chart.png"), b"x").unwrap_err();
        assert!(
            error.message().contains("does not exist"),
            "{}",
            error.message()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_written_to_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("readonly");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = write_atomic(&dir.join("chart.png"), b"x");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A superuser writes anywhere; everyone else is told no.
        if let Err(error) = outcome {
            assert!(
                error.message().contains("no permission"),
                "{}",
                error.message()
            );
        }
        assert!(!dir.join("chart.png.part").exists());
    }

    #[test]
    fn downloads_are_under_the_home_folder() {
        assert_eq!(
            downloads_folder(Some("/Users/me".into()), None),
            Some(PathBuf::from("/Users/me/Downloads"))
        );
        assert_eq!(
            downloads_folder(None, Some("C:\\Users\\me".into())),
            Some(PathBuf::from("C:\\Users\\me").join("Downloads"))
        );
        assert_eq!(downloads_folder(Some("".into()), None), None);
        assert_eq!(downloads_folder(None, None), None);
    }
}
