//! GPUI application locales and embedded Fluent messages for the i18n prototype.

mod text;

#[cfg(feature = "locale-icu")]
mod formatting;
#[cfg(feature = "locale-icu")]
pub use formatting::{Decimal, PluralCategory, format_date, format_number, plural_category};

#[cfg(not(feature = "locale-icu"))]
use fluent_bundle::FluentValue;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use fluent_bundle::{FluentBundle, FluentResource};
use icu_locale_core::LanguageIdentifier;
pub use icu_locale_core::Locale;

use crate::gpui::{App, Global};

#[cfg(feature = "devtools")]
pub(crate) use text::LocalizedElement;
pub use text::LocalizedText;

#[doc(hidden)]
pub use fluent_bundle::FluentArgs;

/// How the application chooses its locale.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LocaleMode {
    /// Negotiate the host's language preferences.
    #[default]
    System,
    /// Negotiate a single explicitly requested locale.
    Explicit(Locale),
}

/// Locale registration, negotiation or formatting failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A catalogue tag was malformed, duplicated or unsupported by Fluent.
    #[error("invalid or duplicate locale {0}")]
    InvalidLocale(String),
    /// The app's required fallback catalogue was absent.
    #[error("default locale {0} is not registered")]
    MissingDefault(String),
    /// Fluent rejected the resource or found a conflicting identifier.
    #[error("invalid Fluent catalogue for {0}: {1}")]
    InvalidCatalogue(String, String),
    /// An explicit selection had no registered match.
    #[error("no registered locale matches {0}")]
    NoMatch(String),
    /// The application has yet to install its catalogues.
    #[error("no application locale is installed")]
    NotInstalled,
    /// ICU4X could not construct or use the requested formatter.
    #[cfg(feature = "locale-icu")]
    #[error("locale formatting failed: {0}")]
    Formatting(String),
}

struct Catalogue {
    locale: Locale,
    bundle: FluentBundle<FluentResource>,
    invalid_plural: Arc<AtomicBool>,
    #[cfg(feature = "locale-icu")]
    formatting: Option<formatting::Formatting>,
}

/// Parsed, owned Fluent resources with an app-supplied default locale.
pub struct Catalogues {
    entries: Vec<Catalogue>,
    default: usize,
}

impl Catalogues {
    /// Registers embedded FTL resources in preference order for same-language ties.
    ///
    /// # Errors
    ///
    /// Rejects malformed or duplicate tags, Fluent syntax or identifier errors,
    /// and a missing default catalogue.
    pub fn embedded(default: &str, sources: &[(&str, &str)]) -> Result<Self, Error> {
        let default_locale = default
            .parse::<Locale>()
            .map_err(|_| Error::InvalidLocale(default.into()))?;
        let mut entries = Vec::<Catalogue>::new();

        for &(tag, ftl) in sources {
            let invalid_locale = || Error::InvalidLocale(tag.into());
            let locale: Locale = tag.parse().map_err(|_| invalid_locale())?;

            if !locale.extensions.is_empty() || entries.iter().any(|entry| entry.locale == locale) {
                return Err(invalid_locale());
            }

            let resource = FluentResource::try_new(ftl.to_owned()).map_err(|(_, errors)| {
                Error::InvalidCatalogue(tag.into(), format!("{errors:?}"))
            })?;
            let language = locale
                .id
                .to_string()
                .parse()
                .map_err(|_| invalid_locale())?;
            let mut bundle = FluentBundle::new(vec![language]);
            let invalid_catalogue =
                |errors: Vec<_>| Error::InvalidCatalogue(tag.into(), format!("{errors:?}"));

            bundle.add_resource(resource).map_err(invalid_catalogue)?;
            let invalid_plural = Arc::new(AtomicBool::new(false));
            #[cfg(not(feature = "locale-icu"))]
            {
                let failed = Arc::clone(&invalid_plural);

                // The Fluent-only build uses Fluent's resolver for the size comparison.
                bundle
                    .add_function("PLURAL", move |args, named| {
                        if let [FluentValue::Number(number)] = args
                            && number.value.is_finite()
                            && named.iter().next().is_none()
                        {
                            return FluentValue::Number(number.clone());
                        }

                        failed.store(true, Ordering::Relaxed);
                        FluentValue::Error
                    })
                    .map_err(|error| invalid_catalogue(vec![error]))?;
            }

            entries.push(Catalogue {
                locale,
                bundle,
                invalid_plural,
                #[cfg(feature = "locale-icu")]
                formatting: None,
            });
        }

        let default = entries
            .iter()
            .position(|entry| entry.locale == default_locale)
            .ok_or_else(|| Error::MissingDefault(default.into()))?;

        Ok(Self { entries, default })
    }

