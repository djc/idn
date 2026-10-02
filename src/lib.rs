//! Internationalized Domain Names in Applications (IDNA), as specified by [UTS #46]
//!
//! This crate converts domain names between their Unicode form and the ASCII-compatible form
//! used in the DNS, applying the mapping, normalization and validation steps of UTS #46 with
//! Nontransitional Processing. It has no dependencies, does not require `std` (only `alloc`),
//! and contains no `unsafe` code.
//!
//! ```
//! let config = idn::Config::new();
//! assert_eq!(config.to_ascii("Bücher.example").unwrap(), "xn--bcher-kva.example");
//! assert_eq!(config.to_unicode("xn--bcher-kva.example").unwrap(), "bücher.example");
//! assert!(config.to_ascii("xn--a.example").is_err());
//! ```
//!
//! The underlying Unicode data and algorithms are also available, which is useful for implementing
//! other UTS #46 processing front ends: the Map and Normalize steps ([`Normalize`]), and the
//! character properties used by the validity criteria ([`BidiClass`], [`JoiningType`],
//! [`is_mark()`] and [`is_virama()`]). Together, they suffice to implement the interface of the
//! [`idna_adapter`] crate, so that the `idna` crate can use this crate as its Unicode back end.
//!
//! [UTS #46]: https://www.unicode.org/reports/tr46/
//! [`idna_adapter`]: https://docs.rs/idna_adapter

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;
use core::error::Error as StdError;
use core::fmt;

mod punycode;

#[rustfmt::skip]
mod tables;

mod unicode;
pub use unicode::{BidiClass, JoiningType, Mode, Normalize, is_mark, is_virama};
use unicode::{Normalizer, REPLACEMENT};

/// Options for UTS #46 processing
///
/// [`Config::new()`] matches the options used by the [WHATWG URL Standard] for host parsing.
/// [`Config::strict()`] enables all checks, which is what the UTS #46 conformance tests assume.
///
/// Some UTS #46 options cannot be changed: _CheckBidi_ and _CheckJoiners_ are always enabled,
/// while _Transitional_Processing_ (deprecated) and _IgnoreInvalidPunycode_ are always disabled.
///
/// [WHATWG URL Standard]: https://url.spec.whatwg.org/#concept-domain-to-ascii
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    std3_rules: Std3Rules,
    hyphens: Hyphens,
    dns_length: DnsLength,
}

impl Config {
    /// Creates a configuration with the options used by the WHATWG URL Standard
    ///
    /// This uses [`Std3Rules::Ignore`], [`Hyphens::Allow`] and [`DnsLength::Ignore`].
    pub const fn new() -> Self {
        Self {
            std3_rules: Std3Rules::Ignore,
            hyphens: Hyphens::Allow,
            dns_length: DnsLength::Ignore,
        }
    }

    /// Creates a configuration with all checks enabled
    ///
    /// This uses [`Std3Rules::Apply`], [`Hyphens::Check`] and [`DnsLength::Verify`].
    pub const fn strict() -> Self {
        Self {
            std3_rules: Std3Rules::Apply,
            hyphens: Hyphens::Check,
            dns_length: DnsLength::Verify,
        }
    }

    /// Sets whether the STD 3 restrictions on ASCII code points apply
    #[must_use]
    pub const fn with_std3_rules(self, std3_rules: Std3Rules) -> Self {
        Self { std3_rules, ..self }
    }

    /// Sets whether hyphen placement is checked
    #[must_use]
    pub const fn with_hyphens(self, hyphens: Hyphens) -> Self {
        Self { hyphens, ..self }
    }

    /// Sets whether the DNS length limits are verified by [`Config::to_ascii()`]
    #[must_use]
    pub const fn with_dns_length(self, dns_length: DnsLength) -> Self {
        Self { dns_length, ..self }
    }

