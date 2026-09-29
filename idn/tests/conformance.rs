//! Runs the UTS #46 conformance tests from `data/IdnaTestV2.txt`

use std::borrow::Cow;
use std::fs;

use idn::{Config, Error};

#[test]
fn strict() {
    run(Config::strict(), &[]);
}

#[test]
fn whatwg() {
    run(Config::new(), &["V2", "V3", "U1", "A4_1", "A4_2"]);
}

/// Checks that the ASCII fast paths agree with the general processing path
///
/// U+00AD SOFT HYPHEN is ignored by the Map step, so appending it never changes the result, but
/// it makes the input non-ASCII, which forces the general path.
#[test]
fn fast_path_matches_general_path() {
    let data =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/IdnaTestV2.txt")).unwrap();
    let mut inputs = Vec::new();
    for line in data.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }

        if let Some(case) = Case::parse(line) {
            inputs.push(case.source);
        }
    }

    let alphabet = b"abcxnXN019-._=%A ";
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..100_000 {
        let mut input = String::new();
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        for i in 0..(state % 12) {
            input.push(char::from(
                alphabet[((state >> (i * 5)) % alphabet.len() as u64) as usize],
            ));
        }
        inputs.push(input);
    }

    for config in [Config::new(), Config::strict()] {
        for input in &inputs {
            let general = format!("{input}\u{ad}");
            assert_eq!(
                config.to_ascii(input),
                config.to_ascii(&general),
                "{input:?}"
            );
            assert_eq!(
                config.to_unicode(input),
                config.to_unicode(&general).map(|s| s.into_owned().into()),
                "{input:?}"
            );
        }
    }
}

#[test]
fn offsets() {
    let (default, strict) = (Config::new(), Config::strict());
    let long = vec!["a".repeat(63); 4].join(".");
    let long_unicode = format!("bücher.{long}");
    let long_punycode = format!("xn--{}", "a".repeat(2001));
    let cases = [
        // In labels that appear in the input unchanged, up to ASCII case and ignored code points,
        // offsets point at the offending code point.
        (default, "a.b\u{378}c.d", Error::DisallowedCodePoint(3)),
        (strict, "www.ex_ample.com", Error::Std3Rules(6)),
        (strict, "WWW.EX_AMPLE.COM", Error::Std3Rules(6)),
        (strict, "ü_ber.example", Error::Std3Rules(2)),
        (strict, "a\u{ad}_b", Error::Std3Rules(3)),
        (strict, "-abc.example", Error::Hyphen(0)),
        (strict, "abc-.example", Error::Hyphen(3)),
        (strict, "example.ab--ü", Error::Hyphen(10)),
        (default, "a.\u{301}b", Error::LeadingMark(2)),
        (default, "ab\u{200d}c.example", Error::Joiner(2)),
        (default, "example.\u{5d0}a", Error::Bidi(10)),
        (default, "\u{5d0}.1a", Error::Bidi(3)),
        (default, "\u{5d0}!", Error::Bidi(2)),
        (default, "\u{5d0}1\u{661}", Error::Bidi(3)),
        // Disallowed code points are always found in the input.
        (default, "ü。b\u{378}", Error::DisallowedCodePoint(6)),
        // Otherwise, offsets point at the start of the label.
        (strict, "a.Ü_ber", Error::Std3Rules(2)),
        (strict, "a.b\u{ff3f}c", Error::Std3Rules(2)),
        // Invalid Punycode is located in the encoded label: an invalid code point, the start of
        // a delta that cannot be decoded, or the first code point beyond the length limit.
        (default, "example.xn--ls8h!", Error::Punycode(16)),
        (default, "XN--LS8H!", Error::Punycode(8)),
        (default, "example.xn--ls8", Error::Punycode(12)),
        (default, "a.xn--99999999999", Error::Punycode(6)),
        (default, "xn--ü-abc", Error::Punycode(4)),
        (default, "a\u{3002}xn--ls8h!", Error::Punycode(12)),
        (default, &long_punycode, Error::Punycode(2004)),
        (default, "a.\u{ff58}n--ls8h!", Error::Punycode(2)),
        // Problems found after decoding Punycode point at the start of the label.
        (default, "example.xn--a-", Error::Punycode(8)),
        (default, "xn--u-ccb", Error::Punycode(0)),
        (default, "a.xn--a", Error::DisallowedCodePoint(2)),
        // U+FFFD REPLACEMENT CHARACTER is disallowed, even though normalization keeps it
        (default, "a.xn--zn7c", Error::DisallowedCodePoint(2)),
        (strict, "xn--x-xbb6045q", Error::DisallowedCodePoint(0)),
        (default, "a.xn--xn---epa", Error::Hyphen(2)),
        (strict, "a.xn--xn---epa", Error::Hyphen(2)),
        (default, "example.xn--a-0hc", Error::Bidi(8)),
        // Labels are also separated by code points that are mapped to U+002E FULL STOP.
        (default, "a\u{ff61}b\u{200c}c", Error::Joiner(5)),
        (strict, "a\u{3002}-b", Error::Hyphen(4)),
        (strict, "a\u{3002}\u{3002}b", Error::LabelLength(4)),
        // Length limits apply to the ASCII form, but offsets point into the input.
        (strict, "a..b", Error::LabelLength(2)),
        (strict, "a.b.", Error::LabelLength(4)),
        (strict, "", Error::DomainLength(0)),
        (strict, &long, Error::DomainLength(192)),
        (strict, &long_unicode, Error::DomainLength(200)),
    ];

    for (config, input, expected) in cases {
        assert_eq!(config.to_ascii(input), Err(expected), "{input:?}");
        if !matches!(expected, Error::LabelLength(_) | Error::DomainLength(_)) {
            assert_eq!(config.to_unicode(input), Err(expected), "{input:?}");
        }
    }

    // Unicode labels are only limited by encoding, so this points at the first code point beyond
    // the limit
    let long_label = "é".repeat(2001);
    assert_eq!(default.to_ascii(&long_label), Err(Error::Punycode(4000)));
}

