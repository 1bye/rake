//! Covers: locale negotiation, resource validation, and Fluent fallback and arguments.
//! Real: ICU locale parser and Fluent resolver. Doubles: supplied host preferences.

use super::*;

fn catalogues() -> Catalogues {
    Catalogues::embedded(
        "en",
        &[
            (
                "en",
                "title = Inbox\ndefault-only = English\nwith-argument = Hi, { $name }",
            ),
            ("pt", "title = Caixa de correio\nparent-only = Português"),
            (
                "pt-PT",
                "title = Caixa de entrada\nwith-argument = Olá, { $name }",
            ),
        ],
    )
    .unwrap()
}

#[test]
fn negotiates_in_host_order_and_warns_on_explicit_fallback() {
    let catalogues = catalogues();
    let system = |tags: &[&str]| {
        catalogues
            .resolve(&LocaleMode::System, tags.iter().map(|tag| (*tag).into()))
            .unwrap()
    };

    assert_eq!(system(&["invalid_tag!", "fr", "PT-pt", "en"]), 2);
    assert_eq!(system(&["pt-BR", "en"]), 1);
    assert_eq!(system(&["fr", "ja"]), 0);
    assert!("pt-@".parse::<Locale>().is_err());
    assert_eq!(
        catalogues
            .resolve(&LocaleMode::Explicit("en-GB".parse().unwrap()), [])
            .unwrap(),
        0
    );
    assert!(matches!(
        catalogues.resolve(&LocaleMode::Explicit("ja".parse().unwrap()), []),
        Err(Error::NoMatch(_))
    ));

    let regional =
        Catalogues::embedded("en", &[("en", "title = Inbox"), ("pt-PT", "title = Caixa")]).unwrap();
    assert_eq!(regional.negotiate(&"pt-BR".parse().unwrap()), Some(1));
}

#[test]
fn rejects_invalid_registrations() {
    assert!(matches!(
        Catalogues::embedded("en", &[("en", "broken = {")]),
        Err(Error::InvalidCatalogue(..))
    ));
    assert!(matches!(
        Catalogues::embedded("en", &[("en", "key = A\nkey = B")]),
        Err(Error::InvalidCatalogue(..))
    ));
    assert!(matches!(
        Catalogues::embedded("en", &[("pt", "key = A")]),
        Err(Error::MissingDefault(_))
    ));
    assert!(matches!(
        Catalogues::embedded("en", &[("pt", "key = A"), ("PT", "key = B")]),
        Err(Error::InvalidLocale(_))
    ));
}

#[test]
fn falls_back_for_missing_messages_and_displays_keys_on_errors() {
    let catalogues = catalogues();
    let args = FluentArgs::new();
    let render = |key, args| LocalizedText::new(key, catalogues.translate(2, key, args));

    assert_eq!(render("title", &args).as_str(), "Caixa de entrada");
    assert_eq!(render("parent-only", &args).as_str(), "Português");
    assert_eq!(render("default-only", &args).as_str(), "English");
    assert_eq!(render("absent", &args).as_str(), "absent");
    assert_eq!(render("with-argument", &args).as_str(), "with-argument");

    let mut args = FluentArgs::new();
    args.set("name", "Inês");
    let translated = render("with-argument", &args);
    assert!(translated.as_str().contains("Inês"));
    assert_eq!(translated.message_key(), "with-argument");
}