    /// Converts `domain` to its ASCII form (the UTS #46 _ToASCII_ operation)
    ///
    /// Returns a borrowed string if `domain` is already in its ASCII form.
    ///
    /// # Errors
    ///
    /// Returns an error if `domain` is not a valid domain name under UTS #46 processing with
    /// these options.
    pub fn to_ascii<'a>(&self, domain: &'a str) -> Result<Cow<'a, str>, Error> {
        let ascii = match self.ascii_fast_path(domain) {
            Some(ascii) => ascii?,
            // Punycode encodings are unique (RFC 3492, section 6.2), so the ASCII form of a valid
            // ASCII domain name is its lowercased form.
            None if domain.is_ascii() => {
                self.process(domain)?;
                match domain.bytes().any(|b| b.is_ascii_uppercase()) {
                    true => Cow::Owned(domain.to_ascii_lowercase()),
                    false => Cow::Borrowed(domain),
                }
            }
            None => {
                let unicode = self.process(domain)?;
                // Punycode labels are usually longer than their UTF-8 form, so this estimate
                // covers typical domain names to avoid reallocating
                let mut ascii = String::with_capacity(4 * unicode.len() + 4);
                for (i, label) in unicode.split(|&c| c == '.').enumerate() {
                    if i > 0 {
                        ascii.push('.');
                    }

                    match label.iter().all(char::is_ascii) {
                        true => ascii.extend(label),
                        false => {
                            ascii.push_str("xn--");
                            punycode::encode_into(label, &mut ascii).map_err(|index| {
                                Error::Punycode(index).locate_decoded(domain, i)
                            })?;
                        }
                    }
                }

                Cow::Owned(ascii)
            }
        };

        match self.dns_length {
            DnsLength::Ignore => {}
            DnsLength::Verify => verify_dns_length(domain, &ascii)?,
        }

        Ok(ascii)
    }

    /// Converts `domain` to its Unicode form (the UTS #46 _ToUnicode_ operation)
    ///
    /// Returns a borrowed string if `domain` is already in its Unicode form.
    ///
    /// # Errors
    ///
    /// Returns an error if `domain` is not a valid domain name under UTS #46 processing with
    /// these options. Unlike the _ToUnicode_ operation in UTS #46, no converted string is
    /// returned in that case.
    pub fn to_unicode<'a>(&self, domain: &'a str) -> Result<Cow<'a, str>, Error> {
        if let Some(unicode) = self.ascii_fast_path(domain) {
            return unicode;
        }

        let unicode = self.process(domain)?;
        let unchanged = domain.chars().eq(unicode.iter().copied());
        Ok(match unchanged {
            true => Cow::Borrowed(domain),
            false => {
                // Collecting would reserve a single byte per code point
                let mut out = String::with_capacity(unicode.iter().map(|c| c.len_utf8()).sum());
                out.extend(unicode);
                Cow::Owned(out)
            }
        })
    }

    /// Processes an ASCII `domain` without Punycode labels in a single pass
    ///
    /// For ASCII input, the Map step only lowercases, Normalize has no effect, and no label can
    /// make the domain name a Bidi domain name. Unless a label needs to be decoded, the output
    /// of both _ToASCII_ and _ToUnicode_ is therefore the lowercased input. Returns `None` if
    /// `domain` needs the general processing path.
    ///
    /// Domain names consisting only of lowercase letters, digits and dots cannot violate any of
    /// the validity criteria, so they are returned unchanged right away.
    fn ascii_fast_path<'a>(&self, domain: &'a str) -> Option<Result<Cow<'a, str>, Error>> {
        let bytes = domain.as_bytes();
        if bytes
            .iter()
            .all(|&b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.'))
        {
            return Some(Ok(Cow::Borrowed(domain)));
        }

        let mut uppercase = false;
        let mut non_ldh = false;
        let mut start = 0;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'a'..=b'z' | b'0'..=b'9' | b'-' => {}
                b'A'..=b'Z' => uppercase = true,
                b'.' => {
                    if let Err(error) = self.check_ascii_label(&bytes[start..i], start, non_ldh)? {
                        return Some(Err(error));
                    }
                    start = i + 1;
                    non_ldh = false;
                }
                0x80.. => return None,
                _ => non_ldh = true,
            }
        }

        if let Err(error) = self.check_ascii_label(&bytes[start..], start, non_ldh)? {
            return Some(Err(error));
        }

        Some(Ok(match uppercase {
            true => Cow::Owned(domain.to_ascii_lowercase()),
            false => Cow::Borrowed(domain),
        }))
    }

    /// Checks the validity criteria for an ASCII label at byte `offset` of the input
    ///
    /// This is part of [`Config::ascii_fast_path()`]. `non_ldh` indicates whether the label
    /// contains code points other than letters, digits and hyphens. Returns `None` for a Punycode
    /// label, which needs the general processing path. The criteria are checked in the same order
    /// as by [`Config::check_label()`], so that both report the same error.
    fn check_ascii_label(
        &self,
        label: &[u8],
        offset: usize,
        non_ldh: bool,
    ) -> Option<Result<(), Error>> {
        if has_punycode_prefix(label) {
            return None;
        }

        let hyphen = match self.hyphens {
            Hyphens::Check if label.first() == Some(&b'-') => Some(0),
            Hyphens::Check if label.last() == Some(&b'-') => Some(label.len() - 1),
            Hyphens::Check if label.get(2..4) == Some(b"--") => Some(2),
            Hyphens::Allow | Hyphens::Check => None,
        };

        if let Some(index) = hyphen {
            return Some(Err(Error::Hyphen(offset + index)));
        }

        if non_ldh && self.std3_rules == Std3Rules::Apply {
            // The caller only sets `non_ldh` for a label containing a byte other than a letter,
            // digit or hyphen (and never passes non-ASCII bytes or dots), so this always finds
            // one and the fallback is never used
            let index = label
                .iter()
                .position(|&b| !b.is_ascii_alphanumeric() && b != b'-');
            return Some(Err(Error::Std3Rules(offset + index.unwrap_or(0))));
        }

        Some(Ok(()))
    }

    /// Applies the UTS #46 Processing steps to `domain`
    ///
    /// Returns the processed domain name as a sequence of code points, with labels separated
    /// by U+002E FULL STOP.
    ///
    /// The input is first split at U+002E FULL STOP, and each part is mapped and normalized on
    /// its own, before splitting the result into labels at any remaining U+002E FULL STOPs.
    /// This is equivalent to processing the whole domain name at once, because U+002E FULL STOP
    /// is valid, and does not take part in canonical reordering or composition. It allows ASCII
    /// parts to skip the Map and Normalize steps, since these only lowercase ASCII.
    fn process(&self, domain: &str) -> Result<Vec<char>, Error> {
        let mut out = Vec::with_capacity(domain.len());
        let mut bidi = false;
        // The index of the current label in `out`, to locate errors in the input
        let mut labels = 0;
        for (i, part) in domain.split('.').enumerate() {
            if i > 0 {
                out.push('.');
            }

            if part.is_ascii() {
                bidi |= self
                    .convert_label(Label::Ascii(part.as_bytes()), &mut out)
                    .map_err(|error| error.locate(domain, labels))?;
                labels += 1;
                continue;
            }

            let start = out.len();
            let mut normalizer = Normalizer::new(Mode::Map);
            for c in part.chars() {
                normalizer.push(c, &mut out);
            }
            normalizer.finish(&mut out);

            let mapped = &out[start..];
            if mapped.contains(&REPLACEMENT) {
                // Earlier parts cannot contain disallowed code points, so this finds one in `part`
                return Err(Error::DisallowedCodePoint(find_disallowed(domain)));
            }

            let punycode = mapped
                .split(|&c| c == '.')
                .any(|label| label.starts_with(&PUNYCODE_PREFIX));
            if !punycode {
                for label in mapped.split(|&c| c == '.') {
                    bidi |= self
                        .check_label(label)
                        .map_err(|error| error.locate(domain, labels))?;
                    labels += 1;
                }
                continue;
            }

            let mapped = out.split_off(start);
            for (i, label) in mapped.split(|&c| c == '.').enumerate() {
                if i > 0 {
                    out.push('.');
                }

                bidi |= self
                    .convert_label(Label::Mapped(label), &mut out)
                    .map_err(|error| error.locate(domain, labels))?;
                labels += 1;
            }
        }

        if bidi {
            for (i, label) in out.split(|&c| c == '.').enumerate() {
                check_bidi(label).map_err(|error| error.locate_decoded(domain, i))?;
            }
        }

        Ok(out)
    }

    /// Applies the Convert/Validate step to a mapped and normalized label, appending it to `out`
    ///
    /// Returns whether the label makes the domain name a Bidi domain name. The offsets of errors
    /// are relative to the label before Punycode decoding, so errors found after decoding point
    /// at the start of the label.
    fn convert_label(&self, label: Label<'_>, out: &mut Vec<char>) -> Result<bool, Error> {
        let start = out.len();
        let decoded = match label {
            Label::Ascii(label) => match has_punycode_prefix(label) {
                true => {
                    decode_label(&label[PUNYCODE_PREFIX.len()..], out)?;
                    true
                }
                false => {
                    out.extend(label.iter().map(|b| char::from(b.to_ascii_lowercase())));
                    false
                }
            },
            Label::Mapped(label) => match label.strip_prefix(&PUNYCODE_PREFIX) {
                Some(encoded) => {
                    decode_label(encoded, out)?;
                    true
                }
                None => {
                    out.extend_from_slice(label);
                    false
                }
            },
        };

        let result = self.check_label(&out[start..]);
        match decoded {
            // Offsets in the decoded label do not correspond to the input
            true => result.map_err(|error| error.map_offset(|_| 0)),
            false => result,
        }
    }

    /// Checks the UTS #46 Validity Criteria that apply to a single label
    ///
    /// The NFC and code point status criteria are checked by the caller. Returns whether the
    /// label contains a character with Bidi_Class R, AL or AN, which makes the domain name a
    /// Bidi domain name. The offsets of errors are relative to the label.
    fn check_label(&self, label: &[char]) -> Result<bool, Error> {
        let Some(&first) = label.first() else {
            return Ok(false);
        };

        let hyphen = match self.hyphens {
            Hyphens::Allow if label.starts_with(&PUNYCODE_PREFIX) => Some(2),
            Hyphens::Check if first == '-' => Some(0),
            Hyphens::Check if label.last() == Some(&'-') => Some(label.len() - 1),
            Hyphens::Check if label.get(2..4) == Some(&['-', '-']) => Some(2),
            Hyphens::Allow | Hyphens::Check => None,
        };

        if let Some(offset) = hyphen {
            return Err(Error::Hyphen(offset));
        }

        if is_mark(first) {
            return Err(Error::LeadingMark(0));
        }

        let mut rtl = false;
        for (i, &c) in label.iter().enumerate() {
            if c.is_ascii() {
                match self.std3_rules {
                    Std3Rules::Ignore => {}
                    Std3Rules::Apply => {
                        if !matches!(c, 'a'..='z' | '0'..='9' | '-') {
                            return Err(Error::Std3Rules(i));
                        }
                    }
                }
                continue;
            }

            if c == ZERO_WIDTH_NON_JOINER || c == ZERO_WIDTH_JOINER {
                check_joiner(label, i)?;
            }

            rtl |= matches!(
                BidiClass::from(c),
                BidiClass::RightToLeft | BidiClass::ArabicLetter | BidiClass::ArabicNumber
            );
        }

        Ok(rtl)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the STD 3 restrictions on ASCII code points apply (_UseSTD3ASCIIRules_)
///
/// When applied, the only ASCII code points allowed in a label (after mapping) are lowercase
/// letters, digits and U+002D HYPHEN-MINUS.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Std3Rules {
    /// Applies the STD 3 rules (_UseSTD3ASCIIRules=true_)
    Apply,
    /// Allows all ASCII code points (_UseSTD3ASCIIRules=false_)
    Ignore,
}