/// A label that can be decoded can be encoded again, whatever the other labels are
///
/// Only a domain name with a non-ASCII label needs Punycode labels to be encoded again.
#[test]
fn long_punycode_label() {
    let config = Config::new();
    let label = config
        .to_ascii(&format!("{}é", "a".repeat(1500)))
        .unwrap()
        .into_owned();
    assert_eq!(config.to_ascii(&label).unwrap(), label);
    assert_eq!(
        config.to_ascii(&format!("{label}.bücher")).unwrap(),
        format!("{label}.xn--bcher-kva")
    );
}

#[test]
fn display() {
    assert_eq!(
        Error::Hyphen(3).to_string(),
        "invalid domain name at offset 3: misplaced hyphen"
    );
}

/// Runs all test cases with `config`, ignoring the status codes for flags it disables
///
/// The test file notes that implementations need only record that there was an error, but it
/// lists all of them, so the error that is reported must correspond to one of the status codes.
fn run(config: Config, ignored: &[&str]) {
    let data =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/IdnaTestV2.txt")).unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for (i, line) in data.lines().enumerate() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }

        let Some(case) = Case::parse(line) else {
            continue;
        };

        count += 1;
        let Case {
            source,
            to_unicode,
            to_unicode_status,
            to_ascii,
            to_ascii_status,
        } = case;

        // X4_2 marks empty labels, which _ToUnicode_ is not required to report.
        let mut unicode_ignored = ignored.to_vec();
        unicode_ignored.push("X4_2");
        let expected = expectation(to_unicode, &to_unicode_status, &unicode_ignored);
        let actual = config.to_unicode(&source);
        if !is_expected(&source, &actual, &expected) {
            failures.push(format!(
                "line {}: to_unicode({source:?}): expected {expected:?}, got {actual:?}",
                i + 1
            ));
        }

        let expected = expectation(to_ascii, &to_ascii_status, ignored);
        let actual = config.to_ascii(&source);
        if !is_expected(&source, &actual, &expected) {
            failures.push(format!(
                "line {}: to_ascii({source:?}): expected {expected:?}, got {actual:?}",
                i + 1
            ));
        }
    }

    assert!(count > 6000, "only {count} test cases found");
    if !failures.is_empty() {
        for failure in failures.iter().take(50) {
            eprintln!("{failure}");
        }
        panic!("{} of {count} test cases failed", failures.len());
    }
}

