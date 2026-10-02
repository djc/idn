//! Exhaustive tests of the Unicode data against an independent reference implementation
//!
//! The reference is a direct transcription of UTS #46 (Map), UAX #15 (NFD, Canonical Ordering,
//! Canonical Composition) and the property definitions, built from the raw UCD files in `data/`.
//! Where possible, it uses different source files than the table generator.

use core::ops::RangeInclusive;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::OnceLock;

use idn::{BidiClass, JoiningType, Mode, Normalize, UNICODE_VERSION};

#[test]
fn unicode_version() {
    let data = read("IdnaMappingTable.txt");
    let version = data
        .lines()
        .find_map(|line| line.strip_prefix("# Version: "))
        .unwrap();
    let (major, minor, update) = UNICODE_VERSION;
    assert_eq!(version.trim(), format!("{major}.{minor}.{update}"));
}

#[test]
fn map_normalize_single_code_points() {
    let reference = Reference::get();
    for c in all_chars() {
        let expected = reference.nfc(&reference.map(&[c], Ignored::Remove));
        let actual = Normalize::new([c], Mode::Map).collect::<Vec<_>>();
        assert_eq!(actual, expected, "U+{:04X}", u32::from(c));

        let expected = reference.nfc(&reference.map(&[c], Ignored::Disallow));
        let actual = Normalize::new([c], Mode::Validate).collect::<Vec<_>>();
        assert_eq!(actual, expected, "U+{:04X}", u32::from(c));
    }
}

#[test]
fn map_normalize_followed_by_combining_marks() {
    let reference = Reference::get();
    let marks = [
        '\u{300}', '\u{323}', '\u{345}', '\u{94d}', '\u{3099}', '\u{1161}', '\u{11a8}',
    ];
    for c in all_chars() {
        for &mark in &marks {
            let input = [c, mark];
            let expected = reference.nfc(&reference.map(&input, Ignored::Remove));
            let actual = Normalize::new(input, Mode::Map).collect::<Vec<_>>();
            assert_eq!(actual, expected, "{input:?}");
        }
    }
}

#[test]
fn map_normalize_random_strings() {
    let reference = Reference::get();
    let alphabet = reference.interesting();
    let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
    for _ in 0..200_000 {
        let len = 1 + rng.next() as usize % 8;
        let mut input = Vec::with_capacity(len);
        for _ in 0..len {
            input.push(alphabet[rng.next() as usize % alphabet.len()]);
        }

        let expected = reference.nfc(&reference.map(&input, Ignored::Remove));
        let actual = Normalize::new(input.iter().copied(), Mode::Map).collect::<Vec<_>>();
        assert_eq!(actual, expected, "{input:?}");

        let expected = reference.nfc(&reference.map(&input, Ignored::Disallow));
        let actual = Normalize::new(input.iter().copied(), Mode::Validate).collect::<Vec<_>>();
        assert_eq!(actual, expected, "{input:?}");
    }
}

/// Checks both the reference and the implementation against `NormalizationTest.txt`
#[test]
fn normalization_test() {
    let reference = Reference::get();
    let data = read("NormalizationTest.txt");
    let mut checked = 0;
    for line in data.lines() {
        let line = line.split('#').next().unwrap();
        if line.starts_with('@') || line.trim().is_empty() {
            continue;
        }

        let mut columns = Vec::new();
        for column in line.split(';').take(5) {
            columns.push(
                code_points(column)
                    .into_iter()
                    .map(|cp| char::from_u32(cp).unwrap())
                    .collect::<Vec<_>>(),
            );
        }
        for (i, column) in columns.iter().enumerate() {
            let expected = match i {
                0..3 => &columns[1],
                _ => &columns[3],
            };
            assert_eq!(&reference.nfc(column), expected, "reference: {line}");
        }

        for column in &columns[..3] {
            let expected = reference.nfc(&reference.map(column, Ignored::Remove));
            let actual = Normalize::new(column.iter().copied(), Mode::Map).collect::<Vec<_>>();
            assert_eq!(actual, expected, "{line}");

            if reference.all_valid(column) {
                assert_eq!(actual, columns[1], "{line}");
                checked += 1;
            }
        }
    }

    assert!(checked > 10_000, "only {checked} NFC checks performed");
}

