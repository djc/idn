//! Unicode data and algorithms underlying UTS #46 processing
//!
//! This module contains the building blocks of [UTS #46] processing that depend on Unicode data:
//! the Map and Normalize steps, and the character properties used by the validity criteria. The
//! crate root re-exports its public items.
//!
//! [UTS #46]: https://www.unicode.org/reports/tr46/

use alloc::vec::Vec;
use core::iter::Fuse;
use core::mem;

use crate::tables;

/// An iterator applying the Map and Normalize steps of UTS #46 processing
///
/// The mapping follows Nontransitional Processing:
///
/// - _ignored_ code points are removed or replaced, depending on the [`Mode`],
/// - _mapped_ code points are replaced by their mapping,
/// - _deviation_ and _valid_ code points are left unchanged,
/// - _disallowed_ code points are replaced by U+FFFD REPLACEMENT CHARACTER (which is itself
///   _disallowed_), so that they are easy to detect afterwards.
///
/// The output is in Normalization Form C. No ASCII deny list (such as _UseSTD3ASCIIRules_) is
/// applied.
///
/// ```
/// use idn::{Mode, Normalize};
///
/// let mapped = Normalize::new("Bu\u{308}cher".chars(), Mode::Map).collect::<String>();
/// assert_eq!(mapped, "bücher");
/// ```
#[derive(Clone, Debug)]
pub struct Normalize<I> {
    input: Fuse<I>,
    normalizer: Normalizer,
    ready: Vec<char>,
    position: usize,
}

impl<I: Iterator<Item = char>> Normalize<I> {
    /// Creates an iterator that maps and normalizes `input` in the given `mode`
    pub fn new(input: impl IntoIterator<Item = char, IntoIter = I>, mode: Mode) -> Self {
        Self {
            input: input.into_iter().fuse(),
            normalizer: Normalizer::new(mode),
            ready: Vec::new(),
            position: 0,
        }
    }
}

impl<I: Iterator<Item = char>> Iterator for Normalize<I> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        loop {
            if let Some(&c) = self.ready.get(self.position) {
                self.position += 1;
                return Some(c);
            }

            self.ready.clear();
            self.position = 0;
            match self.input.next() {
                Some(c) => self.normalizer.push(c, &mut self.ready),
                None => {
                    self.normalizer.finish(&mut self.ready);
                    if self.ready.is_empty() {
                        return None;
                    }
                }
            }
        }
    }
}

/// How [`Normalize`] treats _ignored_ code points
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Removes _ignored_ code points, as the Map step does
    Map,
    /// Replaces _ignored_ code points by U+FFFD REPLACEMENT CHARACTER
    ///
    /// This applies the NFC and code point status validity criteria for Nontransitional
    /// Processing: a label satisfies them if and only if the output is identical to the input
    /// and does not contain U+FFFD REPLACEMENT CHARACTER. (U+FFFD is itself _disallowed_, but
    /// is left unchanged.)
    Validate,
}

/// Returns whether the General_Category of `c` is Mark (Mn, Mc or Me)
pub fn is_mark(c: char) -> bool {
    PROPERTIES.get(c) & MARK_BIT != 0
}

/// Returns whether the Canonical_Combining_Class of `c` is Virama
pub fn is_virama(c: char) -> bool {
    Entry::of(c).ccc() == VIRAMA
}

