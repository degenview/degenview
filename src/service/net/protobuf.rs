//! Just enough of the Protocol Buffers wire format to read the few messages an exchange pushes
//! (MEXC's spot socket is protobuf-only): a message is a run of `(field, value)` pairs, where a
//! value is a varint or a length-delimited run of bytes. Fixed-width fields are skipped.

/// One field's value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value<'a> {
    Varint(u64),
    /// A string, bytes or a nested message.
    Bytes(&'a [u8]),
}

impl<'a> Value<'a> {
    pub fn varint(self) -> Option<u64> {
        match self {
            Self::Varint(value) => Some(value),
            Self::Bytes(_) => None,
        }
    }

    pub fn bytes(self) -> Option<&'a [u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::Varint(_) => None,
        }
    }

    pub fn text(self) -> Option<&'a str> {
        std::str::from_utf8(self.bytes()?).ok()
    }
}

/// The fields of a message in order. A malformed message ends the run where it breaks.
pub fn fields(message: &[u8]) -> Fields<'_> {
    Fields { rest: message }
}

pub struct Fields<'a> {
    rest: &'a [u8],
}

impl<'a> Fields<'a> {
    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for (index, byte) in self.rest.iter().enumerate().take(10) {
            value |= u64::from(byte & 0x7f) << (7 * index);
            if byte & 0x80 == 0 {
                self.rest = &self.rest[index + 1..];
                return Some(value);
            }
        }
        None
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.rest.split_at_checked(length)?;
        self.rest = rest;
        Some(taken)
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.rest.is_empty() {
                return None;
            }
            let tag = self.varint()?;
            let field = u32::try_from(tag >> 3).ok()?;
            match tag & 7 {
                0 => return Some((field, Value::Varint(self.varint()?))),
                2 => {
                    let length = usize::try_from(self.varint()?).ok()?;
                    return Some((field, Value::Bytes(self.take(length)?)));
                }
                // Fixed 64 and 32 bits: not read, only stepped over.
                1 => {
                    self.take(8)?;
                }
                5 => {
                    self.take(4)?;
                }
                _ => {
                    self.rest = &[];
                    return None;
                }
            }
        }
    }
}

/// The first field `number` of `message`.
pub fn field(message: &[u8], number: u32) -> Option<Value<'_>> {
    fields(message).find_map(|(found, value)| (found == number).then_some(value))
}

/// The other direction, for tests that need a frame to read.
#[cfg(test)]
pub(crate) mod encode {
    fn varint_bytes(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    /// A string field.
    pub fn text(number: u32, text: &str) -> Vec<u8> {
        nested(number, text.as_bytes())
    }

    /// A varint field.
    pub fn varint(number: u32, value: u64) -> Vec<u8> {
        let mut out = varint_bytes(u64::from(number) << 3);
        out.extend(varint_bytes(value));
        out
    }

    /// A length-delimited field: a string, bytes or a nested message.
    pub fn nested(number: u32, bytes: &[u8]) -> Vec<u8> {
        let mut out = varint_bytes(u64::from(number) << 3 | 2);
        out.extend(varint_bytes(bytes.len() as u64));
        out.extend(bytes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::encode::{nested, text, varint};
    use super::*;

    #[test]
    fn reads_strings_varints_and_nested_messages_in_order() {
        let mut inner = text(1, "Min1");
        inner.extend(varint(2, 1_794_920_160));
        let mut message = text(1, "channel");
        message.extend(text(3, "BTCUSDT"));
        message.extend(varint(5, 1_794_920_181_910));
        // A field numbered above 127 takes a two-byte tag, as the kline body (308) does.
        message.extend(nested(308, &inner));

        let all: Vec<(u32, Value)> = fields(&message).collect();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0], (1, Value::Bytes(b"channel")));
        assert_eq!(field(&message, 3).and_then(Value::text), Some("BTCUSDT"));
        assert_eq!(
            field(&message, 5).and_then(Value::varint),
            Some(1_794_920_181_910)
        );
        let body = field(&message, 308).and_then(Value::bytes).unwrap();
        assert_eq!(field(body, 1).and_then(Value::text), Some("Min1"));
        assert_eq!(field(body, 2).and_then(Value::varint), Some(1_794_920_160));
        assert_eq!(field(&message, 9), None);
    }

    #[test]
    fn fixed_width_fields_are_stepped_over() {
        let mut message = vec![0x09];
        message.extend([0u8; 8]);
        message.extend(vec![0x15]);
        message.extend([0u8; 4]);
        message.extend(text(2, "after"));
        let all: Vec<(u32, Value)> = fields(&message).collect();
        assert_eq!(all, [(2, Value::Bytes(b"after"))]);
    }

    #[test]
    fn a_broken_message_ends_the_run_where_it_breaks() {
        // A length that runs past the end.
        let truncated = [0x0a, 0x10, b'a'];
        assert_eq!(fields(&truncated).count(), 0);
        // A varint that never ends.
        assert_eq!(fields(&[0x08, 0x80, 0x80]).count(), 0);
        // A group, which this reader does not know.
        assert_eq!(fields(&[0x0b, 0x01]).count(), 0);
        assert_eq!(fields(&[]).count(), 0);
        let mut good = text(1, "ok");
        good.extend([0x0a, 0x09]);
        assert_eq!(fields(&good).count(), 1);
    }
}
