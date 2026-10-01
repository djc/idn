//! Punycode encoding and decoding as specified in [RFC 3492]
//!
//! Both directions take quadratic time in the worst case. To bound the cost of processing
//! untrusted input, they reject inputs longer than 2000 code points, well above the 63-byte DNS
//! label limit. Decoding never produces more code points than its input contains, so anything
//! that can be decoded can also be encoded again.
//!
//! [RFC 3492]: https://www.rfc-editor.org/rfc/rfc3492

use alloc::string::String;
use alloc::vec::Vec;

/// Decodes `input`, appending the decoded code points to `out`
///
/// This is the procedure from RFC 3492, section 6.2. On error, returns the index in `input` of
/// the invalid code point, of the start of the delta that could not be decoded, or of the first
/// code point beyond the length limit. The contents of `out` beyond its original length are then
/// unspecified.
pub(crate) fn decode_into(
    input: &[impl Copy + Into<u32>],
    out: &mut Vec<char>,
) -> Result<(), usize> {
    if input.len() > MAX_INPUT {
        return Err(MAX_INPUT);
    }

    let (basic, extended) = match input.iter().rposition(|&c| c.into() == DELIMITER) {
        Some(delimiter) if delimiter > 0 => (&input[..delimiter], &input[delimiter + 1..]),
        _ => (&input[..0], input),
    };

    let start = out.len();
    for (index, &c) in basic.iter().enumerate() {
        let Some(c) = char::from_u32(c.into()).filter(char::is_ascii) else {
            return Err(index);
        };
        out.push(c);
    }

    // `len` is bounded by `MAX_INPUT`, so it cannot overflow.
    let mut len = basic.len() as u32;
    let mut n = INITIAL_N;
    let mut i = 0u32;
    let mut bias = INITIAL_BIAS;
    let mut extended = extended.iter();
    while !extended.as_slice().is_empty() {
        let delta = input.len() - extended.as_slice().len();
        let old_i = i;
        let mut w = 1u32;
        let mut k = BASE;
        loop {
            let Some(&c) = extended.next() else {
                return Err(delta);
            };
            let digit = match c.into() {
                c @ 0x41..=0x5a => c - 0x41,
                c @ 0x61..=0x7a => c - 0x61,
                c @ 0x30..=0x39 => c - 0x30 + 26,
                _ => return Err(input.len() - extended.as_slice().len() - 1),
            };

            i = digit
                .checked_mul(w)
                .and_then(|product| i.checked_add(product))
                .ok_or(delta)?;
            let t = threshold(k, bias);
            if digit < t {
                break;
            }

            w = w.checked_mul(BASE - t).ok_or(delta)?;
            k += BASE;
        }

        len += 1;
        bias = adapt(i - old_i, len, old_i == 0);
        n = n.checked_add(i / len).ok_or(delta)?;
        i %= len;
        let c = char::from_u32(n).ok_or(delta)?;
        out.insert(start + i as usize, c);
        i += 1;
    }

    Ok(())
}

/// Encodes `input` as Punycode, appending the result to `out`
///
/// This is the procedure from RFC 3492, section 6.3. On error, returns the index in `input` of
/// the first code point beyond the length limit.
pub(crate) fn encode_into(input: &[char], out: &mut String) -> Result<(), usize> {
    if input.len() > MAX_INPUT {
        return Err(MAX_INPUT);
    }

    let mut basic = 0;
    for &c in input {
        if c.is_ascii() {
            out.push(c);
            basic += 1;
        }
    }

    if basic > 0 {
        out.push('-');
    }

    // `len` is bounded by `MAX_INPUT`, so neither it nor `delta` can overflow
    const { assert!(MAX_ENCODE_DELTA <= u32::MAX as u64) };
    let len = input.len() as u32;
    let mut handled = basic;
    let mut n = INITIAL_N;
    let mut delta = 0u32;
    let mut bias = INITIAL_BIAS;
    while handled < len {
        // Since not all code points have been handled, one of them is at least `n`
        let mut m = u32::MAX;
        for &c in input {
            let c = u32::from(c);
            if c >= n && c < m {
                m = c;
            }
        }

        delta += (m - n) * (handled + 1);
        n = m;

        for &c in input {
            let c = u32::from(c);
            if c < n {
                delta += 1;
            }

            if c != n {
                continue;
            }

            let mut q = delta;
            let mut k = BASE;
            loop {
                let t = threshold(k, bias);
                if q < t {
                    break;
                }

                out.push(digit(t + (q - t) % (BASE - t)));
                q = (q - t) / (BASE - t);
                k += BASE;
            }

            out.push(digit(q));
            bias = adapt(delta, handled + 1, handled == basic);
            delta = 0;
            handled += 1;
        }

        delta += 1;
        n += 1;
    }

    Ok(())
}