/// Whether hyphen placement is checked (_CheckHyphens_)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hyphens {
    /// Only rejects labels that start with `xn--` after conversion (_CheckHyphens=false_)
    Allow,
    /// Checks hyphen placement (_CheckHyphens=true_)
    ///
    /// Labels must not start or end with a hyphen, or have hyphens in both the third and fourth
    /// positions.
    Check,
}

/// Whether DNS length limits are verified by _ToASCII_ (_VerifyDnsLength_)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsLength {
    /// Does not verify lengths (_VerifyDnsLength=false_)
    Ignore,
    /// Verifies lengths (_VerifyDnsLength=true_)
    ///
    /// The domain name must be 1 to 253 bytes long, and each label 1 to 63 bytes long, without a
    /// trailing root label.
    Verify,
}

/// An error indicating that a domain name is invalid
///
/// Each variant corresponds to a check in UTS #46 processing, and carries the byte offset in the
/// input at which the first problem was found (also available through [`Error::offset()`]).
/// Where the problem is a specific code point, the offset points at it if its label appears in
/// the input unchanged, apart from ASCII case and _ignored_ code points. If the label was
/// otherwise changed by the Map and Normalize steps, the offset points at the start of the label
/// instead. This also applies to problems found in a label after decoding it from Punycode.
///
/// Variants are listed in the order of the corresponding steps in UTS #46: conversion from
/// Punycode, the validity criteria and the DNS length restrictions of _ToASCII_.
///
/// ```
/// use idn::{Config, Error};
///
/// let config = Config::new();
/// assert_eq!(config.to_ascii("ab\u{200d}c.example"), Err(Error::Joiner(2)));
/// assert_eq!(
///     Config::strict().to_ascii("www.ex_ample.com"),
///     Err(Error::Std3Rules(6))
/// );
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A label is not valid Punycode, or is too long to encode as Punycode
    ///
    /// A label starting with `xn--` must be valid Punycode that decodes to a label in NFC
    /// containing a non-ASCII code point. For invalid Punycode, the offset points at the invalid
    /// code point, at the start of the delta that could not be decoded, or at the first code point
    /// beyond the length limit.
    Punycode(usize),
    /// A label contains a misplaced hyphen
    ///
    /// With [`Hyphens::Check`], a label starts or ends with a hyphen or has hyphens in both the
    /// third and fourth positions. With [`Hyphens::Allow`], a label starts with `xn--` after
    /// Punycode decoding.
    Hyphen(usize),
    /// A label starts with a combining mark
    LeadingMark(usize),
    /// A code point is _disallowed_
    ///
    /// For a Punycode label, this also applies to decoded code points that are not _valid_.
    DisallowedCodePoint(usize),
    /// An ASCII code point is not allowed by [`Std3Rules::Apply`]
    ///
    /// Only letters, digits and U+002D HYPHEN-MINUS are allowed.
    Std3Rules(usize),
    /// A joiner occurs in a context where the CONTEXTJ rules do not allow it
    ///
    /// The joiners are U+200C ZERO WIDTH NON-JOINER and U+200D ZERO WIDTH JOINER, and the
    /// CONTEXTJ rules are specified in RFC 5892, appendix A.
    Joiner(usize),
    /// A label does not satisfy the Bidi rule (RFC 5893, section 2)
    ///
    /// The rule applies to all labels of a domain name that contains right-to-left characters.
    Bidi(usize),
    /// With [`DnsLength::Verify`], the domain name is empty or longer than 253 bytes
    DomainLength(usize),
    /// With [`DnsLength::Verify`], a label is empty or longer than 63 bytes
    LabelLength(usize),
}