/// Values of the Unicode Bidi_Class property
///
/// Variants are listed in the order of Table 4 in [UAX #9].
///
/// [UAX #9]: https://www.unicode.org/reports/tr9/#Table_Bidirectional_Character_Types
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BidiClass {
    /// Left_To_Right (L)
    LeftToRight,
    /// Right_To_Left (R)
    RightToLeft,
    /// Arabic_Letter (AL)
    ArabicLetter,
    /// European_Number (EN)
    EuropeanNumber,
    /// European_Separator (ES)
    EuropeanSeparator,
    /// European_Terminator (ET)
    EuropeanTerminator,
    /// Arabic_Number (AN)
    ArabicNumber,
    /// Common_Separator (CS)
    CommonSeparator,
    /// Nonspacing_Mark (NSM)
    NonspacingMark,
    /// Boundary_Neutral (BN)
    BoundaryNeutral,
    /// Paragraph_Separator (B)
    ParagraphSeparator,
    /// Segment_Separator (S)
    SegmentSeparator,
    /// White_Space (WS)
    WhiteSpace,
    /// Other_Neutral (ON)
    OtherNeutral,
    /// Left_To_Right_Embedding (LRE)
    LeftToRightEmbedding,
    /// Left_To_Right_Override (LRO)
    LeftToRightOverride,
    /// Right_To_Left_Embedding (RLE)
    RightToLeftEmbedding,
    /// Right_To_Left_Override (RLO)
    RightToLeftOverride,
    /// Pop_Directional_Format (PDF)
    PopDirectionalFormat,
    /// Left_To_Right_Isolate (LRI)
    LeftToRightIsolate,
    /// Right_To_Left_Isolate (RLI)
    RightToLeftIsolate,
    /// First_Strong_Isolate (FSI)
    FirstStrongIsolate,
    /// Pop_Directional_Isolate (PDI)
    PopDirectionalIsolate,
}

impl From<char> for BidiClass {
    /// Returns the Bidi_Class property value of `c`
    fn from(c: char) -> Self {
        BIDI_CLASSES[usize::from(PROPERTIES.get(c) & BIDI_CLASS_MASK)]
    }
}

/// Values of the Unicode Joining_Type property
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JoiningType {
    /// Dual_Joining (D)
    DualJoining,
    /// Join_Causing (C)
    JoinCausing,
    /// Left_Joining (L)
    LeftJoining,
    /// Non_Joining (U)
    NonJoining,
    /// Right_Joining (R)
    RightJoining,
    /// Transparent (T)
    Transparent,
}

impl From<char> for JoiningType {
    /// Returns the Joining_Type property value of `c`
    fn from(c: char) -> Self {
        JOINING_TYPES[usize::from((PROPERTIES.get(c) >> JOINING_TYPE_SHIFT) & JOINING_TYPE_MASK)]
    }
}

/// Whether `label` only contains valid or deviation code points and is in NFC
///
/// Equivalent to checking the output of [`Normalize`] in [`Mode::Validate`] as described there,
/// but avoids the normalization work if every code point is known to be unchanged by it.
pub(crate) fn is_valid_nfc(label: &[char]) -> bool {
    let mut quick = true;
    for &c in label {
        let entry = Entry::of(c);
        match entry.kind() {
            Kind::Keep => {}
            Kind::PoolComposed => quick &= entry.pool_composed().0 == c,
            // Normalization leaves U+FFFD REPLACEMENT CHARACTER unchanged, although it is
            // disallowed, so these cannot be left to the comparison below
            Kind::Disallowed | Kind::Ignored => return false,
            Kind::SingleBoundary | Kind::Single | Kind::PoolBoundary | Kind::Pool => quick = false,
        }
    }

    if quick {
        return true;
    }

    let mut normalized = Vec::with_capacity(label.len());
    normalize_into(label.iter().copied(), Mode::Validate, &mut normalized);
    normalized == label
}

/// Whether the Map step removes `c`
pub(crate) fn is_ignored(c: char) -> bool {
    !c.is_ascii() && Entry::of(c).kind() == Kind::Ignored
}

/// Whether the Map step replaces `c` by U+FFFD REPLACEMENT CHARACTER
pub(crate) fn is_disallowed(c: char) -> bool {
    !c.is_ascii() && Entry::of(c).kind() == Kind::Disallowed
}

/// Whether the Map step turns `c` into U+002E FULL STOP
pub(crate) fn maps_to_full_stop(c: char) -> bool {
    if c.is_ascii() {
        return c == '.';
    }

    let entry = Entry::of(c);
    entry.kind() == Kind::SingleBoundary && entry.target(c) == '.'
}

/// Appends the output of [`Normalize`] for `input` to `out`
///
/// Unlike the iterator, this appends completed segments directly to `out`, without buffering.
pub(crate) fn normalize_into(
    input: impl IntoIterator<Item = char>,
    mode: Mode,
    out: &mut Vec<char>,
) {
    let mut normalizer = Normalizer::new(mode);
    for c in input {
        normalizer.push(c, out);
    }

    normalizer.finish(out);
}