/// Returns the expected value, or the status codes that are not ignored if there are any
fn expectation(value: String, status: &[String], ignored: &[&str]) -> Result<String, Vec<String>> {
    let codes = status
        .iter()
        .filter(|code| !ignored.contains(&code.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    match codes.is_empty() {
        true => Ok(value),
        false => Err(codes),
    }
}

/// Whether `actual` is the expected value, or an error for one of the expected status codes
///
/// The offset of an error must also be at a code point boundary in `source`.
fn is_expected(
    source: &str,
    actual: &Result<Cow<'_, str>, Error>,
    expected: &Result<String, Vec<String>>,
) -> bool {
    match (actual, expected) {
        (Ok(actual), Ok(expected)) => actual == expected,
        (Err(error), Err(codes)) => {
            source.is_char_boundary(error.offset())
                && codes
                    .iter()
                    .any(|code| status_codes(error).contains(&code.as_str()))
        }
        _ => false,
    }
}

/// The status codes for the checks that can cause `error`
fn status_codes(error: &Error) -> &'static [&'static str] {
    match error {
        Error::Punycode(_) => &["P4", "V1", "A3"],
        Error::Hyphen(_) => &["V2", "V3", "V4"],
        Error::LeadingMark(_) => &["V6"],
        Error::DisallowedCodePoint(_) => &["V7"],
        Error::Std3Rules(_) => &["U1"],
        Error::Joiner(_) => &["C1", "C2"],
        Error::Bidi(_) => &["B1", "B2", "B3", "B4", "B5", "B6"],
        Error::DomainLength(_) => &["A4_1"],
        Error::LabelLength(_) => &["A4_2"],
        _ => &[],
    }
}

struct Case {
    source: String,
    to_unicode: String,
    to_unicode_status: Vec<String>,
    to_ascii: String,
    to_ascii_status: Vec<String>,
}

impl Case {
    /// Parses a test case, returning `None` if it contains ill-formed strings
    fn parse(line: &str) -> Option<Self> {
        let fields = line.split(';').map(str::trim).collect::<Vec<_>>();
        let source = unescape(fields[0])?;
        let to_unicode = match fields[1] {
            "" => source.clone(),
            value => unescape(value)?,
        };
        let to_unicode_status = status(fields[2]).unwrap_or_default();
        let to_ascii = match fields[3] {
            "" => to_unicode.clone(),
            value => unescape(value)?,
        };
        let to_ascii_status = status(fields[4]).unwrap_or_else(|| to_unicode_status.clone());
        Some(Self {
            source,
            to_unicode,
            to_unicode_status,
            to_ascii,
            to_ascii_status,
        })
    }
}

/// Parses a status field; a blank field yields `None`
fn status(field: &str) -> Option<Vec<String>> {
    let inner = field.strip_prefix('[')?.strip_suffix(']').unwrap();
    let mut codes = Vec::new();
    for code in inner.split(',') {
        let code = code.trim();
        if !code.is_empty() {
            codes.push(code.to_owned());
        }
    }

    Some(codes)
}

/// Resolves `\uXXXX` and `\x{X...}` escapes, returning `None` for unpaired surrogates
fn unescape(field: &str) -> Option<String> {
    if field == "\"\"" {
        return Some(String::new());
    }

    let mut out = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }

        let hex = match chars.next() {
            Some('u') => (0..4).map(|_| chars.next().unwrap()).collect::<String>(),
            Some('x') => {
                assert_eq!(chars.next(), Some('{'));
                chars.by_ref().take_while(|&c| c != '}').collect()
            }
            other => panic!("unexpected escape {other:?} in {field:?}"),
        };

        out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap())?);
    }

    Some(out)
}