impl Error {
    /// The byte offset in the input at which the problem was found
    pub const fn offset(&self) -> usize {
        match *self {
            Self::Punycode(offset)
            | Self::Hyphen(offset)
            | Self::LeadingMark(offset)
            | Self::DisallowedCodePoint(offset)
            | Self::Std3Rules(offset)
            | Self::Joiner(offset)
            | Self::Bidi(offset)
            | Self::DomainLength(offset)
            | Self::LabelLength(offset) => offset,
        }
    }

    /// Converts an offset in label `label` of `domain` into an offset in `domain`
    ///
    /// The offset counts the code points of the processed label before Punycode decoding (see
    /// [`input_offset()`]).
    fn locate(self, domain: &str, label: usize) -> Self {
        self.map_offset(|index| input_offset(domain, label, index, false))
    }

    /// Like [`Error::locate()`], for an offset relative to the label after Punycode decoding
    fn locate_decoded(self, domain: &str, label: usize) -> Self {
        self.map_offset(|index| input_offset(domain, label, index, true))
    }

    fn map_offset(mut self, f: impl FnOnce(usize) -> usize) -> Self {
        let (Self::Punycode(offset)
        | Self::Hyphen(offset)
        | Self::LeadingMark(offset)
        | Self::DisallowedCodePoint(offset)
        | Self::Std3Rules(offset)
        | Self::Joiner(offset)
        | Self::Bidi(offset)
        | Self::DomainLength(offset)
        | Self::LabelLength(offset)) = &mut self;
        *offset = f(*offset);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let problem = match self {
            Self::Punycode(_) => "invalid Punycode label",
            Self::Hyphen(_) => "misplaced hyphen",
            Self::LeadingMark(_) => "label starts with a combining mark",
            Self::DisallowedCodePoint(_) => "disallowed code point",
            Self::Std3Rules(_) => "code point not allowed by STD3 rules",
            Self::Joiner(_) => "joiner not allowed in this context",
            Self::Bidi(_) => "label does not satisfy the Bidi rule",
            Self::DomainLength(_) => "domain name is empty or longer than 253 bytes",
            Self::LabelLength(_) => "label is empty or longer than 63 bytes",
        };

        write!(
            f,
            "invalid domain name at offset {}: {problem}",
            self.offset()
        )
    }
}