    fn exact(&self, id: &LanguageIdentifier) -> Option<usize> {
        self.entries.iter().position(|entry| &entry.locale.id == id)
    }

    fn parent(&self, id: &LanguageIdentifier) -> Option<usize> {
        let mut parent = id.clone();

        if !parent.variants.is_empty() {
            parent.variants = Default::default();

            if let Some(index) = self.exact(&parent) {
                return Some(index);
            }
        }

        if parent.region.take().is_some()
            && let Some(index) = self.exact(&parent)
        {
            return Some(index);
        }

        parent.script.take().and_then(|_| self.exact(&parent))
    }

    fn negotiate(&self, requested: &Locale) -> Option<usize> {
        self.exact(&requested.id)
            .or_else(|| self.parent(&requested.id))
            .or_else(|| {
                self.entries.iter().position(|entry| {
                    entry.locale.id.language == requested.id.language
                        && (requested.id.script.is_none()
                            || entry.locale.id.script == requested.id.script)
                })
            })
    }

    fn resolve(
        &self,
        mode: &LocaleMode,
        system: impl IntoIterator<Item = String>,
    ) -> Result<usize, Error> {
        match mode {
            LocaleMode::System => Ok(system
                .into_iter()
                .filter_map(|tag| tag.parse().ok())
                .find_map(|requested| self.negotiate(&requested))
                .unwrap_or(self.default)),
            LocaleMode::Explicit(requested) => {
                let selected = self
                    .negotiate(requested)
                    .ok_or_else(|| Error::NoMatch(requested.to_string()))?;

                if &self.entries[selected].locale != requested {
                    tracing::warn!(%requested, resolved = %self.entries[selected].locale, "locale override fell back");
                }

                Ok(selected)
            }
        }
    }

    fn translate(&self, selected: usize, key: &str, args: &FluentArgs) -> Option<String> {
        let parent = self.parent(&self.entries[selected].locale.id);

        for index in [Some(selected), parent, Some(self.default)]
            .into_iter()
            .flatten()
        {
            let catalogue = &self.entries[index];
            let Some(pattern) = catalogue
                .bundle
                .get_message(key)
                .and_then(|message| message.value())
            else {
                continue;
            };

            let mut errors = Vec::new();
            let text = catalogue
                .bundle
                .format_pattern(pattern, Some(args), &mut errors);
            let invalid_plural = catalogue.invalid_plural.swap(false, Ordering::Relaxed);

            // Resolver errors can contain argument values. Log only the key and count.
            if !errors.is_empty() || invalid_plural || text.trim().is_empty() {
                tracing::warn!(
                    key,
                    errors = errors.len(),
                    invalid_plural,
                    "Fluent message could not be formatted"
                );
                return None;
            }

            return Some(text.into_owned());
        }

        tracing::warn!(key, "Fluent message is missing");
        None
    }
}

/// Observable global installed and updated by Rake's locale setters.
pub struct LocaleGlobal {
    catalogues: Catalogues,
    selected: usize,
    mode: LocaleMode,
}

impl Global for LocaleGlobal {}

/// Installs catalogues and resolves the startup preference before the first render.
///
/// # Errors
///
/// An explicit preference with no match leaves any existing locale intact.
pub fn install(cx: &mut App, catalogues: Catalogues, mode: LocaleMode) -> Result<(), Error> {
    let selected = resolve(&catalogues, &mode)?;

    sync_kit(&catalogues.entries[selected].locale);
    cx.set_global(LocaleGlobal {
        catalogues,
        selected,
        mode,
    });
    cx.refresh_windows();

    Ok(())
}

