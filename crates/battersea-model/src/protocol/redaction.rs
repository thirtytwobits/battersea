//! Match diagnostic encodings while retaining the source spans that must be masked.
use std::{collections::VecDeque, ops::Range};

#[derive(Clone, Copy)]
struct Byte {
    value: u8,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
enum Encoding {
    Uri { form: bool },
    Json,
}

struct Decoder<I> {
    source: I,
    lookahead: VecDeque<Byte>,
    output: VecDeque<Byte>,
    encoding: Encoding,
}
impl<I: Iterator<Item = Byte>> Decoder<I> {
    fn new(source: I, encoding: Encoding) -> Self {
        Self {
            source,
            lookahead: VecDeque::new(),
            output: VecDeque::new(),
            encoding,
        }
    }
    fn peek(&mut self, index: usize) -> Option<Byte> {
        while self.lookahead.len() <= index {
            self.lookahead.push_back(self.source.next()?);
        }
        self.lookahead.get(index).copied()
    }
    fn take(&mut self) -> Option<Byte> {
        self.lookahead.pop_front().or_else(|| self.source.next())
    }
    fn hex_quad(&mut self, start: usize) -> Option<u32> {
        let mut value = 0;
        for index in start..start + 4 {
            value = value * 16 + u32::from(hex(self.peek(index)?.value)?);
        }
        Some(value)
    }
    // The leading backslash has been consumed; lookahead begins at the escape code.
    fn json_escape(&mut self) -> Option<(char, usize)> {
        Some(match self.peek(0)?.value {
            b'"' => ('"', 1),
            b'\\' => ('\\', 1),
            b'/' => ('/', 1),
            b'b' => ('\u{0008}', 1),
            b'f' => ('\u{000c}', 1),
            b'n' => ('\n', 1),
            b'r' => ('\r', 1),
            b't' => ('\t', 1),
            b'u' => {
                let high = self.hex_quad(1)?;
                if (0xd800..=0xdbff).contains(&high) {
                    if self.peek(5)?.value != b'\\' || self.peek(6)?.value != b'u' {
                        return None;
                    }
                    let low = self.hex_quad(7)?;
                    if !(0xdc00..=0xdfff).contains(&low) {
                        return None;
                    }
                    (
                        char::from_u32(0x10000 + ((high - 0xd800) << 10) + low - 0xdc00)?,
                        11,
                    )
                } else {
                    (char::from_u32(high)?, 5)
                }
            }
            _ => return None,
        })
    }
}
impl<I: Iterator<Item = Byte>> Iterator for Decoder<I> {
    type Item = Byte;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(byte) = self.output.pop_front() {
            return Some(byte);
        }
        let first = self.take()?;
        match (self.encoding, first.value) {
            (Encoding::Uri { .. }, b'%') => {
                if let Some((high, low)) = self.peek(0).and_then(|a| {
                    self.peek(1)
                        .and_then(|b| Some((hex(a.value)?, hex(b.value)?)))
                }) {
                    self.take();
                    let last = self.take().unwrap();
                    return Some(Byte {
                        value: high * 16 + low,
                        end: last.end,
                        ..first
                    });
                }
            }
            (Encoding::Uri { form: true }, b'+') => {
                return Some(Byte {
                    value: b' ',
                    ..first
                });
            }
            (Encoding::Json, b'\\') => {
                if let Some((character, length)) = self.json_escape() {
                    let end = self.peek(length - 1).unwrap().end;
                    for _ in 0..length {
                        self.take();
                    }
                    let mut bytes = [0; 4];
                    for &value in character.encode_utf8(&mut bytes).as_bytes() {
                        self.output.push_back(Byte {
                            value,
                            end,
                            ..first
                        });
                    }
                    return self.output.pop_front();
                }
            }
            _ => (),
        }
        Some(first)
    }
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct Matches<'a> {
    secret: &'a [u8],
    prefix: Vec<usize>,
    spans: Vec<Range<usize>>,
}
impl<'a> Matches<'a> {
    fn new(secret: &'a str) -> Self {
        let secret = secret.as_bytes();
        let mut prefix = vec![0; secret.len()];
        let mut matched = 0;
        // Prefix links avoid rescanning diagnostic bytes after a partial match.
        for index in 1..secret.len() {
            while matched > 0 && secret[index] != secret[matched] {
                matched = prefix[matched - 1];
            }
            if secret[index] == secret[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self {
            secret,
            prefix,
            spans: Vec::new(),
        }
    }
    fn scan(&mut self, source: impl Iterator<Item = Byte>) {
        let mut starts = VecDeque::new();
        let mut matched = 0;
        let scan_start = self.spans.len();
        for byte in source {
            starts.push_back(byte.start);
            if starts.len() > self.secret.len() {
                starts.pop_front();
            }
            while matched > 0 && byte.value != self.secret[matched] {
                matched = self.prefix[matched - 1];
            }
            if byte.value == self.secret[matched] {
                matched += 1;
            }
            if matched == self.secret.len() {
                let start = *starts.front().unwrap();
                if self.spans.len() > scan_start && start <= self.spans.last().unwrap().end {
                    let previous = self.spans.last_mut().unwrap();
                    previous.end = previous.end.max(byte.end);
                } else {
                    self.spans.push(start..byte.end);
                }
                matched = self.prefix[matched - 1];
            }
        }
    }
    fn mask(mut self, text: &str) -> String {
        self.spans.sort_unstable_by_key(|span| span.start);
        let mut spans = self.spans.into_iter();
        let Some(mut masked) = spans.next() else {
            return text.to_owned();
        };
        let mut result = String::new();
        let mut copied = 0;
        for span in spans {
            if span.start <= masked.end {
                masked.end = masked.end.max(span.end);
            } else {
                result.push_str(&text[copied..masked.start]);
                result.push_str("[REDACTED]");
                copied = masked.end;
                masked = span;
            }
        }
        result.push_str(&text[copied..masked.start]);
        result.push_str("[REDACTED]");
        result.push_str(&text[masked.end..]);
        result
    }
}

pub(super) fn redact(secret: &str, text: &str) -> String {
    if secret.is_empty() {
        return text.to_owned();
    }
    let source = || {
        text.bytes().enumerate().map(|(start, value)| Byte {
            value,
            start,
            end: start + 1,
        })
    };
    let mut matches = Matches::new(secret);
    matches.scan(source());
    let percent = text.contains('%');
    let plus = text.contains('+');
    let backslash = text.contains('\\');
    if backslash {
        matches.scan(Decoder::new(source(), Encoding::Json));
    }
    for form in [false, true] {
        if percent || (form && plus) {
            matches.scan(Decoder::new(source(), Encoding::Uri { form }));
            // URI-encoded JSON escapes can occur inside a diagnostic's query values.
            matches.scan(Decoder::new(
                Decoder::new(source(), Encoding::Uri { form }),
                Encoding::Json,
            ));
        }
        if backslash {
            matches.scan(Decoder::new(
                Decoder::new(source(), Encoding::Json),
                Encoding::Uri { form },
            ));
        }
    }
    matches.mask(text)
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn uri_and_form_representations_accept_mixed_hex_case_and_partial_encoding() {
        let secret = "account~+ /token";
        for encoded in [
            secret,
            "account%7E%2B+%2Ftoken",
            "account%7e%2b+%2ftoken",
            "account~%2B%20%2Ftoken",
            "account~%2b%20%2ftoken",
            "%61ccount%7e%2B+%2ftoken",
        ] {
            assert_eq!(
                redact(secret, &format!("rejected {encoded}; retry")),
                "rejected [REDACTED]; retry"
            );
        }
        assert_eq!(
            redact(secret, "Account~%2b%20%2ftoken"),
            "Account~%2b%20%2ftoken"
        );
    }

    #[test]
    fn json_escapes_include_backslashes_quotes_slashes_controls_and_surrogate_pairs() {
        for (secret, encoded) in [
            (r#"account\path/"token"#, r#"account\\path\/\"token"#),
            (
                r#"account\path/"token"#,
                r#"account\u005cpath\u002f\u0022token"#,
            ),
            ("clé/🚀", r"cl\u00e9\/\ud83d\ude80"),
            ("clé/🚀", r"cl\u00E9\u002F\uD83D\uDE80"),
            ("key\n\r\t\u{0008}\u{000c}\0", r"key\n\r\t\b\f\u0000"),
        ] {
            assert_eq!(
                redact(secret, &format!("rejected {encoded}; retry")),
                "rejected [REDACTED]; retry"
            );
        }
    }

    #[test]
    fn json_and_uri_encodings_compose_without_changing_credential_bytes() {
        let secrets = [r"key\path+ /~", "%2B\\n+%20", "clé/🚀"]
            .into_iter()
            .map(str::to_owned)
            .chain((0..=127u8).map(|byte| format!("key{}token", char::from(byte))));
        for secret in secrets {
            let json = serde_json::to_string(&secret).unwrap();
            let escaped = &json[1..json.len() - 1];
            let form = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect::<String>();
            let json_form = serde_json::to_string(&form)
                .unwrap()
                .replace('%', r"\u0025");
            let json_form = json_form[1..json_form.len() - 1].to_owned();
            let form_json =
                url::form_urlencoded::byte_serialize(escaped.as_bytes()).collect::<String>();
            for encoded in [
                secret.clone(),
                escaped.to_owned(),
                form,
                json_form,
                form_json,
            ] {
                assert_eq!(
                    redact(&secret, &format!("rejected {encoded}; retry")),
                    "rejected [REDACTED]; retry"
                );
            }
        }
    }

    #[test]
    fn malformed_encodings_remain_diagnostic_text_and_overlaps_are_masked() {
        let malformed = r"a%2G% a\uD800 a\uDC00 a\u12 a\q";
        assert_eq!(redact("token", malformed), malformed);
        assert_eq!(redact("", malformed), malformed);
        assert_eq!(redact("token", ""), "");
        assert_eq!(redact("aba", "ababa"), "[REDACTED]");
        assert_eq!(redact("aaaa", &"a".repeat(20_000)), "[REDACTED]");
        assert_eq!(redact("token", "token+token"), "[REDACTED]+[REDACTED]");
        let secret = format!("{}b", "a".repeat(256));
        let text = format!("{} {secret}", "a".repeat(20_000));
        assert_eq!(
            redact(&secret, &text),
            format!("{} [REDACTED]", "a".repeat(20_000))
        );
    }
}