impl StdError for Error {}

/// A label to be converted, before the Convert/Validate step
#[derive(Clone, Copy, Debug)]
enum Label<'a> {
    /// An ASCII label from the input, which only needs to be lowercased
    Ascii(&'a [u8]),
    /// A label that has been mapped and normalized
    Mapped(&'a [char]),
}

/// Decodes a Punycode label (without its `xn--` prefix), appending the result to `out`
///
/// This checks the parts of the Convert/Validate step that only apply to Punycode labels: the
/// decoded label must contain a non-ASCII code point, and it must satisfy the NFC and code point
/// status validity criteria. It cannot contain U+002E FULL STOP, because `encoded` is a single
/// label and decoding only inserts non-ASCII code points.
///
/// Basic code points are lowercased, as the Map step would have done before decoding. Since
/// Punycode decoding copies basic code points to the output unchanged, this can happen after
/// decoding.
///
/// The offsets of errors are relative to the label including its prefix.
fn decode_label(encoded: &[impl Copy + Into<u32>], out: &mut Vec<char>) -> Result<(), Error> {
    let start = out.len();
    punycode::decode_into(encoded, out)
        .map_err(|index| Error::Punycode(PUNYCODE_PREFIX.len() + index))?;
    let decoded = &mut out[start..];
    for c in decoded.iter_mut() {
        c.make_ascii_lowercase();
    }

    if decoded.iter().all(char::is_ascii) {
        return Err(Error::Punycode(0));
    }

    if unicode::is_valid_nfc(decoded) {
        return Ok(());
    }

    // A label that only contains valid code points can only fail by not being in NFC
    match decoded.iter().all(|&c| unicode::is_valid_nfc(&[c])) {
        true => Err(Error::Punycode(0)),
        false => Err(Error::DisallowedCodePoint(0)),
    }
}

/// Checks the CONTEXTJ rule for the joiner at `label[index]` (RFC 5892, appendix A.1 and A.2)
fn check_joiner(label: &[char], index: usize) -> Result<(), Error> {
    let (before, after) = (&label[..index], &label[index + 1..]);
    if before.last().is_some_and(|&c| is_virama(c)) {
        return Ok(());
    }

    if label[index] == ZERO_WIDTH_JOINER {
        return Err(Error::Joiner(index));
    }

    match joins(before.iter().rev(), JoiningType::LeftJoining)
        && joins(after.iter(), JoiningType::RightJoining)
    {
        true => Ok(()),
        false => Err(Error::Joiner(index)),
    }
}

/// Whether the first non-transparent code point in `chars` joins towards the joiner
///
/// That is the case if its Joining_Type is `side` or Dual_Joining.
fn joins<'a>(chars: impl Iterator<Item = &'a char>, side: JoiningType) -> bool {
    for &c in chars {
        let joining_type = JoiningType::from(c);
        match joining_type {
            JoiningType::Transparent => continue,
            JoiningType::DualJoining => return true,
            JoiningType::LeftJoining | JoiningType::RightJoining => return joining_type == side,
            JoiningType::JoinCausing | JoiningType::NonJoining => return false,
        }
    }

    false
}