/// Incremental implementation of UTS #46 mapping followed by NFC normalization
///
/// The mapping data stores, for each code point, the canonical decomposition of its mapping.
/// Because canonical decomposition distributes over concatenation (up to canonical reordering),
/// mapping and decomposing each input code point, then applying the Canonical Ordering and
/// Canonical Composition algorithms yields the NFC form of the mapped string.
///
/// The decomposed stream is processed in segments. A segment starts at a code point that has
/// Canonical_Combining_Class 0 and never composes with a preceding code point (a "boundary"),
/// and extends up to the next boundary. Reordering and composition never cross a boundary, so
/// each segment is finalized as soon as the next boundary is seen.
///
/// Most segments consist of a single mapped code point followed by a boundary. For those, the
/// mapping data directly provides the composed result, and no decomposition or composition work
/// is needed.
#[derive(Clone, Debug)]
struct Normalizer {
    mode: Mode,
    pending: Pending,
    segment: Vec<(char, u8)>,
}

impl Normalizer {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            pending: Pending::Empty,
            segment: Vec::new(),
        }
    }

    /// Feeds `c` to the normalizer, appending any completed output to `out`
    ///
    /// ASCII code points are all valid or mapped to their lowercase form, and they are all
    /// composition boundaries, so they skip the table lookup.
    fn push(&mut self, c: char, out: &mut Vec<char>) {
        if c.is_ascii() {
            self.start(Pending::Single(c.to_ascii_lowercase()), out);
            return;
        }

        let entry = Entry::of(c);
        match entry.kind() {
            Kind::Keep => self.start(Pending::Single(c), out),
            Kind::Disallowed => self.start(Pending::Single(REPLACEMENT), out),
            Kind::Ignored => match self.mode {
                Mode::Map => {}
                Mode::Validate => self.start(Pending::Single(REPLACEMENT), out),
            },
            Kind::SingleBoundary => self.start(Pending::Single(entry.target(c)), out),
            Kind::Single => {
                self.buffer();
                self.push_decomposed(entry.target(c));
            }
            Kind::PoolComposed => {
                let (composed, decomposed) = entry.pool_composed();
                self.start(Pending::Composed(composed, decomposed), out);
            }
            Kind::PoolBoundary => {
                self.flush(out);
                self.buffer();
                self.push_utf16(entry.pool());
            }
            Kind::Pool => {
                self.buffer();
                self.push_utf16(entry.pool());
            }
        }
    }

    /// Completes normalization, appending any remaining output to `out`
    fn finish(&mut self, out: &mut Vec<char>) {
        self.flush(out);
    }

    /// Starts a new segment with `pending`, after finalizing the current one
    fn start(&mut self, pending: Pending, out: &mut Vec<char>) {
        self.flush(out);
        self.pending = pending;
    }

    /// Finalizes the current segment, appending its NFC form to `out`
    fn flush(&mut self, out: &mut Vec<char>) {
        match mem::replace(&mut self.pending, Pending::Empty) {
            Pending::Empty => {}
            Pending::Single(c) | Pending::Composed(c, _) => out.push(c),
            Pending::Buffered => {
                compose(&mut self.segment, out);
                self.segment.clear();
            }
        }
    }

    /// Ensures that the current segment is stored in decomposed form in the segment buffer
    fn buffer(&mut self) {
        match mem::replace(&mut self.pending, Pending::Buffered) {
            Pending::Empty | Pending::Buffered => {}
            Pending::Single(c) => match hangul_decomposition(c) {
                Some((l, v, t)) => {
                    self.segment.push((l, 0));
                    self.segment.push((v, 0));
                    if let Some(t) = t {
                        self.segment.push((t, 0));
                    }
                }
                None => self.push_decomposed(c),
            },
            Pending::Composed(_, decomposed) => self.push_utf16(decomposed),
        }
    }

    /// Appends decomposed code points from the UTF-16 mapping pool to the segment buffer
    fn push_utf16(&mut self, units: &'static [u16]) {
        for c in char::decode_utf16(units.iter().copied()) {
            self.push_decomposed(c.unwrap_or(REPLACEMENT));
        }
    }

    fn push_decomposed(&mut self, c: char) {
        self.segment.push((c, Entry::of(c).ccc()));
    }
}

