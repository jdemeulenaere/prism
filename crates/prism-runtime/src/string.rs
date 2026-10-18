//! Strings (D2, D3, D4 and D7 of `runtime-strings-and-drops`).
//!
//! A `String` is a [`PrismString`]: a pointer to UTF-8 text, the text's length in bytes, its
//! number of characters (Unicode scalar values) and the capacity of the buffer the value owns.
//! A capacity of 0 means the value owns no memory: its text is static data, a literal of a code
//! unit or a text of the runtime such as `"true"`, or empty. A positive capacity means the value
//! alone owns a buffer of exactly that many bytes, allocated through
//! [`crate::prism_rt_allocate`] with alignment 1, holding its text at the start. Every string the
//! runtime builds is exactly sized (capacity equal to byte length), since nothing appends to a
//! string yet.
//!
//! Generated code inlines building a literal and reading `length`; everything else is a call of
//! a function here, taking strings by address and writing a result string to a destination
//! address, as aggregates are returned.

use crate::allocation::{prism_rt_allocate, prism_rt_free};
use crate::double::format_double;
use crate::output::{append, format_bool, format_char, format_int};
use crate::panic::{panic_at, panic_without_location, text, PanicKind};

/// A `String` value, laid out as the backend lays it out: 32 bytes on a 64-bit target.
#[repr(C)]
#[derive(Debug)]
pub struct PrismString {
    /// The first byte of the UTF-8 text; never null, even for an empty text.
    pub pointer: *const u8,
    /// The text's length in bytes.
    pub byte_length: usize,
    /// The number of Unicode scalar values in the text, equal to `byte_length` exactly when the
    /// text is ASCII.
    pub character_count: usize,
    /// The size of the owned buffer in bytes, or 0 when the value owns none.
    pub capacity: usize,
}

/// The text every empty string points at.
static EMPTY_TEXT: u8 = 0;

impl PrismString {
    /// The empty string, which owns nothing.
    pub fn empty() -> Self {
        Self::of_static("")
    }

    /// A string holding `text`, which lives as long as the program, owning nothing.
    pub fn of_static(text: &'static str) -> Self {
        if text.is_empty() {
            return PrismString {
                pointer: &EMPTY_TEXT,
                byte_length: 0,
                character_count: 0,
                capacity: 0,
            };
        }
        PrismString {
            pointer: text.as_ptr(),
            byte_length: text.len(),
            character_count: text.chars().count(),
            capacity: 0,
        }
    }

    /// A string owning a new buffer holding `text`, or the empty string, which owns nothing.
    pub fn owned(text: &str) -> Self {
        Self::from_parts(&[text.as_bytes()], text.chars().count())
    }

    /// A string holding the concatenation of `parts`, which have `character_count` characters
    /// together, in a new, exactly sized buffer; the empty string when they are all empty.
    fn from_parts(parts: &[&[u8]], character_count: usize) -> Self {
        let byte_length = parts
            .iter()
            .try_fold(0usize, |total, part| total.checked_add(part.len()))
            .unwrap_or_else(|| panic_without_location(PanicKind::OutOfMemory, ""));
        if byte_length == 0 {
            return Self::empty();
        }
        let buffer = prism_rt_allocate(byte_length, 1);
        let mut offset = 0;
        for part in parts {
            // SAFETY: the buffer has room for every part, and does not overlap them.
            unsafe {
                std::ptr::copy_nonoverlapping(part.as_ptr(), buffer.add(offset), part.len());
            }
            offset += part.len();
        }
        PrismString {
            pointer: buffer,
            byte_length,
            character_count,
            capacity: byte_length,
        }
    }

    /// The bytes of the text.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: a string's pointer is never null and points at its `byte_length` bytes.
        unsafe { std::slice::from_raw_parts(self.pointer, self.byte_length) }
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        // SAFETY: a string holds UTF-8: literals are, and the runtime only builds strings from
        // other strings and from Rust's text.
        unsafe { std::str::from_utf8_unchecked(self.bytes()) }
    }

    /// Whether the value owns its buffer.
    pub fn owns_buffer(&self) -> bool {
        self.capacity > 0
    }

    /// An independent copy: the same value when it owns nothing, a new buffer otherwise.
    fn duplicate(&self) -> Self {
        if !self.owns_buffer() {
            return PrismString { ..*self };
        }
        Self::from_parts(&[self.bytes()], self.character_count)
    }

    /// Frees the buffer the value owns, if any.
    ///
    /// # Safety
    ///
    /// The value must not be used afterwards.
    unsafe fn release(&self) {
        if self.owns_buffer() {
            prism_rt_free(self.pointer as *mut u8, self.capacity, 1);
        }
    }
}

