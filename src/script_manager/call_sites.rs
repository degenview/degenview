//! The lines a function or method is called on, as a list can show them: the line number and the
//! start of the line's code. Pure text work; every index is a character index into the text, like
//! the rest of the editor, so CRLF, tabs and astral characters need no special care by the caller.

use std::ops::Range;

/// The most characters of a line a row shows before it is cut with `…`.
const PREVIEW_CHARS: usize = 72;

/// One line that calls the function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallSite {
    /// One-based, as the gutter counts.
    pub line: usize,
    /// The line without its leading whitespace, cut to [`PREVIEW_CHARS`].
    pub preview: String,
    /// The first call on the line.
    pub range: Range<usize>,
    /// Where that call's name sits in `preview`, in characters; `None` when the cut took it off.
    pub name: Option<Range<usize>>,
    /// The function or method the call is in, for the caller to fill in (`None` at the top level).
    pub context: Option<String>,
}

impl CallSite {
    /// The row's text: `42  self.forget()`.
    pub fn label(&self) -> String {
        format!("{:>4}  {}", self.line, self.preview)
    }
}

/// The character index each line starts at. A line ends at `\n`, `\r\n` or a lone `\r`.
fn line_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, character) in chars.iter().enumerate() {
        let ends = match character {
            '\n' => true,
            '\r' => chars.get(index + 1) != Some(&'\n'),
            _ => false,
        };
        if ends {
            starts.push(index + 1);
        }
    }
    starts
}

/// One row for each line that holds a call in `ranges` (sorted by position): two calls on a line
/// are one place, and the row points at the first.
pub fn describe(source: &str, ranges: &[Range<usize>]) -> Vec<CallSite> {
    let chars: Vec<char> = source.chars().collect();
    let starts = line_starts(&chars);
    let mut sites: Vec<CallSite> = Vec::new();
    for range in ranges {
        let line = starts.partition_point(|start| *start <= range.start);
        if sites.last().is_some_and(|site| site.line == line) {
            continue;
        }
        let from = starts[line - 1];
        let to = chars[from..]
            .iter()
            .position(|c| matches!(c, '\n' | '\r'))
            .map_or(chars.len(), |length| from + length);
        let raw: String = chars[from..to]
            .iter()
            .map(|c| if *c == '\t' { ' ' } else { *c })
            .collect();
        let indent = raw.chars().take_while(|c| c.is_whitespace()).count();
        let text = raw.trim();
        let (preview, kept) = if text.chars().count() > PREVIEW_CHARS {
            let cut: String = text.chars().take(PREVIEW_CHARS).collect();
            let cut = cut.trim_end();
            (format!("{cut}…"), cut.chars().count())
        } else {
            (text.to_owned(), text.chars().count())
        };
        let length = range.end - range.start;
        let name = range
            .start
            .checked_sub(from + indent)
            .filter(|start| start + length <= kept)
            .map(|start| start..start + length);
        sites.push(CallSite {
            line,
            preview,
            range: range.clone(),
            name,
            context: None,
        });
    }
    sites
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The range of the `occurrence`th `needle` in `source`, in characters.
    fn find(source: &str, needle: &str, occurrence: usize) -> Range<usize> {
        let chars: Vec<char> = source.chars().collect();
        let needle: Vec<char> = needle.chars().collect();
        let mut seen = 0;
        for start in 0..chars.len() {
            if chars[start..].starts_with(&needle) {
                if seen == occurrence {
                    return start..start + needle.len();
                }
                seen += 1;
            }
        }
        panic!("no occurrence {occurrence} of {needle:?}");
    }

    #[test]
    fn a_row_is_the_one_based_line_and_the_code_without_its_indentation() {
        let source = "a = 1\nif a > 0\n        self.forget()\n";
        let sites = describe(source, &[find(source, "forget", 0)]);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].line, 3);
        assert_eq!(sites[0].preview, "self.forget()");
        assert_eq!(sites[0].label(), "   3  self.forget()");
        assert_eq!(sites[0].range, find(source, "forget", 0));
    }

    #[test]
    fn two_calls_on_one_line_are_one_place_pointing_at_the_first() {
        let source = "x = f(1) + f(2)\ny = f(3)\n";
        let ranges = [
            find(source, "f(", 0),
            find(source, "f(", 1),
            find(source, "f(", 2),
        ];
        let sites = describe(source, &ranges);
        assert_eq!(sites.iter().map(|s| s.line).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(sites[0].range, find(source, "f(", 0));
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let line = format!("x = call({})", "🙂".repeat(100));
        let source = format!("{line}\n");
        let sites = describe(&source, &[find(&source, "call", 0)]);
        assert_eq!(sites[0].preview.chars().count(), PREVIEW_CHARS + 1);
        assert!(sites[0].preview.ends_with('…'));
        assert!(sites[0].preview.starts_with("x = call("));
        // A line exactly as long as the limit is kept whole.
        let exact = "y".repeat(PREVIEW_CHARS);
        let source = format!("{exact}\n");
        let first = 0..1;
        let sites = describe(&source, std::slice::from_ref(&first));
        assert_eq!(sites[0].preview, exact);
    }

    #[test]
    fn crlf_cr_and_tabs_count_one_line_break_and_read_as_spaces() {
        let source = "a = 1\r\n\tb = f()\r\n\r\nc = f()\rd = f()";
        let ranges = [
            find(source, "f()", 0),
            find(source, "f()", 1),
            find(source, "f()", 2),
        ];
        let sites = describe(source, &ranges);
        assert_eq!(sites.iter().map(|s| s.line).collect::<Vec<_>>(), [2, 4, 5]);
        assert_eq!(sites[0].preview, "b = f()");
        assert_eq!(sites[1].preview, "c = f()");
        assert_eq!(sites[2].preview, "d = f()");
        let source = "x\t=\tf()";
        assert_eq!(
            describe(source, &[find(source, "f()", 0)])[0].preview,
            "x = f()"
        );
    }

    #[test]
    fn astral_characters_before_a_call_do_not_shift_its_line() {
        let source = "// 🙂🙂\nz = f()\n";
        let sites = describe(source, &[find(source, "f()", 0)]);
        assert_eq!((sites[0].line, sites[0].preview.as_str()), (2, "z = f()"));
    }

    #[test]
    fn the_called_name_is_located_in_the_preview() {
        let source = "a = 1\n\t\tb = half(2)\n";
        let sites = describe(source, &[find(source, "half", 0)]);
        let name = sites[0].name.clone().expect("the name is shown");
        let shown: String = sites[0]
            .preview
            .chars()
            .skip(name.start)
            .take(name.len())
            .collect();
        assert_eq!(
            (sites[0].preview.as_str(), shown.as_str()),
            ("b = half(2)", "half")
        );
        // A call cut off the end of a long line has no place in the preview.
        let source = format!("{}half(1)\n", "x".repeat(100));
        let sites = describe(&source, &[find(&source, "half", 0)]);
        assert_eq!(sites[0].name, None);
    }

    #[test]
    fn no_calls_no_rows() {
        assert!(describe("a = 1\n", &[]).is_empty());
    }
}