/// The bias adaptation function from RFC 3492, section 6.1
fn adapt(delta: u32, num_points: u32, first_time: bool) -> u32 {
    let mut delta = match first_time {
        true => delta / DAMP,
        false => delta / 2,
    };

    delta += delta / num_points;
    let mut k = 0;
    while delta > ((BASE - T_MIN) * T_MAX) / 2 {
        delta /= BASE - T_MIN;
        k += BASE;
    }

    k + ((BASE - T_MIN + 1) * delta) / (delta + SKEW)
}

/// The threshold `t` for position `k`, clamped to `T_MIN..=T_MAX`
fn threshold(k: u32, bias: u32) -> u32 {
    k.saturating_sub(bias).clamp(T_MIN, T_MAX)
}

/// The basic code point representing `digit`
///
/// Callers guarantee `digit < BASE`: either `digit < t <= T_MAX`, or `digit` is
/// `t + (q - t) % (BASE - t)`, which is less than `t + BASE - t`.
fn digit(digit: u32) -> char {
    char::from(DIGITS[digit as usize])
}

const DIGITS: &[u8; BASE as usize] = b"abcdefghijklmnopqrstuvwxyz0123456789";

const BASE: u32 = 36;
const T_MIN: u32 = 1;
const T_MAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 0x80;
const DELIMITER: u32 = 0x2d;

const MAX_INPUT: usize = 2000;