/// Examples with known results, from the tests of the `idna_adapter` back ends
///
/// These come from `icu_normalizer` (UTS #46 and NFD/NFKD tests) and `unicode-normalization`
/// (NFC tests), with the expected results adjusted for the Map step where it applies. Unlike
/// the tests above, they do not depend on the reference implementation.
#[test]
fn map_normalize_examples() {
    let examples = [
        ("a\u{308}", "\u{e4}"),
        ("A\u{308}", "\u{e4}"),
        ("a\u{301}", "\u{e1}"),
        ("\u{301}a", "\u{301}a"),
        ("e\u{323}\u{302}", "\u{1ec7}"),
        ("E\u{323}\u{302}", "\u{1ec7}"),
        // Recomposition after reordering into a precomposed character
        ("\u{1e0b}\u{323}", "\u{1e0d}\u{307}"),
        ("\u{1e0d}\u{307}", "\u{1e0d}\u{307}"),
        // Stable reordering of the marks that follow a composed one
        (
            "a\u{300}\u{305}\u{315}\u{5ae}b",
            "\u{e0}\u{5ae}\u{305}\u{315}b",
        ),
        // Mapping whose decomposition ends in a mark that reorders with a following one
        ("\u{1c4}\u{323}", "d\u{1e93}\u{30c}"),
        ("DZ\u{30c}\u{323}", "d\u{1e93}\u{30c}"),
        ("\u{1e0b}\u{1c4}", "\u{1e0b}d\u{17e}"),
        // Two compositions, the second one across a mark with a lower combining class
        ("\u{ddd}\u{334}", "\u{ddd}\u{334}"),
        ("\u{dd9}\u{dcf}\u{334}\u{dca}", "\u{ddd}\u{334}"),
        // Hangul
        ("\u{d4db}", "\u{d4db}"),
        ("\u{ac1c}", "\u{ac1c}"),
        ("\u{1111}\u{1171}\u{11b6}", "\u{d4db}"),
        // Composition exclusion
        ("\u{1d15e}", "\u{1d157}\u{1d165}"),
        // Singleton decomposition (OHM SIGN)
        ("\u{2126}", "\u{3c9}"),
        // Halfwidth forms, mapped to fullwidth and then composed
        ("\u{ff8d}\u{ff9e}", "\u{30d9}"),
        ("\u{ff8d}\u{ff9f}", "\u{30da}"),
        // Expanded ligatures
        ("\u{fb01}", "fi"),
        (
            "\u{fdfa}",
            "\u{635}\u{644}\u{649} \u{627}\u{644}\u{644}\u{647} \u{639}\u{644}\u{64a}\u{647} \u{648}\u{633}\u{644}\u{645}",
        ),
        // Parenthesized Hangul, expanded to conjoining jamo and partially recomposed
        ("\u{320e}", "(\u{ac00})"),
        // Deviations
        ("\u{200c}", "\u{200c}"),
        ("\u{200d}", "\u{200d}"),
        ("\u{df}", "\u{df}"),
        ("\u{3c2}", "\u{3c2}"),
        // Iota subscript
        ("\u{345}", "\u{3b9}"),
        // Disallowed
        ("\u{61c}", "\u{fffd}"),
    ];

    for (input, expected) in examples {
        let actual = Normalize::new(input.chars(), Mode::Map).collect::<String>();
        assert_eq!(actual, expected, "{input:?}");
        let actual = Normalize::new(input.chars(), Mode::Validate).collect::<String>();
        assert_eq!(actual, expected, "{input:?}");
    }

    // Ignored
    let input = "a\u{180b}b";
    let actual = Normalize::new(input.chars(), Mode::Map).collect::<String>();
    assert_eq!(actual, "ab");
    let actual = Normalize::new(input.chars(), Mode::Validate).collect::<String>();
    assert_eq!(actual, "a\u{fffd}b");
}

