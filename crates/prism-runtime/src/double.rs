//! The text form of a `Double` (the `execution` spec, "Double Text Form"; D15 of
//! `codegen-core`).
//!
//! A `Double` is written as Kotlin writes it: the shortest decimal that reads back as the same
//! value, with at least one digit after the point, in plain notation when its magnitude is in
//! `[10^-3, 10^7)` and in scientific notation (`1.0E7`, `1.0E-4`) otherwise, and `NaN`,
//! `Infinity`, `-Infinity` and `-0.0` for those values. The shortest digits come from Rust's
//! `{:e}` formatting. Where they are a single digit, Java's `Double.toString` writes the closest
//! decimal of one or two digits instead (`4.9E-324` rather than `5.0E-324`), which only differs
//! for subnormals, whose precision is below two digits; `{:.1e}` gives that decimal.

/// The text form of `value`.
pub fn format_double(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_string();
    }
    // `{:e}` writes the shortest round-trip digits as `d[.ddd]e<exponent>`.
    let (mut digits, mut exponent) = decimal_digits(&format!("{:e}", value.abs()));
    if digits.len() == 1 {
        // The closest decimal of two digits, when it reads back as the same value.
        let closest = format!("{:.1e}", value.abs());
        if closest.parse::<f64>() == Ok(value.abs()) {
            (digits, exponent) = decimal_digits(&closest);
        }
    }
    let mut text = String::new();
    if value < 0.0 {
        text.push('-');
    }
    if (-3..7).contains(&exponent) {
        write_plain(&mut text, &digits, exponent);
    } else {
        // `d.ddd` with at least one digit after the point, then `E<exponent>`.
        text.push_str(&digits[..1]);
        text.push('.');
        text.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        text.push('E');
        text.push_str(&exponent.to_string());
    }
    text
}

/// The significant digits of `scientific`, written as `{:e}` writes it (`d[.ddd]e<exponent>`),
/// without trailing zeros, and its exponent.
fn decimal_digits(scientific: &str) -> (String, i32) {
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("`{:e}` always writes an exponent");
    let exponent: i32 = exponent.parse().expect("`{:e}` writes a decimal exponent");
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    (digits, exponent)
}

/// Writes the value `0.<digits> * 10^(exponent + 1)` in plain notation, with at least one digit
/// on each side of the point.
fn write_plain(text: &mut String, digits: &str, exponent: i32) {
    if exponent < 0 {
        text.push_str("0.");
        for _ in 0..(-exponent - 1) {
            text.push('0');
        }
        text.push_str(digits);
        return;
    }
    let integer_length = exponent as usize + 1;
    if digits.len() <= integer_length {
        text.push_str(digits);
        for _ in digits.len()..integer_length {
            text.push('0');
        }
        text.push_str(".0");
    } else {
        text.push_str(&digits[..integer_length]);
        text.push('.');
        text.push_str(&digits[integer_length..]);
    }
}

#[cfg(test)]
mod tests {
    use super::format_double;

    #[test]
    fn printing_doubles() {
        // The scenario "Printing doubles" of the `execution` spec.
        let cases = [
            (1.0, "1.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (10000000.0, "1.0E7"),
            (0.0001, "1.0E-4"),
            (f64::INFINITY, "Infinity"),
            (f64::NAN, "NaN"),
            (-0.0, "-0.0"),
        ];
        for (value, text) in cases {
            assert_eq!(format_double(value), text, "{value:?}");
        }
    }

    #[test]
    fn plain_and_scientific_thresholds() {
        let cases = [
            (123.456, "123.456"),
            (0.001, "0.001"),
            (1.0e-3, "0.001"),
            (0.000999, "9.99E-4"),
            (9999999.0, "9999999.0"),
            (1.0e7, "1.0E7"),
            (12345678.9, "1.23456789E7"),
            (-2.5, "-2.5"),
            (100.0, "100.0"),
            (0.5, "0.5"),
            (0.0123, "0.0123"),
            (1.0e100, "1.0E100"),
            (-1.5e-10, "-1.5E-10"),
            (f64::NEG_INFINITY, "-Infinity"),
            (0.0, "0.0"),
            (f64::MAX, "1.7976931348623157E308"),
            (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
            // Java writes the closest decimal of one or two digits where the shortest has one
            // digit; that only differs for subnormals, whose precision is below two digits.
            (5e-324, "4.9E-324"),
            (1e-323, "9.9E-324"),
            (1.5e-323, "1.5E-323"),
            (2e-323, "2.0E-323"),
            (1e22, "1.0E22"),
            (1e23, "1.0E23"),
            (0.3, "0.3"),
        ];
        for (value, text) in cases {
            assert_eq!(format_double(value), text, "{value:?}");
        }
    }

    #[test]
    fn every_output_reads_back_to_the_same_bits() {
        let mut values = vec![
            1.0,
            0.1 + 0.2,
            1.0e7,
            1.0e-4,
            -0.0,
            123.456,
            0.001,
            9999999.0,
            f64::MAX,
            f64::MIN_POSITIVE,
            5e-324,
            std::f64::consts::PI,
            -std::f64::consts::E * 1.0e20,
        ];
        // A deterministic spread of bit patterns.
        let mut bits: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..10_000 {
            bits ^= bits << 13;
            bits ^= bits >> 7;
            bits ^= bits << 17;
            let value = f64::from_bits(bits);
            if value.is_finite() {
                values.push(value);
            }
        }
        for value in values {
            let text = format_double(value);
            let read: f64 = text.parse().unwrap_or_else(|_| panic!("{text} parses"));
            assert_eq!(read.to_bits(), value.to_bits(), "{text}");
            assert!(text.contains('.'), "{text} has a point");
        }
    }
}