/// The segment currently being normalized
#[derive(Clone, Copy, Debug)]
enum Pending {
    Empty,
    /// A single code point that is its own NFD form, or a Hangul syllable
    Single(char),
    /// A segment with the given NFD form, which composes to a single code point
    Composed(char, &'static [u16]),
    /// A segment stored in the segment buffer, in NFD form but not yet canonically ordered
    Buffered,
}

/// Applies the Canonical Ordering and Canonical Composition algorithms to a decomposed segment
///
/// The result is appended to `out`. See UAX #15, sections 3.11 and 3.10.
fn compose(segment: &mut [(char, u8)], out: &mut Vec<char>) {
    // Swap adjacent non-starters whose combining classes are out of order. This is an insertion
    // sort, which is stable and fast for the short runs of non-starters found in practice.
    for i in 1..segment.len() {
        let ccc = segment[i].1;
        let mut j = i;
        while ccc != 0 && j > 0 && segment[j - 1].1 > ccc {
            segment.swap(j - 1, j);
            j -= 1;
        }
    }

    let mut starter = None;
    let mut last_ccc = None;
    for &(c, ccc) in segment.iter() {
        if let Some(index) = starter {
            if last_ccc.map_or(true, |last| last < ccc) {
                if let Some(composed) = compose_pair(out[index], c) {
                    out[index] = composed;
                    continue;
                }
            }
        }

        match ccc {
            0 => {
                starter = Some(out.len());
                last_ccc = None;
            }
            _ => last_ccc = Some(ccc),
        }

        out.push(c);
    }
}

/// Returns the primary composite for the pair `(first, second)`, if any
fn compose_pair(first: char, second: char) -> Option<char> {
    let (first_cp, second_cp) = (u32::from(first), u32::from(second));
    if let (Some(l), Some(v)) = (
        first_cp
            .checked_sub(HANGUL_L_BASE)
            .filter(|&l| l < HANGUL_L_COUNT),
        second_cp
            .checked_sub(HANGUL_V_BASE)
            .filter(|&v| v < HANGUL_V_COUNT),
    ) {
        return char::from_u32(HANGUL_S_BASE + (l * HANGUL_V_COUNT + v) * HANGUL_T_COUNT);
    }

    if let (Some(s), Some(t)) = (
        first_cp
            .checked_sub(HANGUL_S_BASE)
            .filter(|&s| s < HANGUL_S_COUNT && s % HANGUL_T_COUNT == 0),
        second_cp
            .checked_sub(HANGUL_T_BASE)
            .filter(|&t| t > 0 && t < HANGUL_T_COUNT),
    ) {
        return char::from_u32(HANGUL_S_BASE + s + t);
    }

    match (u16::try_from(first_cp), u16::try_from(second_cp)) {
        (Ok(first), Ok(second)) => {
            let index = tables::COMPOSITIONS_BMP
                .binary_search_by(|&(a, b, _)| (a, b).cmp(&(first, second)))
                .ok()?;
            char::from_u32(u32::from(tables::COMPOSITIONS_BMP[index].2))
        }
        _ => {
            let index = tables::COMPOSITIONS_SUPPLEMENTARY
                .binary_search_by(|&(a, b, _)| (a, b).cmp(&(first, second)))
                .ok()?;
            Some(tables::COMPOSITIONS_SUPPLEMENTARY[index].2)
        }
    }
}

/// Returns the canonical decomposition of `c` if it is a precomposed Hangul syllable
fn hangul_decomposition(c: char) -> Option<(char, char, Option<char>)> {
    let s = u32::from(c)
        .checked_sub(HANGUL_S_BASE)
        .filter(|&s| s < HANGUL_S_COUNT)?;
    let l = char::from_u32(HANGUL_L_BASE + s / (HANGUL_V_COUNT * HANGUL_T_COUNT))?;
    let v =
        char::from_u32(HANGUL_V_BASE + (s % (HANGUL_V_COUNT * HANGUL_T_COUNT)) / HANGUL_T_COUNT)?;
    let t = match s % HANGUL_T_COUNT {
        0 => None,
        t => Some(char::from_u32(HANGUL_T_BASE + t)?),
    };
    Some((l, v, t))
}

/// A mapping table entry, laid out as described in `tests/codegen.rs`
#[derive(Clone, Copy, Debug)]
struct Entry(u32);

impl Entry {
    fn of(c: char) -> Self {
        Self(MAPPING.get(c))
    }