/// The processing code relies on these to skip mapping for ASCII and to split at U+002E early
#[test]
fn ascii_and_full_stop() {
    let reference = Reference::get();
    for b in 0..0x80u8 {
        let c = char::from(b);
        assert_eq!(
            Normalize::new([c], Mode::Map).collect::<Vec<_>>(),
            [c.to_ascii_lowercase()]
        );
        assert_eq!(reference.ccc(u32::from(b)), 0);
    }

    // ASCII code points are composition boundaries, and U+002E FULL STOP does not compose at all
    for &(first, second) in reference.compositions.keys() {
        assert_ne!(first, 0x2e);
        assert!(
            second >= 0x80,
            "ASCII second element {second:04X} after {first:04X}"
        );
    }

    // Locating errors in the input relies on labels being separated by single code points
    for c in (0x80..=0x10_ffff).filter_map(char::from_u32) {
        let mapped = Normalize::new([c], Mode::Map).collect::<Vec<_>>();
        assert!(
            !mapped.contains(&'.') || mapped == ['.'],
            "U+{:04X}",
            u32::from(c)
        );
    }
}

#[test]
fn properties() {
    let reference = Reference::get();
    for c in all_chars() {
        let cp = u32::from(c);
        assert_eq!(
            format!("{:?}", BidiClass::from(c)),
            reference.bidi_class[&cp],
            "U+{cp:04X}"
        );
        assert_eq!(
            format!("{:?}", JoiningType::from(c)),
            reference.joining_type[&cp],
            "U+{cp:04X}"
        );
        assert_eq!(idn::is_mark(c), reference.marks.contains(&cp), "U+{cp:04X}");
        assert_eq!(idn::is_virama(c), reference.ccc(cp) == 9, "U+{cp:04X}");
    }
}

/// Bidi_Class values around the default ranges, from the `unicode-bidi` tests
///
/// Most unassigned code points get their value from the `@missing` lines in
/// `DerivedBidiClass.txt`, which both the generator and the reference implementation parse, so
/// they are checked against known values here. Unlike in the original test, noncharacters are
/// Boundary_Neutral, as listed in `DerivedBidiClass.txt`.
#[test]
fn bidi_class_defaults() {
    use BidiClass::*;

    let examples = [
        // ASCII
        ('\u{0}', BoundaryNeutral),
        ('\u{40}', OtherNeutral),
        ('\u{41}', LeftToRight),
        ('\u{62}', LeftToRight),
        ('\u{7f}', BoundaryNeutral),
        // Hebrew
        ('\u{590}', RightToLeft),
        ('\u{5d0}', RightToLeft),
        ('\u{5d1}', RightToLeft),
        ('\u{5ff}', RightToLeft),
        // Arabic
        ('\u{600}', ArabicNumber),
        ('\u{627}', ArabicLetter),
        ('\u{7bf}', ArabicLetter),
        // Default R, and Arabic extensions
        ('\u{7c0}', RightToLeft),
        ('\u{85f}', RightToLeft),
        ('\u{860}', ArabicLetter),
        ('\u{870}', ArabicLetter),
        ('\u{89f}', NonspacingMark),
        ('\u{8a0}', ArabicLetter),
        ('\u{8ff}', NonspacingMark),
        // Default ET
        ('\u{20a0}', EuropeanTerminator),
        ('\u{20cf}', EuropeanTerminator),
        // Arabic Presentation Forms
        ('\u{fb1d}', RightToLeft),
        ('\u{fb4f}', RightToLeft),
        ('\u{fb50}', ArabicLetter),
        ('\u{fdcf}', OtherNeutral),
        ('\u{fdf0}', ArabicLetter),
        ('\u{fdff}', OtherNeutral),
        ('\u{fe70}', ArabicLetter),
        ('\u{fefe}', ArabicLetter),
        ('\u{feff}', BoundaryNeutral),
        // Noncharacters
        ('\u{fdd0}', BoundaryNeutral),
        ('\u{fdd1}', BoundaryNeutral),
        ('\u{fdee}', BoundaryNeutral),
        ('\u{fdef}', BoundaryNeutral),
        ('\u{fffe}', BoundaryNeutral),
        ('\u{ffff}', BoundaryNeutral),
        // Default R and AL outside the BMP
        ('\u{10800}', RightToLeft),
        ('\u{10fff}', RightToLeft),
        ('\u{1e800}', RightToLeft),
        ('\u{1edff}', RightToLeft),
        ('\u{1ee00}', ArabicLetter),
        ('\u{1eeff}', ArabicLetter),
        ('\u{1ef00}', RightToLeft),
        ('\u{1efff}', RightToLeft),
        // Unassigned planes
        ('\u{30000}', LeftToRight),
        ('\u{40000}', LeftToRight),
        ('\u{50000}', LeftToRight),
        ('\u{60000}', LeftToRight),
        ('\u{70000}', LeftToRight),
        ('\u{80000}', LeftToRight),
        ('\u{90000}', LeftToRight),
        ('\u{a0000}', LeftToRight),
    ];

    for (c, expected) in examples {
        assert_eq!(BidiClass::from(c), expected, "U+{:04X}", u32::from(c));
    }
}