/// Changes the locale preference and refreshes all existing GPUI windows.
///
/// # Errors
///
/// An uninstalled locale or an explicit preference with no match leaves the selection intact.
pub fn set_mode(cx: &mut App, mode: LocaleMode) -> Result<(), Error> {
    let global = global(cx)?;
    let selected = resolve(&global.catalogues, &mode)?;

    if selected != global.selected || mode != global.mode {
        let global = cx.global_mut::<LocaleGlobal>();

        global.selected = selected;
        global.mode = mode;
        sync_kit(&global.catalogues.entries[selected].locale);
        cx.refresh_windows();
    }

    Ok(())
}

/// Reads host preferences again in System mode.
///
/// # Errors
///
/// Returns an error if the application has yet to install its catalogues.
pub fn redetect(cx: &mut App) -> Result<(), Error> {
    if global(cx)?.mode == LocaleMode::System {
        set_mode(cx, LocaleMode::System)?;
    }

    Ok(())
}

/// Returns the resolved registered locale, if installed.
pub fn current(cx: &App) -> Option<&Locale> {
    selected(cx).ok().map(|entry| &entry.locale)
}

fn global(cx: &App) -> Result<&LocaleGlobal, Error> {
    cx.try_global::<LocaleGlobal>().ok_or(Error::NotInstalled)
}

fn selected(cx: &App) -> Result<&Catalogue, Error> {
    let global = global(cx)?;

    Ok(&global.catalogues.entries[global.selected])
}

fn resolve(catalogues: &Catalogues, mode: &LocaleMode) -> Result<usize, Error> {
    let preferences = match mode {
        LocaleMode::System => system_locales(),
        LocaleMode::Explicit(_) => Vec::new(),
    };

    catalogues.resolve(mode, preferences)
}

#[cfg(not(target_family = "wasm"))]
fn system_locales() -> Vec<String> {
    sys_locale::get_locales().collect()
}

#[cfg(target_family = "wasm")]
fn system_locales() -> Vec<String> {
    let Some(window) = web_sys::window() else {
        return Vec::new();
    };

    window
        .navigator()
        .languages()
        .iter()
        .filter_map(|language| language.as_string())
        .collect()
}

/// Picks Kit's catalogue: exact tag, bare language, then a regional sibling such as
/// `pt-BR` for `pt-PT`, and English only when Kit has no catalogue in that language.
#[cfg(any(all(feature = "kit", not(target_family = "wasm")), test))]
fn kit_locale(locale: &Locale) -> &'static str {
    // shortcut: Kit 0.9's supported tags are pinned here until Kit exposes the catalogue list.
    const SUPPORTED: &[&str] = &["en", "pt-BR", "ru", "zh-CN", "zh-HK", "zh-TW", "it"];
    let exact = locale.id.to_string();
    let language = locale.id.language.as_str();

    let find = |matches: &dyn Fn(&str) -> bool| SUPPORTED.iter().copied().find(|tag| matches(tag));

    find(&|tag| tag == exact)
        .or_else(|| find(&|tag| tag == language))
        .or_else(|| find(&|tag| tag.split('-').next() == Some(language)))
        .unwrap_or("en")
}

#[cfg(all(feature = "kit", not(target_family = "wasm")))]
fn sync_kit(locale: &Locale) {
    crate::kit::component::set_locale(kit_locale(locale));
}

#[cfg(not(all(feature = "kit", not(target_family = "wasm"))))]
fn sync_kit(_: &Locale) {}

#[doc(hidden)]
#[track_caller]
pub fn translate(cx: &App, key: &'static str, args: &FluentArgs) -> LocalizedText {
    let translated = match global(cx) {
        Ok(global) => global.catalogues.translate(global.selected, key, args),
        Err(_) => {
            tracing::warn!(key, "translation requested before installing a locale");
            None
        }
    };

    LocalizedText::new(key, translated)
}

/// Formats a Fluent message using the application's resolved locale.
///
/// The first argument is the GPUI context. Missing messages and resolver
/// errors emit a diagnostic and display the key.
#[macro_export]
macro_rules! t {
    ($cx:expr, $key:literal $(,)?) => {
        $crate::locale::translate($cx, $key, &$crate::locale::FluentArgs::new())
    };
    ($cx:expr, $key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::locale::FluentArgs::new();
        $(args.set(stringify!($name), $value);)+
        $crate::locale::translate($cx, $key, &args)
    }};
}

#[cfg(test)]
mod tests;