/// Writes `value` to `destination`, which holds no string yet.
///
/// # Safety
///
/// `destination` must be valid for a write of a [`PrismString`].
unsafe fn write(destination: *mut PrismString, value: PrismString) {
    destination.write(value);
}

/// `left + right`: a new string holding `left`'s text followed by `right`'s. When one operand is
/// empty, the result is a copy of the other as [`prism_rt_string_clone`] makes it, so it owns a
/// buffer only when that operand does; when both are, it is the empty string.
///
/// # Safety
///
/// `left` and `right` must point at strings and `destination` must be valid for a write of one;
/// `destination` may be either operand's address only if that string is not dropped afterwards.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_string_concatenate(
    destination: *mut PrismString,
    left: *const PrismString,
    right: *const PrismString,
) {
    let (left, right) = (&*left, &*right);
    let result = if left.byte_length == 0 {
        right.duplicate()
    } else if right.byte_length == 0 {
        left.duplicate()
    } else {
        let count = left
            .character_count
            .checked_add(right.character_count)
            .unwrap_or_else(|| panic_without_location(PanicKind::OutOfMemory, ""));
        PrismString::from_parts(&[left.bytes(), right.bytes()], count)
    };
    write(destination, result);
}

/// `left == right`: 1 when the two strings hold the same text, 0 otherwise. UTF-8 is canonical
/// for a sequence of scalar values, so equal texts have equal bytes.
///
/// # Safety
///
/// `left` and `right` must point at strings.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_string_equals(
    left: *const PrismString,
    right: *const PrismString,
) -> u8 {
    let (left, right) = (&*left, &*right);
    u8::from(left.byte_length == right.byte_length && left.bytes() == right.bytes())
}

/// `string[index]`: the character at character index `index`, read in O(1) from ASCII text and
/// by a scan from the start otherwise. An index outside `0..<length` raises "string index <i> out
/// of range for length <n>" at `file:line:column`, the location of the indexing expression.
///
/// # Safety
///
/// `string` must point at a string, and `file` at `file_length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_string_character(
    string: *const PrismString,
    index: i64,
    file: *const u8,
    file_length: usize,
    line: u32,
    column: u32,
) -> u32 {
    let string = &*string;
    let character = usize::try_from(index)
        .ok()
        .filter(|index| *index < string.character_count)
        .and_then(|index| {
            if string.character_count == string.byte_length {
                Some(u32::from(string.bytes()[index]))
            } else {
                string.as_str().chars().nth(index).map(u32::from)
            }
        });
    match character {
        Some(character) => character,
        None => panic_at(
            PanicKind::StringIndexOutOfRange,
            &format!(
                "{} out of range for length {}",
                index, string.character_count
            ),
            &text(file, file_length),
            line,
            column,
        ),
    }
}

/// `source.clone()`: a copy of the value when it owns no buffer, since nothing can change or
/// free static text; otherwise a new, exactly sized buffer holding the same text.
///
/// # Safety
///
/// `source` must point at a string and `destination` must be valid for a write of one.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_string_clone(
    destination: *mut PrismString,
    source: *const PrismString,
) {
    let copy = (*source).duplicate();
    write(destination, copy);
}

/// Drops the string at `string`: frees its buffer when it owns one.
///
/// # Safety
///
/// `string` must point at a string that is not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_string_drop(string: *mut PrismString) {
    (*string).release();
}

/// `print` of a `String`: writes its text.
///
/// # Safety
///
/// `string` must point at a string.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_print_string(string: *const PrismString) {
    append((*string).bytes());
}

/// `toString()` of an `Int`: the text `print` writes.
///
/// # Safety
///
/// `destination` must be valid for a write of a string.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_int_to_string(destination: *mut PrismString, value: i64) {
    write(destination, PrismString::owned(&format_int(value)));
}