/// Checks the Bidi Rule from RFC 5893, section 2
fn check_bidi(label: &[char]) -> Result<(), Error> {
    let Some(&first) = label.first() else {
        return Ok(());
    };

    let first = BidiClass::from(first);
    let direction = if first == BidiClass::LeftToRight {
        Direction::LeftToRight
    } else if matches!(first, BidiClass::RightToLeft | BidiClass::ArabicLetter) {
        Direction::RightToLeft
    } else {
        return Err(Error::Bidi(0));
    };

    let mut end = label.len();
    while end > 1 && BidiClass::from(label[end - 1]) == BidiClass::NonspacingMark {
        end -= 1;
    }

    let (mut european, mut arabic) = (false, false);
    for (i, &c) in label.iter().enumerate() {
        let class = BidiClass::from(c);
        let allowed = match direction {
            Direction::LeftToRight => matches!(
                class,
                BidiClass::LeftToRight
                    | BidiClass::EuropeanNumber
                    | BidiClass::EuropeanSeparator
                    | BidiClass::CommonSeparator
                    | BidiClass::EuropeanTerminator
                    | BidiClass::OtherNeutral
                    | BidiClass::BoundaryNeutral
                    | BidiClass::NonspacingMark
            ),
            Direction::RightToLeft => matches!(
                class,
                BidiClass::RightToLeft
                    | BidiClass::ArabicLetter
                    | BidiClass::ArabicNumber
                    | BidiClass::EuropeanNumber
                    | BidiClass::EuropeanSeparator
                    | BidiClass::CommonSeparator
                    | BidiClass::EuropeanTerminator
                    | BidiClass::OtherNeutral
                    | BidiClass::BoundaryNeutral
                    | BidiClass::NonspacingMark
            ),
        };

        if !allowed {
            return Err(Error::Bidi(i));
        }

        european |= class == BidiClass::EuropeanNumber;
        arabic |= class == BidiClass::ArabicNumber;
    }

    let last = BidiClass::from(label[end - 1]);
    let valid = match direction {
        Direction::LeftToRight => {
            matches!(last, BidiClass::LeftToRight | BidiClass::EuropeanNumber)
        }
        Direction::RightToLeft => matches!(
            last,
            BidiClass::RightToLeft
                | BidiClass::ArabicLetter
                | BidiClass::EuropeanNumber
                | BidiClass::ArabicNumber
        ),
    };

    if !valid {
        return Err(Error::Bidi(end - 1));
    }

    // Left-to-right labels cannot contain Arabic numbers, so this only applies to right-to-left
    // labels. The error points at the first number of the kind that occurs last.
    if european && arabic {
        let first = |class| label.iter().position(|&c| BidiClass::from(c) == class);
        let offset = first(BidiClass::EuropeanNumber).max(first(BidiClass::ArabicNumber));
        return Err(Error::Bidi(offset.unwrap_or(0)));
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Direction {
    LeftToRight,
    RightToLeft,
}

/// Verifies the DNS length restrictions from UTS #46, section 4.2, step 4
///
/// `ascii` is the ASCII form of `domain`, which is used to locate errors.
fn verify_dns_length(domain: &str, ascii: &str) -> Result<(), Error> {
    // The length of the domain name excludes the root label and its dot, although an empty root
    // label is then rejected as an empty label
    let len = ascii.strip_suffix('.').unwrap_or(ascii).len();
    if len == 0 || len > 253 {
        // The label containing the first byte beyond the limit
        let label = ascii.bytes().take(254).filter(|&b| b == b'.').count();
        return Err(Error::DomainLength(input_offset(domain, label, 0, true)));
    }

    for (i, label) in ascii.split('.').enumerate() {
        if label.is_empty() || label.len() > 63 {
            return Err(Error::LabelLength(input_offset(domain, i, 0, true)));
        }
    }

    Ok(())
}

/// Finds the byte offset in `domain` of code point `index` in label `label` of its processed form
///
/// `index` counts the code points of the label before Punycode decoding, or after it if
/// `decoded`. The label corresponds to the part of `domain` between the code points that the Map
/// step turns into U+002E FULL STOP. If that part does not consist of the label's code points,
/// apart from ASCII case and _ignored_ code points, or if `decoded` and the label is decoded as
/// Punycode, this returns the offset of the part instead.
#[cold]
fn input_offset(domain: &str, label: usize, index: usize, decoded: bool) -> usize {
    let (mut start, mut end) = (0, domain.len());
    let mut labels = 0;
    for (i, c) in domain.char_indices() {
        if unicode::maps_to_full_stop(c) {
            if labels == label {
                end = i;
                break;
            }

            labels += 1;
            start = i + c.len_utf8();
        }
    }

    let part = &domain[start..end];
    // The Map and Normalize steps only lowercase ASCII (see `Config::process()`)
    if part.is_ascii() {
        return match index < part.len() && !(decoded && has_punycode_prefix(part.as_bytes())) {
            true => start + index,
            false => start,
        };
    }

    let mapped = Normalize::new(part.chars(), Mode::Map).collect::<Vec<_>>();
    let mut kept = part
        .char_indices()
        .filter(|&(_, c)| !unicode::is_ignored(c));
    let unchanged = kept
        .clone()
        .map(|(_, c)| c.to_ascii_lowercase())
        .eq(mapped.iter().copied());

    match unchanged && !(decoded && mapped.starts_with(&PUNYCODE_PREFIX)) {
        true => kept.nth(index).map_or(start, |(i, _)| start + i),
        false => start,
    }
}

/// Finds the byte offset of the first disallowed code point in `domain`
#[cold]
fn find_disallowed(domain: &str) -> usize {
    domain
        .char_indices()
        .find(|&(_, c)| unicode::is_disallowed(c))
        .map_or(0, |(i, _)| i)
}

/// Whether `label` starts with `xn--`, ignoring ASCII case
fn has_punycode_prefix(label: &[u8]) -> bool {
    label.len() >= 4 && label[..2].eq_ignore_ascii_case(b"xn") && &label[2..4] == b"--"
}

/// The version of Unicode (major, minor, update) that the data in this crate corresponds to
pub const UNICODE_VERSION: (u8, u8, u8) = tables::UNICODE_VERSION;

const PUNYCODE_PREFIX: [char; 4] = ['x', 'n', '-', '-'];
const ZERO_WIDTH_NON_JOINER: char = '\u{200c}';
const ZERO_WIDTH_JOINER: char = '\u{200d}';
