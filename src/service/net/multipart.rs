//! A `multipart/form-data` body (RFC 7578), built by hand so no dependency is needed for it.
//!
//! Image hosts that are not DegenView's own take their upload as a form: a few text fields and
//! the file. A provider builds one with [`Form`], and the result is a request body and the
//! `Content-Type` that names its boundary.

/// What a form field's name and a file's name may not hold: they sit inside quotes in a header,
/// so a quote, a line break or a backslash would end or bend it.
fn header_safe(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .collect()
}

/// A form being assembled.
pub struct Form {
    boundary: String,
    body: Vec<u8>,
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    pub fn new() -> Self {
        // 24 random bytes, hex: no text field or image can be expected to contain it.
        let mut random = [0u8; 24];
        if getrandom::fill(&mut random).is_err() {
            // Without a random source the boundary is still unique enough for one request: the
            // clock differs between forms (and the rest is zeros).
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            random[..16].copy_from_slice(&nanos.to_le_bytes());
        }
        let boundary: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Self::with_boundary(format!("----DegenView{boundary}"))
    }

    fn with_boundary(boundary: String) -> Self {
        Self {
            boundary,
            body: Vec::new(),
        }
    }

    fn part_head(&mut self, name: &str, file_name: Option<&str>, content_type: Option<&str>) {
        self.body.extend_from_slice(b"--");
        self.body.extend_from_slice(self.boundary.as_bytes());
        self.body.extend_from_slice(b"\r\n");
        self.body
            .extend_from_slice(b"Content-Disposition: form-data; name=\"");
        self.body.extend_from_slice(header_safe(name).as_bytes());
        self.body.push(b'"');
        if let Some(file_name) = file_name {
            self.body.extend_from_slice(b"; filename=\"");
            self.body
                .extend_from_slice(header_safe(file_name).as_bytes());
            self.body.push(b'"');
        }
        self.body.extend_from_slice(b"\r\n");
        if let Some(content_type) = content_type {
            self.body.extend_from_slice(b"Content-Type: ");
            self.body
                .extend_from_slice(header_safe(content_type).as_bytes());
            self.body.extend_from_slice(b"\r\n");
        }
        self.body.extend_from_slice(b"\r\n");
    }

    /// A text field. Not every provider's form has one (ImgBB's carries its settings in the
    /// query), but others (an upload preset, a signature) do.
    #[allow(dead_code)]
    pub fn text(mut self, name: &str, value: &str) -> Self {
        self.part_head(name, None, None);
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
        self
    }

    /// A file field, its bytes sent as they are.
    pub fn file(mut self, name: &str, file_name: &str, content_type: &str, bytes: &[u8]) -> Self {
        self.part_head(name, Some(file_name), Some(content_type));
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    /// The `Content-Type` that goes with the body.
    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    /// The finished body.
    pub fn finish(mut self) -> Vec<u8> {
        self.body.extend_from_slice(b"--");
        self.body.extend_from_slice(self.boundary.as_bytes());
        self.body.extend_from_slice(b"--\r\n");
        self.body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form::with_boundary("XYZ".to_owned())
    }

    #[test]
    fn a_form_is_parts_each_behind_the_boundary_and_a_closing_one() {
        let body = form()
            .text("expiration", "600")
            .file(
                "image",
                "snapshot.png",
                "image/png",
                b"\x89PNG\r\n\x1a\nDATA",
            )
            .finish();
        let expected = b"--XYZ\r\n\
Content-Disposition: form-data; name=\"expiration\"\r\n\
\r\n\
600\r\n\
--XYZ\r\n\
Content-Disposition: form-data; name=\"image\"; filename=\"snapshot.png\"\r\n\
Content-Type: image/png\r\n\
\r\n\
\x89PNG\r\n\x1a\nDATA\r\n\
--XYZ--\r\n";
        assert_eq!(
            String::from_utf8_lossy(&body),
            String::from_utf8_lossy(expected)
        );
        assert_eq!(body, expected);
    }

    #[test]
    fn the_content_type_names_the_boundary() {
        assert_eq!(form().content_type(), "multipart/form-data; boundary=XYZ");
    }

    #[test]
    fn a_name_cannot_break_out_of_its_quotes() {
        let body = form().text("a\"b\r\nX-Evil: 1", "v").finish();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("name=\"abX-Evil: 1\"\r\n"), "{text}");
        assert!(!text.contains("a\"b"), "{text}");
        // Exactly one header line was written for the part.
        assert_eq!(text.matches("Content-Disposition").count(), 1);
    }

    #[test]
    fn every_form_gets_its_own_boundary() {
        let (a, b) = (Form::new(), Form::new());
        assert_ne!(a.content_type(), b.content_type());
        assert!(
            a.content_type()
                .starts_with("multipart/form-data; boundary=----DegenView")
        );
    }

    #[test]
    fn an_empty_form_is_just_its_closing_boundary() {
        assert_eq!(form().finish(), b"--XYZ--\r\n");
    }
}