    fn kind(self) -> Kind {
        KINDS[((self.0 >> KIND_SHIFT) & KIND_MASK) as usize]
    }

    /// The single code point that `c` maps to (for the `Single*` kinds)
    ///
    /// The generator guarantees that the target is a valid `char`, which the exhaustive tests
    /// confirm, so the fallback is never used.
    fn target(self, c: char) -> char {
        let target = u32::from(c).wrapping_add(self.payload()) & PAYLOAD_MASK;
        char::from_u32(target).unwrap_or(REPLACEMENT)
    }

    /// The composed and decomposed (UTF-16) forms (for `Kind::PoolComposed`)
    ///
    /// The pool stores the composed form, followed by the decomposed form of length `len`.
    fn pool_composed(self) -> (char, &'static [u16]) {
        let (offset, len) = self.pool_range();
        let pool = &tables::MAPPING_POOL[offset..];
        let composed = match char::decode_utf16(pool.iter().copied()).next() {
            Some(Ok(composed)) => composed,
            Some(Err(_)) | None => REPLACEMENT,
        };

        let start = composed.len_utf16();
        (composed, &pool[start..start + len])
    }

    /// The decomposed form, in UTF-16 (for `Kind::Pool` and `Kind::PoolBoundary`)
    fn pool(self) -> &'static [u16] {
        let (offset, len) = self.pool_range();
        &tables::MAPPING_POOL[offset..offset + len]
    }

    fn pool_range(self) -> (usize, usize) {
        let payload = self.payload() as usize;
        (payload & POOL_OFFSET_MASK, payload >> POOL_OFFSET_BITS)
    }

    fn payload(self) -> u32 {
        self.0 & PAYLOAD_MASK
    }

    fn ccc(self) -> u8 {
        (self.0 >> CCC_SHIFT) as u8
    }
}

/// How a code point is mapped (must be kept in sync with `tests/codegen.rs`)
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    /// Valid, its own NFD form and a boundary; or a Hangul syllable
    Keep,
    /// Disallowed, maps to U+FFFD
    Disallowed,
    /// Ignored, maps to the empty string
    Ignored,
    /// Maps to a single code point (in NFD) which is a boundary
    SingleBoundary,
    /// Maps to a single code point (in NFD) which is not a boundary
    Single,
    /// Maps to a sequence (in NFD) starting with a boundary, composing to a single code point
    PoolComposed,
    /// Maps to a sequence (in NFD) starting with a boundary
    PoolBoundary,
    /// Maps to a sequence (in NFD) starting with a code point that is not a boundary
    Pool,
}

const KINDS: [Kind; 8] = [
    Kind::Keep,
    Kind::Disallowed,
    Kind::Ignored,
    Kind::SingleBoundary,
    Kind::Single,
    Kind::PoolComposed,
    Kind::PoolBoundary,
    Kind::Pool,
];

/// A three-level lookup table covering all code points (see `tests/codegen.rs`)
struct Trie<T: 'static> {
    first: &'static [u8],
    second: &'static [u16],
    third: &'static [T],
}

impl<T: Copy> Trie<T> {
    #[inline]
    fn get(&self, c: char) -> T {
        let cp = u32::from(c) as usize;
        let block = usize::from(self.first[cp >> (tables::SECOND_BITS + tables::THIRD_BITS)]);
        let index = (block << tables::SECOND_BITS) | ((cp >> tables::THIRD_BITS) & SECOND_MASK);
        let offset = usize::from(self.second[index]);
        self.third[offset + (cp & THIRD_MASK)]
    }
}

static MAPPING: Trie<u32> = Trie {
    first: &tables::MAPPING_FIRST,
    second: &tables::MAPPING_SECOND,
    third: &tables::MAPPING_THIRD,
};

static PROPERTIES: Trie<u16> = Trie {
    first: &tables::PROPERTIES_FIRST,
    second: &tables::PROPERTIES_SECOND,
    third: &tables::PROPERTIES_THIRD,
};