/// `toString()` of a `Double`: the text `print` writes ("Double Text Form").
///
/// # Safety
///
/// `destination` must be valid for a write of a string.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_double_to_string(destination: *mut PrismString, value: f64) {
    write(destination, PrismString::owned(&format_double(value)));
}

/// `toString()` of a `Bool`, passed as a byte holding 0 or 1: static `true` or `false`, which
/// allocates nothing.
///
/// # Safety
///
/// `destination` must be valid for a write of a string.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_bool_to_string(destination: *mut PrismString, value: u8) {
    write(destination, PrismString::of_static(format_bool(value != 0)));
}

/// `toString()` of a `Char`, passed as its scalar value: its UTF-8 encoding.
///
/// # Safety
///
/// `destination` must be valid for a write of a string.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_char_to_string(destination: *mut PrismString, value: u32) {
    write(destination, PrismString::owned(&format_char(value)));
}

/// One part of a string template, as generated code lays out the table it passes to
/// [`prism_rt_string_template`] (D12 of `string-templates`): a kind word and three payload words,
/// 32 bytes on a 64-bit target.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TemplatePart {
    /// One of the `TEMPLATE_*` kinds.
    pub kind: u64,
    /// For [`TEMPLATE_TEXT`], the address, byte length and character count of static text; for
    /// [`TEMPLATE_STRING`], the address of a `PrismString`; for a scalar, its value in the first
    /// word (an `Int`, the bits of a `Double`, 0 or 1 for a `Bool`, a `Char`'s scalar value).
    pub words: [u64; 3],
}

/// Static text: address, byte length, character count.
pub const TEMPLATE_TEXT: u64 = 0;
/// A borrowed string, by address.
pub const TEMPLATE_STRING: u64 = 1;
/// An `Int`.
pub const TEMPLATE_INT: u64 = 2;
/// A `Double`, by its bits.
pub const TEMPLATE_DOUBLE: u64 = 3;
/// A `Bool`, 0 or 1.
pub const TEMPLATE_BOOL: u64 = 4;
/// A `Char`, by its scalar value.
pub const TEMPLATE_CHAR: u64 = 5;