#[test]
fn property_value_names() {
    assert_eq!(format!("{:?}", BidiClass::ArabicLetter), "ArabicLetter");
    assert_eq!(format!("{:?}", JoiningType::DualJoining), "DualJoining");
}

/// Mapped output only contains valid (or disallowed) code points, as UTS #46 promises
#[test]
fn mapping_output_is_valid() {
    let reference = Reference::get();
    for c in all_chars() {
        for out in Normalize::new([c], Mode::Map) {
            let status = reference.status(u32::from(out));
            assert!(
                matches!(
                    status,
                    Status::Valid | Status::Deviation | Status::Disallowed
                ),
                "U+{:04X} maps to U+{:04X} with status {status:?}",
                u32::from(c),
                u32::from(out)
            );
        }
    }
}

struct Reference {
    status: HashMap<u32, Status>,
    ccc: HashMap<u32, u8>,
    decompositions: HashMap<u32, Vec<u32>>,
    compositions: HashMap<(u32, u32), u32>,
    marks: HashSet<u32>,
    bidi_class: HashMap<u32, String>,
    joining_type: HashMap<u32, String>,
}

impl Reference {
    fn get() -> &'static Self {
        static REFERENCE: OnceLock<Reference> = OnceLock::new();
        REFERENCE.get_or_init(Self::load)
    }

    fn load() -> Self {
        let mut status = HashMap::new();
        for (range, fields) in records(&read("IdnaMappingTable.txt")) {
            let value = match fields[0] {
                "valid" => Status::Valid,
                "deviation" => Status::Deviation,
                "ignored" => Status::Ignored,
                "disallowed" => Status::Disallowed,
                "mapped" => Status::Mapped(code_points(fields[1])),
                other => panic!("unknown status {other}"),
            };
            for cp in range {
                status.insert(cp, value.clone());
            }
        }
        assert_eq!(
            status.len(),
            0x11_0000,
            "IDNA mapping table covers all code points"
        );

        let mut ccc = HashMap::new();
        let mut decompositions = HashMap::new();
        let mut marks = HashSet::new();
        let mut range_start = None;
        for line in read("UnicodeData.txt").lines() {
            let fields = line.split(';').collect::<Vec<_>>();
            let cp = u32::from_str_radix(fields[0], 16).unwrap();
            let range = match (
                fields[1].ends_with(", First>"),
                fields[1].ends_with(", Last>"),
            ) {
                (true, _) => {
                    range_start = Some(cp);
                    continue;
                }
                (false, true) => range_start.take().unwrap()..=cp,
                (false, false) => cp..=cp,
            };

            for cp in range {
                ccc.insert(cp, fields[3].parse::<u8>().unwrap());
                if matches!(fields[2], "Mn" | "Mc" | "Me") {
                    marks.insert(cp);
                }
            }

            if !fields[5].is_empty() && !fields[5].starts_with('<') {
                decompositions.insert(cp, code_points(fields[5]));
            }
        }

        let mut exclusions = HashSet::new();
        for (range, _) in records(&read("CompositionExclusions.txt")) {
            exclusions.extend(range);
        }

        let mut compositions = HashMap::new();
        let mut full_exclusions = HashSet::new();
        for (&cp, decomposed) in &decompositions {
            let singleton = decomposed.len() == 1;
            let non_starter = ccc.get(&cp).copied().unwrap_or(0) != 0
                || ccc.get(&decomposed[0]).copied().unwrap_or(0) != 0;
            if exclusions.contains(&cp) || singleton || non_starter {
                full_exclusions.insert(cp);
                continue;
            }

            compositions.insert((decomposed[0], decomposed[1]), cp);
        }

        let mut derived = HashSet::new();
        for (range, fields) in records(&read("DerivedNormalizationProps.txt")) {
            if fields[0] == "Full_Composition_Exclusion" {
                derived.extend(range);
            }
        }
        assert_eq!(
            full_exclusions, derived,
            "Full_Composition_Exclusion derivation"
        );

        Self {
            status,
            ccc,
            decompositions,
            compositions,
            marks,
            bidi_class: property(&read("DerivedBidiClass.txt"), BIDI_CLASSES),
            joining_type: property(&read("DerivedJoiningType.txt"), JOINING_TYPES),
        }
    }

    /// The Map step of UTS #46, section 4, with disallowed code points replaced by U+FFFD
    fn map(&self, input: &[char], ignored: Ignored) -> Vec<char> {
        let mut out = Vec::new();
        for &c in input {
            match (self.status(u32::from(c)), ignored) {
                (Status::Valid | Status::Deviation, _) => out.push(c),
                (Status::Ignored, Ignored::Remove) => {}
                (Status::Ignored, Ignored::Disallow) | (Status::Disallowed, _) => {
                    out.push('\u{fffd}')
                }
                (Status::Mapped(mapping), _) => {
                    for &cp in mapping {
                        out.push(char::from_u32(cp).unwrap());
                    }
                }
            }
        }

        out
    }

    fn all_valid(&self, input: &[char]) -> bool {
        for &c in input {
            if !matches!(self.status(u32::from(c)), Status::Valid | Status::Deviation) {
                return false;
            }
        }

        true
    }

    /// Normalization Form C, per UAX #15 sections 3.11 and 3.10, implemented literally
    fn nfc(&self, input: &[char]) -> Vec<char> {
        let mut decomposed = Vec::new();
        for &c in input {
            self.decompose(u32::from(c), &mut decomposed);
        }

        let mut swapped = true;
        while swapped {
            swapped = false;
            for i in 1..decomposed.len() {
                let (a, b) = (self.ccc(decomposed[i - 1]), self.ccc(decomposed[i]));
                if a > b && b > 0 {
                    decomposed.swap(i - 1, i);
                    swapped = true;
                }
            }
        }

        let mut i = 1;
        'outer: while i < decomposed.len() {
            let c = decomposed[i];
            let mut j = i;
            while j > 0 {
                j -= 1;
                let b = decomposed[j];
                if self.ccc(b) == 0 {
                    let blocked = decomposed[j + 1..i]
                        .iter()
                        .any(|&between| self.ccc(between) == 0 || self.ccc(between) >= self.ccc(c));
                    if !blocked {
                        if let Some(composite) = self.primary_composite(b, c) {
                            decomposed[j] = composite;
                            decomposed.remove(i);
                            continue 'outer;
                        }
                    }
                    break;
                }
            }
            i += 1;
        }

        decomposed
            .into_iter()
            .map(|cp| char::from_u32(cp).unwrap())
            .collect()
    }

    fn decompose(&self, cp: u32, out: &mut Vec<u32>) {
        if (0xac00..0xac00 + 11_172).contains(&cp) {
            let index = cp - 0xac00;
            out.push(0x1100 + index / 588);
            out.push(0x1161 + (index % 588) / 28);
            if index % 28 != 0 {
                out.push(0x11a7 + index % 28);
            }
            return;
        }

        match self.decompositions.get(&cp) {
            Some(decomposed) => {
                for &cp in decomposed {
                    self.decompose(cp, out);
                }
            }
            None => out.push(cp),
        }
    }

    fn primary_composite(&self, first: u32, second: u32) -> Option<u32> {
        if (0x1100..0x1100 + 19).contains(&first) && (0x1161..0x1161 + 21).contains(&second) {
            return Some(0xac00 + ((first - 0x1100) * 21 + (second - 0x1161)) * 28);
        }

        if (0xac00..0xac00 + 11_172).contains(&first)
            && (first - 0xac00) % 28 == 0
            && (0x11a8..0x11a7 + 28).contains(&second)
        {
            return Some(first + (second - 0x11a7));
        }

        self.compositions.get(&(first, second)).copied()
    }

    /// Code points that exercise mapping and normalization, for building random strings
    fn interesting(&self) -> Vec<char> {
        let mut set = HashSet::new();
        for (&(first, second), &composite) in &self.compositions {
            set.extend([first, second, composite]);
        }
        for (&cp, &ccc) in &self.ccc {
            if ccc != 0 {
                set.insert(cp);
            }
        }
        for (&cp, status) in &self.status {
            if matches!(
                status,
                Status::Ignored | Status::Deviation | Status::Mapped(_)
            ) {
                set.insert(cp);
            }
        }
        set.extend([
            0x1100, 0x1161, 0x11a8, 0xac00, 0xac01, 0xd7a3, 0xfffd, 0x10_ffff,
        ]);
        set.extend(0x20..0x80);

        let mut chars = set
            .into_iter()
            .filter_map(char::from_u32)
            .collect::<Vec<_>>();
        chars.sort();
        chars
    }

    fn status(&self, cp: u32) -> &Status {
        &self.status[&cp]
    }

    fn ccc(&self, cp: u32) -> u8 {
        self.ccc.get(&cp).copied().unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
enum Status {
    Valid,
    Deviation,
    Ignored,
    Mapped(Vec<u32>),
    Disallowed,
}

#[derive(Clone, Copy, Debug)]
enum Ignored {
    Remove,
    Disallow,
}

/// Parses a property file with `@missing` defaults into Rust-style value names
fn property(data: &str, names: &[(&str, &str)]) -> HashMap<u32, String> {
    let name = |value: &str| {
        for &(short, long) in names {
            if value == short || value == long {
                return long.replace('_', "");
            }
        }
        panic!("unknown property value {value}");
    };

    let mut values = HashMap::new();
    for line in data.lines() {
        let Some(missing) = line.strip_prefix("# @missing: ") else {
            continue;
        };
        let (range, value) = missing.split_once(';').unwrap();
        let value = name(value.trim());
        for cp in parse_range(range.trim()) {
            values.insert(cp, value.clone());
        }
    }

    for (range, fields) in records(data) {
        let value = name(fields[0]);
        for cp in range {
            values.insert(cp, value.clone());
        }
    }

    values
}

fn records(data: &str) -> Vec<(RangeInclusive<u32>, Vec<&str>)> {
    let mut records = Vec::new();
    for line in data.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }

        let mut fields = line.split(';').map(str::trim);
        let range = parse_range(fields.next().unwrap());
        records.push((range, fields.collect()));
    }

    records
}