const SECOND_MASK: usize = (1 << tables::SECOND_BITS) - 1;
const THIRD_MASK: usize = (1 << tables::THIRD_BITS) - 1;

const PAYLOAD_MASK: u32 = (1 << 21) - 1;
const POOL_OFFSET_BITS: u32 = 16;
const POOL_OFFSET_MASK: usize = (1 << POOL_OFFSET_BITS) - 1;
const KIND_SHIFT: u32 = 21;
const KIND_MASK: u32 = 0b111;
const CCC_SHIFT: u32 = 24;

const BIDI_CLASS_MASK: u16 = 0b1_1111;
const JOINING_TYPE_SHIFT: u16 = 5;
const JOINING_TYPE_MASK: u16 = 0b111;
const MARK_BIT: u16 = 1 << 8;

/// In the order used by `tests/codegen.rs`
const BIDI_CLASSES: [BidiClass; 23] = [
    BidiClass::LeftToRight,
    BidiClass::RightToLeft,
    BidiClass::ArabicLetter,
    BidiClass::EuropeanNumber,
    BidiClass::EuropeanSeparator,
    BidiClass::EuropeanTerminator,
    BidiClass::ArabicNumber,
    BidiClass::CommonSeparator,
    BidiClass::NonspacingMark,
    BidiClass::BoundaryNeutral,
    BidiClass::ParagraphSeparator,
    BidiClass::SegmentSeparator,
    BidiClass::WhiteSpace,
    BidiClass::OtherNeutral,
    BidiClass::LeftToRightEmbedding,
    BidiClass::LeftToRightOverride,
    BidiClass::RightToLeftEmbedding,
    BidiClass::RightToLeftOverride,
    BidiClass::PopDirectionalFormat,
    BidiClass::LeftToRightIsolate,
    BidiClass::RightToLeftIsolate,
    BidiClass::FirstStrongIsolate,
    BidiClass::PopDirectionalIsolate,
];

/// In the order used by `tests/codegen.rs`
const JOINING_TYPES: [JoiningType; 6] = [
    JoiningType::NonJoining,
    JoiningType::JoinCausing,
    JoiningType::DualJoining,
    JoiningType::LeftJoining,
    JoiningType::RightJoining,
    JoiningType::Transparent,
];

const VIRAMA: u8 = 9;
pub(crate) const REPLACEMENT: char = '\u{fffd}';

const HANGUL_S_BASE: u32 = 0xac00;
const HANGUL_L_BASE: u32 = 0x1100;
const HANGUL_V_BASE: u32 = 0x1161;
const HANGUL_T_BASE: u32 = 0x11a7;
const HANGUL_L_COUNT: u32 = 19;
const HANGUL_V_COUNT: u32 = 21;
const HANGUL_T_COUNT: u32 = 28;
const HANGUL_S_COUNT: u32 = HANGUL_L_COUNT * HANGUL_V_COUNT * HANGUL_T_COUNT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_nfc_matches_normalize_validate() {
        let marks = ['\u{301}', '\u{323}', '\u{3099}', '\u{1161}', '\u{11a8}'];
        for c in (0..=0x10_ffff).filter_map(char::from_u32) {
            let mark = marks[u32::from(c) as usize % marks.len()];
            for label in [&[c][..], &[c, mark][..], &['x', mark, c][..]] {
                let normalized =
                    Normalize::new(label.iter().copied(), Mode::Validate).collect::<Vec<_>>();
                let expected = normalized == label && !normalized.contains(&REPLACEMENT);
                assert_eq!(is_valid_nfc(label), expected, "{label:?}");
            }
        }
    }

    #[test]
    fn predicates_match_map_normalize() {
        for c in (0..=0x10_ffff).filter_map(char::from_u32) {
            let mapped = Normalize::new([c], Mode::Map).collect::<Vec<_>>();
            assert_eq!(is_ignored(c), mapped.is_empty(), "{c:?}");
            assert_eq!(is_disallowed(c), mapped.contains(&REPLACEMENT), "{c:?}");
            assert_eq!(maps_to_full_stop(c), mapped == ['.'], "{c:?}");
        }
    }
}