/// An upper bound for `delta` while encoding (RFC 3492, section 6.4)
///
/// Before `delta` is reset, it is increased by at most `(0x10_ffff - INITIAL_N) * (len + 1)` for
/// the step to the next code point, and by at most `len` in each of two passes over the input.
const MAX_ENCODE_DELTA: u64 =
    (0x10_ffff - INITIAL_N as u64) * (MAX_INPUT as u64 + 1) + 2 * MAX_INPUT as u64;

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn rfc3492_samples() {
        for (unicode, punycode) in SAMPLES {
            assert!(
                encode(unicode).unwrap().eq_ignore_ascii_case(punycode),
                "{unicode}"
            );
            assert_eq!(decode(punycode).unwrap(), *unicode, "{punycode}");
        }
    }

    /// Errors point at an invalid code point, or at the start of the delta that failed
    #[test]
    fn invalid() {
        for (input, index) in [
            ("-", 0),
            ("-a", 0),
            ("99999999999", 0),
            ("\u{e9}-a", 0),
            ("zzzzzzzzzzzzzzzzzzzzzzz", 22),
            ("!", 0),
            ("a-b!", 3),
            ("a-b", 2),
            ("ls8h!", 4),
            ("ls8h99999999999", 4),
        ] {
            assert_eq!(decode(input), Err(index), "{input}");
        }
    }

    #[test]
    fn length_limits() {
        assert!(encode(&"\u{e9}".repeat(MAX_INPUT)).is_ok());
        assert_eq!(encode(&"\u{e9}".repeat(MAX_INPUT + 1)), Err(MAX_INPUT));
        assert_eq!(decode(&"a".repeat(MAX_INPUT + 1)), Err(MAX_INPUT));
    }

    #[test]
    fn round_trip_boundaries() {
        for input in [
            "\u{80}",
            "\u{10ffff}",
            "a\u{10ffff}b\u{80}",
            "\u{d7ff}\u{e000}",
        ] {
            let encoded = encode(input).unwrap();
            assert_eq!(decode(&encoded).unwrap(), input.to_string());
        }
    }

    /// Sample strings from RFC 3492, section 7.1
    const SAMPLES: &[(&str, &str)] = &[
        (
            "\u{644}\u{64a}\u{647}\u{645}\u{627}\u{628}\u{62a}\u{643}\u{644}\u{645}\u{648}\u{634}\u{639}\u{631}\u{628}\u{64a}\u{61f}",
            "egbpdaj6bu4bxfgehfvwxn",
        ),
        (
            "\u{4ed6}\u{4eec}\u{4e3a}\u{4ec0}\u{4e48}\u{4e0d}\u{8bf4}\u{4e2d}\u{6587}",
            "ihqwcrb4cv8a8dqg056pqjye",
        ),
        (
            "\u{4ed6}\u{5011}\u{7232}\u{4ec0}\u{9ebd}\u{4e0d}\u{8aaa}\u{4e2d}\u{6587}",
            "ihqwctvzc91f659drss3x8bo0yb",
        ),
        (
            "Pro\u{10d}prost\u{11b}nemluv\u{ed}\u{10d}esky",
            "Proprostnemluvesky-uyb24dma41a",
        ),
        (
            "\u{5dc}\u{5de}\u{5d4}\u{5d4}\u{5dd}\u{5e4}\u{5e9}\u{5d5}\u{5d8}\u{5dc}\u{5d0}\u{5de}\u{5d3}\u{5d1}\u{5e8}\u{5d9}\u{5dd}\u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}",
            "4dbcagdahymbxekheh6e0a7fei0b",
        ),
        (
            "\u{92f}\u{939}\u{932}\u{94b}\u{917}\u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}\u{915}\u{94d}\u{92f}\u{94b}\u{902}\u{928}\u{939}\u{940}\u{902}\u{92c}\u{94b}\u{932}\u{938}\u{915}\u{924}\u{947}\u{939}\u{948}\u{902}",
            "i1baa7eci9glrd9b2ae1bj0hfcgg6iyaf8o0a1dig0cd",
        ),
        (
            "\u{306a}\u{305c}\u{307f}\u{3093}\u{306a}\u{65e5}\u{672c}\u{8a9e}\u{3092}\u{8a71}\u{3057}\u{3066}\u{304f}\u{308c}\u{306a}\u{3044}\u{306e}\u{304b}",
            "n8jok5ay5dzabd5bym9f0cm5685rrjetr6pdxa",
        ),
        (
            "\u{c138}\u{acc4}\u{c758}\u{baa8}\u{b4e0}\u{c0ac}\u{b78c}\u{b4e4}\u{c774}\u{d55c}\u{ad6d}\u{c5b4}\u{b97c}\u{c774}\u{d574}\u{d55c}\u{b2e4}\u{ba74}\u{c5bc}\u{b9c8}\u{b098}\u{c88b}\u{c744}\u{ae4c}",
            "989aomsvi5e83db1d2a355cv1e0vak1dwrv93d5xbh15a0dt30a5jpsd879ccm6fea98c",
        ),
        (
            "\u{43f}\u{43e}\u{447}\u{435}\u{43c}\u{443}\u{436}\u{435}\u{43e}\u{43d}\u{438}\u{43d}\u{435}\u{433}\u{43e}\u{432}\u{43e}\u{440}\u{44f}\u{442}\u{43f}\u{43e}\u{440}\u{443}\u{441}\u{441}\u{43a}\u{438}",
            "b1abfaaepdrnnbgefbaDotcwatmq2g4l",
        ),
        (
            "Porqu\u{e9}nopuedensimplementehablarenEspa\u{f1}ol",
            "PorqunopuedensimplementehablarenEspaol-fmd56a",
        ),
        (
            "3\u{5e74}B\u{7d44}\u{91d1}\u{516b}\u{5148}\u{751f}",
            "3B-ww4c5e180e575a65lsy2b",
        ),
        (
            "\u{5b89}\u{5ba4}\u{5948}\u{7f8e}\u{6075}-with-SUPER-MONKEYS",
            "-with-SUPER-MONKEYS-pc58ag80a8qai00g7n9n",
        ),
        (
            "Hello-Another-Way-\u{305d}\u{308c}\u{305e}\u{308c}\u{306e}\u{5834}\u{6240}",
            "Hello-Another-Way--fc4qua05auwb3674vfr0b",
        ),
        (
            "\u{3072}\u{3068}\u{3064}\u{5c4b}\u{6839}\u{306e}\u{4e0b}2",
            "2-u9tlzr9756bt3uc0v",
        ),
        (
            "Maji\u{3067}Koi\u{3059}\u{308b}5\u{79d2}\u{524d}",
            "MajiKoi5-783gue6qz075azm5e",
        ),
        (
            "\u{30d1}\u{30d5}\u{30a3}\u{30fc}de\u{30eb}\u{30f3}\u{30d0}",
            "de-jg4avhby1noc0d",
        ),
        (
            "\u{305d}\u{306e}\u{30b9}\u{30d4}\u{30fc}\u{30c9}\u{3067}",
            "d9juau41awczczp",
        ),
        ("-> $1.00 <-", "-> $1.00 <--"),
    ];

    fn decode(input: &str) -> Result<String, usize> {
        let mut out = Vec::new();
        decode_into(input.as_bytes(), &mut out)?;
        Ok(out.into_iter().collect())
    }

    fn encode(input: &str) -> Result<String, usize> {
        let input = input.chars().collect::<Vec<_>>();
        let mut out = String::new();
        encode_into(&input, &mut out)?;
        Ok(out)
    }
}