/// The decimal digits of `value`, with its sign, written at the end of `buffer`; returns where
/// they start. 20 bytes hold `i64::MIN`.
fn int_digits(value: i64, buffer: &mut [u8; 20]) -> usize {
    let mut magnitude = value.unsigned_abs();
    let mut start = buffer.len();
    loop {
        start -= 1;
        buffer[start] = b'0' + (magnitude % 10) as u8;
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
    if value < 0 {
        start -= 1;
        buffer[start] = b'-';
    }
    start
}

/// The number of bytes of the text of `value`, its sign included.
fn int_length(value: i64) -> usize {
    let mut magnitude = value.unsigned_abs();
    let mut length = usize::from(value < 0) + 1;
    while magnitude >= 10 {
        magnitude /= 10;
        length += 1;
    }
    length
}

/// A string template's value (D12 of `string-templates`): the texts of the `count` parts of
/// `parts`, in order, in a new string of exactly their length, allocated once, or the empty
/// string, which owns nothing. A scalar part has the text `print` writes. The parts are measured
/// first: text and strings by their stored lengths and counts, `Int`, `Bool` and `Char` without
/// formatting them, and each `Double` by its text, formatted once into the runtime's own scratch
/// memory, which is not a program allocation. A total that does not fit an `isize` is "out of
/// memory".
///
/// # Safety
///
/// `parts` must point at `count` parts, whose text parts point at their bytes and whose string
/// parts point at strings, and `destination` must be valid for a write of a string; the
/// destination may not be the address of a string part.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_string_template(
    destination: *mut PrismString,
    parts: *const TemplatePart,
    count: usize,
) {
    let parts = if count == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(parts, count)
    };
    let out_of_memory = || panic_without_location(PanicKind::OutOfMemory, "");
    let mut doubles: Vec<String> = Vec::new();
    let (mut byte_length, mut character_count) = (0usize, 0usize);
    for part in parts {
        let [first, second, third] = part.words;
        let (bytes, characters) = match part.kind {
            TEMPLATE_TEXT => (second as usize, third as usize),
            TEMPLATE_STRING => {
                let string = &*(first as usize as *const PrismString);
                (string.byte_length, string.character_count)
            }
            TEMPLATE_INT => {
                let length = int_length(first as i64);
                (length, length)
            }
            TEMPLATE_DOUBLE => {
                let text = format_double(f64::from_bits(first));
                let length = text.len();
                doubles.push(text);
                (length, length)
            }
            TEMPLATE_BOOL => (format_bool(first != 0).len(), format_bool(first != 0).len()),
            _ => (template_char(first).len_utf8(), 1),
        };
        byte_length = byte_length.checked_add(bytes).unwrap_or_else(out_of_memory);
        character_count += characters;
    }
    if byte_length > isize::MAX as usize {
        out_of_memory();
    }
    if byte_length == 0 {
        write(destination, PrismString::empty());
        return;
    }
    let buffer = prism_rt_allocate(byte_length, 1);
    let mut offset = 0;
    let mut doubles = doubles.iter();
    let mut put = |bytes: &[u8]| {
        // SAFETY: the parts measured `byte_length` bytes in all, and the buffer overlaps none.
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.add(offset), bytes.len());
        offset += bytes.len();
    };
    for part in parts {
        let [first, second, _] = part.words;
        match part.kind {
            TEMPLATE_TEXT => put(std::slice::from_raw_parts(
                first as usize as *const u8,
                second as usize,
            )),
            TEMPLATE_STRING => put((*(first as usize as *const PrismString)).bytes()),
            TEMPLATE_INT => {
                let mut digits = [0u8; 20];
                let start = int_digits(first as i64, &mut digits);
                put(&digits[start..]);
            }
            TEMPLATE_DOUBLE => put(doubles.next().map_or(&[][..], String::as_bytes)),
            TEMPLATE_BOOL => put(format_bool(first != 0).as_bytes()),
            _ => {
                let mut encoded = [0u8; 4];
                put(template_char(first).encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    write(
        destination,
        PrismString {
            pointer: buffer,
            byte_length,
            character_count,
            capacity: byte_length,
        },
    );
}

/// The `Char` of a template part's word, as [`format_char`] reads it.
fn template_char(word: u64) -> char {
    u32::try_from(word)
        .ok()
        .and_then(char::from_u32)
        .unwrap_or(char::REPLACEMENT_CHARACTER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocation::{allocation_statistics, start_tracking, test_lock};
    use std::mem::MaybeUninit;

    /// Runs `test` under the allocation check, then checks that it freed everything it
    /// allocated, and returns how many allocations it made.
    fn checked(test: impl FnOnce()) -> u64 {
        let _lock = test_lock();
        start_tracking(true);
        test();
        let statistics = allocation_statistics();
        start_tracking(false);
        assert_eq!(statistics.live_allocations, 0, "{:?}", statistics);
        statistics.allocations_made
    }

    fn call(f: impl FnOnce(*mut PrismString)) -> PrismString {
        let mut result = MaybeUninit::<PrismString>::uninit();
        f(result.as_mut_ptr());
        // SAFETY: every runtime function writes its destination.
        unsafe { result.assume_init() }
    }

    fn concatenate(left: &PrismString, right: &PrismString) -> PrismString {
        // SAFETY: both operands are strings.
        call(|destination| unsafe { prism_rt_string_concatenate(destination, left, right) })
    }

    fn drop_string(mut string: PrismString) {
        // SAFETY: the string is not used afterwards.
        unsafe { prism_rt_string_drop(&mut string) };
    }

    fn equals(left: &PrismString, right: &PrismString) -> bool {
        // SAFETY: both operands are strings.
        unsafe { prism_rt_string_equals(left, right) == 1 }
    }

    #[test]
    fn the_layout_is_four_words() {
        assert_eq!(
            std::mem::size_of::<PrismString>(),
            4 * std::mem::size_of::<usize>()
        );
        assert_eq!(
            std::mem::align_of::<PrismString>(),
            std::mem::align_of::<usize>()
        );
        assert!(!PrismString::empty().pointer.is_null());
    }

    #[test]
    fn concatenation_of_static_and_owned_texts() {
        let made = checked(|| {
            let static_text = PrismString::of_static("pri");
            let owned = PrismString::owned("sm");
            let joined = concatenate(&static_text, &owned);
            assert_eq!(
                (joined.as_str(), joined.character_count, joined.capacity),
                ("prism", 5, 5)
            );
            let accented = concatenate(&joined, &PrismString::of_static("é"));
            assert_eq!((accented.byte_length, accented.character_count), (7, 6));
            drop_string(accented);
            drop_string(joined);
            drop_string(owned);
            drop_string(static_text);
        });
        assert_eq!(made, 3);
    }

    #[test]
    fn concatenation_with_an_empty_operand_allocates_nothing() {
        let made = checked(|| {
            let empty = PrismString::empty();
            let literal = PrismString::of_static("hi");
            let both_empty = concatenate(&empty, &empty);
            assert_eq!((both_empty.byte_length, both_empty.capacity), (0, 0));
            let left = concatenate(&literal, &empty);
            let right = concatenate(&empty, &literal);
            assert_eq!((left.as_str(), left.capacity), ("hi", 0));
            assert_eq!((right.as_str(), right.capacity), ("hi", 0));
            for string in [both_empty, left, right] {
                drop_string(string);
            }
        });
        assert_eq!(made, 0);
    }

    #[test]
    fn equality_compares_the_text() {
        let a = PrismString::of_static("abc");
        let b = PrismString::of_static("abd");
        let c = PrismString::of_static("ab");
        checked(|| {
            let owned = PrismString::owned("abc");
            assert!(equals(&a, &owned));
            assert!(!equals(&a, &b));
            assert!(!equals(&a, &c));
            assert!(equals(&PrismString::empty(), &PrismString::of_static("")));
            drop_string(owned);
        });
    }

    fn character(string: &PrismString, index: i64) -> char {
        let file = "main.prism";
        // SAFETY: the string is a string and the file name is text.
        let value =
            unsafe { prism_rt_string_character(string, index, file.as_ptr(), file.len(), 1, 1) };
        char::from_u32(value).expect("a scalar value")
    }

    #[test]
    fn characters_of_ascii_and_other_text() {
        let ascii = PrismString::of_static("abc");
        assert_eq!([character(&ascii, 0), character(&ascii, 2)], ['a', 'c']);
        let text = PrismString::of_static("héllo🚀");
        assert_eq!(text.character_count, 6);
        assert_eq!(
            [
                character(&text, 1),
                character(&text, 4),
                character(&text, 5)
            ],
            ['é', 'o', '🚀']
        );
    }

    #[test]
    fn the_index_error_message() {
        assert_eq!(
            crate::panic::panic_message(
                PanicKind::StringIndexOutOfRange,
                "3 out of range for length 3"
            ),
            "string index 3 out of range for length 3"
        );
    }

    #[test]
    fn clones_are_independent_and_dropped_once() {
        let made = checked(|| {
            let literal = PrismString::of_static("hi");
            // SAFETY: the source is a string.
            let copy = call(|destination| unsafe { prism_rt_string_clone(destination, &literal) });
            assert_eq!((copy.pointer, copy.capacity), (literal.pointer, 0));
            let owned = PrismString::owned("héllo");
            // SAFETY: the source is a string.
            let clone = call(|destination| unsafe { prism_rt_string_clone(destination, &owned) });
            assert_ne!(clone.pointer, owned.pointer);
            assert_eq!(
                (clone.as_str(), clone.character_count, clone.capacity),
                ("héllo", 5, 6)
            );
            for string in [literal, copy, owned, clone] {
                drop_string(string);
            }
        });
        assert_eq!(made, 2);
    }

    #[test]
    fn scalars_convert_to_the_text_print_writes() {
        fn text_of(f: impl FnOnce(*mut PrismString)) -> String {
            let string = call(f);
            let text = string.as_str().to_string();
            drop_string(string);
            text
        }
        // SAFETY: each destination is valid for a write of a string.
        unsafe {
            checked(|| {
                assert_eq!(text_of(|d| prism_rt_int_to_string(d, -42)), "-42");
                assert_eq!(
                    text_of(|d| prism_rt_double_to_string(d, 0.1 + 0.2)),
                    "0.30000000000000004"
                );
                assert_eq!(text_of(|d| prism_rt_double_to_string(d, 1.0e7)), "1.0E7");
                assert_eq!(
                    text_of(|d| prism_rt_double_to_string(d, 5e-324)),
                    "4.9E-324"
                );
                assert_eq!(text_of(|d| prism_rt_char_to_string(d, 'é' as u32)), "é");
            });
            let made = checked(|| {
                assert_eq!(text_of(|d| prism_rt_bool_to_string(d, 1)), "true");
                assert_eq!(text_of(|d| prism_rt_bool_to_string(d, 0)), "false");
            });
            assert_eq!(made, 0);
        }
    }

    fn template(parts: &[TemplatePart]) -> PrismString {
        // SAFETY: the parts point at live texts and strings.
        call(|destination| unsafe {
            prism_rt_string_template(destination, parts.as_ptr(), parts.len())
        })
    }

    fn text_part(text: &'static str) -> TemplatePart {
        TemplatePart {
            kind: TEMPLATE_TEXT,
            words: [
                text.as_ptr() as u64,
                text.len() as u64,
                text.chars().count() as u64,
            ],
        }
    }

    fn string_part(string: &PrismString) -> TemplatePart {
        TemplatePart {
            kind: TEMPLATE_STRING,
            words: [string as *const PrismString as u64, 0, 0],
        }
    }

    fn scalar_part(kind: u64, word: u64) -> TemplatePart {
        TemplatePart {
            kind,
            words: [word, 0, 0],
        }
    }

    #[test]
    fn the_template_part_layout_is_four_words() {
        assert_eq!(std::mem::size_of::<TemplatePart>(), 32);
    }

    #[test]
    fn a_template_formats_every_kind_of_part_with_one_allocation() {
        let allocations = checked(|| {
            let owned = PrismString::owned("Ada");
            let result = template(&[
                text_part("é "),
                string_part(&owned),
                text_part(" "),
                scalar_part(TEMPLATE_INT, (-42i64) as u64),
                text_part("|"),
                scalar_part(TEMPLATE_INT, i64::MIN as u64),
                text_part("|"),
                scalar_part(TEMPLATE_DOUBLE, 1.0e7f64.to_bits()),
                text_part("|"),
                scalar_part(TEMPLATE_DOUBLE, 4.9e-324f64.to_bits()),
                text_part("|"),
                scalar_part(TEMPLATE_BOOL, 1),
                scalar_part(TEMPLATE_BOOL, 0),
                scalar_part(TEMPLATE_CHAR, 'é' as u64),
                scalar_part(TEMPLATE_INT, 0),
            ]);
            assert_eq!(
                result.as_str(),
                "é Ada -42|-9223372036854775808|1.0E7|4.9E-324|truefalseé0"
            );
            assert_eq!(result.character_count, result.as_str().chars().count());
            assert_eq!(result.capacity, result.byte_length);
            drop_string(result);
            drop_string(owned);
        });
        // One for `owned`, one for the template.
        assert_eq!(allocations, 2);
    }

    #[test]
    fn scalar_parts_have_the_text_print_writes() {
        checked(|| {
            for value in [0, 7, -7, 10, 99, 100, -100, i64::MAX, i64::MIN] {
                let result = template(&[scalar_part(TEMPLATE_INT, value as u64)]);
                assert_eq!(result.as_str(), format_int(value));
                drop_string(result);
            }
            for value in [0.1 + 0.2, -0.0, f64::NAN, f64::INFINITY, 123.0] {
                let result = template(&[scalar_part(TEMPLATE_DOUBLE, value.to_bits())]);
                assert_eq!(result.as_str(), format_double(value));
                drop_string(result);
            }
        });
    }

    #[test]
    fn an_empty_template_allocates_nothing() {
        let allocations = checked(|| {
            let empty = PrismString::empty();
            let result = template(&[string_part(&empty), string_part(&empty)]);
            assert_eq!(result.byte_length, 0);
            assert!(!result.owns_buffer());
            assert_eq!(template(&[]).byte_length, 0);
        });
        assert_eq!(allocations, 0);
    }

    #[test]
    fn printing_a_string_writes_its_text() {
        let _lock = test_lock();
        crate::output::start(Vec::new(), false);
        let text = PrismString::of_static("Grüße, 🚀");
        // SAFETY: the argument is a string.
        unsafe { prism_rt_print_string(&text) };
        assert_eq!(crate::output::take_output(), "Grüße, 🚀".as_bytes());
    }
}
