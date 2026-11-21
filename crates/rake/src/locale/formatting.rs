use std::sync::{Arc, atomic::Ordering};

pub use fixed_decimal::Decimal;
use fluent_bundle::FluentValue;
use icu_calendar::{Date, Gregorian};
use icu_datetime::{FixedCalendarDateTimeFormatter, fieldsets::YMD};
use icu_decimal::DecimalFormatter;
pub use icu_plurals::PluralCategory;
use icu_plurals::PluralRules;
use icu_provider_blob::BlobDataProvider;
use writeable::Writeable as _;

use super::{App, Catalogues, Error, selected};

pub(super) struct Formatting {
    decimal: DecimalFormatter,
    date: FixedCalendarDateTimeFormatter<Gregorian, YMD>,
    plural: Arc<PluralRules>,
}

impl Catalogues {
    /// Installs decimal, Gregorian date and cardinal plural data for each catalogue.
    ///
    /// Use PLURAL($count) as a Fluent selector to use the same ICU4X cardinal rules.
    ///
    /// # Errors
    ///
    /// Returns an error if the blob is invalid or lacks data for a registered locale.
    pub fn with_icu_data(mut self, blob: &'static [u8]) -> Result<Self, Error> {
        let provider = BlobDataProvider::try_new_from_static_blob(blob).map_err(format_error)?;

        for catalogue in &mut self.entries {
            let locale = &catalogue.locale;
            let decimal = DecimalFormatter::try_new_with_buffer_provider(
                &provider,
                locale.into(),
                Default::default(),
            )
            .map_err(format_error)?;
            let date = FixedCalendarDateTimeFormatter::try_new_with_buffer_provider(
                &provider,
                locale.into(),
                YMD::medium(),
            )
            .map_err(format_error)?;
            let plural = Arc::new(
                PluralRules::try_new_cardinal_with_buffer_provider(&provider, locale.into())
                    .map_err(format_error)?,
            );
            let failed = Arc::clone(&catalogue.invalid_plural);
            let rules = Arc::clone(&plural);

            // Fluent's function interface cannot add a resolver error for a failed selector.
            catalogue
                .bundle
                .add_function("PLURAL", move |args, named| {
                    if let [FluentValue::Number(number)] = args
                        && named.iter().next().is_none()
                        && let Ok(decimal) = number.as_string().parse::<Decimal>()
                    {
                        let category = match rules.category_for(&decimal) {
                            PluralCategory::Zero => "zero",
                            PluralCategory::One => "one",
                            PluralCategory::Two => "two",
                            PluralCategory::Few => "few",
                            PluralCategory::Many => "many",
                            PluralCategory::Other => "other",
                        };

                        return FluentValue::from(category);
                    }

                    failed.store(true, Ordering::Relaxed);
                    FluentValue::Error
                })
                .map_err(format_error)?;

            catalogue.formatting = Some(Formatting {
                decimal,
                date,
                plural,
            });
        }

        Ok(self)
    }
}

/// Formats a decimal with the installed locale's default grouping and symbols.
///
/// # Errors
///
/// Returns an error if the locale or ICU data has yet to be installed.
pub fn format_number(cx: &App, value: &Decimal) -> Result<String, Error> {
    Ok(formatting(cx)?
        .decimal
        .format(value)
        .write_to_string()
        .into_owned())
}

/// Formats a Gregorian civil date using the locale's medium date pattern.
///
/// # Errors
///
/// Returns an error for an invalid date or missing locale or ICU data.
pub fn format_date(cx: &App, year: i32, month: u8, day: u8) -> Result<String, Error> {
    let date = Date::try_new_gregorian(year, month, day).map_err(format_error)?;

    Ok(formatting(cx)?
        .date
        .format(&date)
        .write_to_string()
        .into_owned())
}

/// Selects a CLDR cardinal category, preserving the decimal's visible precision.
///
/// # Errors
///
/// Returns an error if the locale or ICU data has yet to be installed.
pub fn plural_category(cx: &App, value: &Decimal) -> Result<PluralCategory, Error> {
    Ok(formatting(cx)?.plural.category_for(value))
}

fn formatting(cx: &App) -> Result<&Formatting, Error> {
    selected(cx)?
        .formatting
        .as_ref()
        .ok_or_else(|| Error::Formatting("ICU data is not installed".into()))
}

fn format_error(error: impl std::fmt::Display) -> Error {
    Error::Formatting(error.to_string())
}