fn parse_range(s: &str) -> RangeInclusive<u32> {
    match s.split_once("..") {
        Some((start, end)) => {
            u32::from_str_radix(start, 16).unwrap()..=u32::from_str_radix(end, 16).unwrap()
        }
        None => {
            let cp = u32::from_str_radix(s, 16).unwrap();
            cp..=cp
        }
    }
}

fn code_points(s: &str) -> Vec<u32> {
    s.split_whitespace()
        .map(|cp| u32::from_str_radix(cp, 16).unwrap())
        .collect()
}

fn all_chars() -> impl Iterator<Item = char> {
    (0..=0x10_ffff).filter_map(char::from_u32)
}

fn read(name: &str) -> String {
    fs::read_to_string(format!("{}/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const BIDI_CLASSES: &[(&str, &str)] = &[
    ("L", "Left_To_Right"),
    ("R", "Right_To_Left"),
    ("AL", "Arabic_Letter"),
    ("EN", "European_Number"),
    ("ES", "European_Separator"),
    ("ET", "European_Terminator"),
    ("AN", "Arabic_Number"),
    ("CS", "Common_Separator"),
    ("NSM", "Nonspacing_Mark"),
    ("BN", "Boundary_Neutral"),
    ("B", "Paragraph_Separator"),
    ("S", "Segment_Separator"),
    ("WS", "White_Space"),
    ("ON", "Other_Neutral"),
    ("LRE", "Left_To_Right_Embedding"),
    ("LRO", "Left_To_Right_Override"),
    ("RLE", "Right_To_Left_Embedding"),
    ("RLO", "Right_To_Left_Override"),
    ("PDF", "Pop_Directional_Format"),
    ("LRI", "Left_To_Right_Isolate"),
    ("RLI", "Right_To_Left_Isolate"),
    ("FSI", "First_Strong_Isolate"),
    ("PDI", "Pop_Directional_Isolate"),
];

const JOINING_TYPES: &[(&str, &str)] = &[
    ("U", "Non_Joining"),
    ("C", "Join_Causing"),
    ("D", "Dual_Joining"),
    ("L", "Left_Joining"),
    ("R", "Right_Joining"),
    ("T", "Transparent"),
];
