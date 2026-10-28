//! Pure parsing for finite lengths, fractions, opacity and literal colors.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Length {
    Pixels(f32),
    Rems(f32),
    Relative(f32),
}

pub(super) fn number(value: &str) -> Result<f32, &'static str> {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());

    let decimal = value.split_once('.').map_or_else(
        || digits(value),
        |(whole, fraction)| digits(whole) && digits(fraction),
    );

    if !decimal {
        return Err("expected a finite number");
    }

    value
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or("expected a finite number")
}

pub(super) fn length(value: &str) -> Result<Length, &'static str> {
    if let Some(value) = value.strip_suffix("px") {
        return number(value).map(Length::Pixels);
    }

    if let Some(value) = value.strip_suffix("rem") {
        return number(value).map(Length::Rems);
    }

    if let Some(value) = value.strip_suffix('%') {
        return number(value).map(|n| Length::Relative(n / 100.));
    }

    Err("arbitrary lengths require a unit; add px, rem or %")
}

pub(super) fn scale(value: &str) -> Result<Length, &'static str> {
    let n = number(value)?;

    if n < 0. || (n * 2.).fract() != 0. {
        return Err("spacing values are whole or half steps (`p-4`, `p-1.5`) or `[value]`");
    }

    Ok(Length::Rems(n / 4.))
}

pub(super) fn fraction(value: &str) -> Result<Length, &'static str> {
    let (numerator, denominator) = value.split_once('/').ok_or("expected a fraction")?;

    let numerator = numerator
        .parse::<u16>()
        .map_err(|_| "invalid fraction numerator")?;

    let denominator = denominator
        .parse::<u16>()
        .map_err(|_| "invalid fraction denominator")?;

    if denominator == 0 {
        return Err("fraction denominator cannot be zero");
    }

    if ![2, 3, 4, 5, 6, 12].contains(&denominator) || numerator > denominator {
        return Err(
            "fractions require a numerator from 0 to the denominator and denominator 2, 3, 4, 5, 6 or 12",
        );
    }

    Ok(Length::Relative(
        f32::from(numerator) / f32::from(denominator),
    ))
}

pub(super) fn color(value: &str) -> Result<(u32, bool), &'static str> {
    let value = value
        .strip_prefix('#')
        .ok_or("colors require #rgb, #rrggbb or #rrggbbaa")?;

    if ![3, 6, 8].contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid hex color; expected #rgb, #rrggbb or #rrggbbaa");
    }

    let expanded = if value.len() == 3 {
        value.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        value.to_owned()
    };

    Ok((
        u32::from_str_radix(&expanded, 16).expect("validated hex"),
        value.len() == 8,
    ))
}

pub(super) fn opacity(value: &str) -> Result<f32, &'static str> {
    let n = if let Some(value) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        number(value)?
    } else {
        let n = value
            .parse::<u16>()
            .map_err(|_| "opacity uses 0–100 in steps of 5 or [0..1]")?;

        if n > 100 || n % 5 != 0 {
            return Err("opacity uses 0–100 in steps of 5 or [0..1]");
        }

        f32::from(n) / 100.
    };

    if !(0.0..=1.0).contains(&n) {
        return Err("opacity must be in the range 0..=1");
    }

    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_require_finite_numbers_units_and_valid_ranges() {
        assert_eq!(length("320px"), Ok(Length::Pixels(320.)));
        assert_eq!(length("0.75rem"), Ok(Length::Rems(0.75)));
        assert_eq!(length("50%"), Ok(Length::Relative(0.5)));
        assert_eq!(scale("13"), Ok(Length::Rems(3.25)));
        assert_eq!(scale("1.5"), Ok(Length::Rems(0.375)));
        assert_eq!(fraction("1/2"), Ok(Length::Relative(0.5)));
        assert_eq!(color("#abc"), Ok((0xaabbcc, false)));
        assert_eq!(color("#12345678"), Ok((0x12345678, true)));
        assert_eq!(opacity("35"), Ok(0.35));
        assert_eq!(opacity("[0.35]"), Ok(0.35));

        for value in [
            "320", "NaNpx", "infrem", "3em", "px", "1e1px", "4.px", ".5rem", "+4%", "-5px",
        ] {
            assert!(length(value).is_err(), "{value}");
        }

        for value in ["-1", "1.25", "NaN", "inf", "1e1", "4.", ".5", "+4"] {
            assert!(scale(value).is_err(), "{value}");
        }

        for value in ["1/0", "1/7", "3/2", "x/2", "1/x"] {
            assert!(fraction(value).is_err(), "{value}");
        }

        for value in ["#12", "#xxxxxx", "#1234567", "red"] {
            assert!(color(value).is_err(), "{value}");
        }

        for value in [
            "101", "33", "[-0.1]", "[1.01]", "[NaN]", "[1e0]", "[1.]", "[.5]", "[+0.5]",
        ] {
            assert!(opacity(value).is_err(), "{value}");
        }
    }
}
