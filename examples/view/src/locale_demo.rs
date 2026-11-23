use rake::gpui::{self, AnyView, Context, Window, prelude::*};
use rake::locale::{self, Catalogues, LocaleMode};
use rake::router::{MountError, Navigator, OpenTarget, PageContext, RouteViews};

#[cfg(all(feature = "kit", not(target_family = "wasm")))]
use rake::kit::component::select::{Select, SelectState};

use super::Colors;

#[derive(Clone, rake::Route)]
pub(super) enum Page {
    #[route(id = "inbox", path = "/")]
    Inbox,
    #[route(id = "counter", path = "/counter")]
    Counter,
}

struct Pages;

impl RouteViews<Page> for Pages {
    fn mount(
        &self,
        page: &Page,
        context: PageContext<Page>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
        if locale::current(cx).is_none() {
            install(cx).map_err(|error| MountError(error.to_string()))?;
        }

        match page {
            Page::Inbox => {
                window.set_window_title("Rake i18n");
                Ok(cx
                    .new(|cx| Inbox::new(context.navigator(), window, cx))
                    .into())
            }
            Page::Counter => Ok(super::mount_counter(window, cx)),
        }
    }
}

fn install(cx: &mut gpui::App) -> Result<(), locale::Error> {
    let catalogues = Catalogues::embedded(
        "en",
        &[
            ("en", include_str!("../locales/en.ftl")),
            ("pt-PT", include_str!("../locales/pt-PT.ftl")),
        ],
    )?;
    #[cfg(feature = "i18n-icu")]
    let catalogues = catalogues.with_icu_data(include_bytes!("../locales/icu.postcard"))?;

    #[cfg(not(target_family = "wasm"))]
    let mode = match std::env::var("RAKE_VIEW_LOCALE") {
        Ok(tag) => explicit(&tag)?,
        Err(_) => LocaleMode::System,
    };
    #[cfg(target_family = "wasm")]
    let mode = LocaleMode::System;

    locale::install(cx, catalogues, mode)
}

fn explicit(tag: &str) -> Result<LocaleMode, locale::Error> {
    tag.parse()
        .map(LocaleMode::Explicit)
        .map_err(|_| locale::Error::InvalidLocale(tag.into()))
}

fn pick(cx: &mut gpui::App, tag: Option<&str>) {
    let mode = tag.map_or(Ok(LocaleMode::System), explicit);

    if let Err(error) = mode.and_then(|mode| locale::set_mode(cx, mode)) {
        rake::warn!(%error, "could not set example locale");
    }
}

struct Inbox {
    count: u32,
    navigator: Navigator<Page>,
    #[cfg(all(feature = "kit", not(target_family = "wasm")))]
    kit_default: gpui::Entity<SelectState<Vec<&'static str>>>,
    #[cfg(all(feature = "kit", not(target_family = "wasm")))]
    kit_localized: gpui::Entity<SelectState<Vec<&'static str>>>,
}

impl Inbox {
    fn new(navigator: Navigator<Page>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            count: 0,
            navigator,
            #[cfg(all(feature = "kit", not(target_family = "wasm")))]
            kit_default: _cx.new(|cx| SelectState::new(vec!["Inbox"], None, _window, cx)),
            #[cfg(all(feature = "kit", not(target_family = "wasm")))]
            kit_localized: _cx.new(|cx| SelectState::new(vec!["Inbox"], None, _window, cx)),
        }
    }
}

impl Render for Inbox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counter = self.navigator.clone();
        let open = self.navigator.clone();
        let language = locale::current(cx)
            .map(ToString::to_string)
            .unwrap_or_default();
        let title = rake::t!(cx, "inbox-title");
        let add = rake::t!(cx, "inbox-add");
        let system = rake::t!(cx, "inbox-system");
        let english = rake::t!(cx, "inbox-english");
        let portuguese = rake::t!(cx, "inbox-portuguese");
        let new_window = rake::t!(cx, "inbox-new-window");
        let counter_label = rake::t!(cx, "inbox-counter");

        #[cfg(feature = "i18n-icu")]
        let formats = {
            let decimal = "1234.50".parse().expect("example decimal literal is valid");
            let amount =
                locale::format_number(cx, &decimal).unwrap_or_else(|error| error.to_string());
            let date =
                locale::format_date(cx, 2026, 11, 21).unwrap_or_else(|error| error.to_string());

            rake::view! {
                <div class="flex flex-col gap-1">
                    {rake::t!(cx, "inbox-decimal", amount = amount)}
                    {rake::t!(cx, "inbox-date", date = date)}
                </div>
            }
            .into_any_element()
        };
        #[cfg(not(feature = "i18n-icu"))]
        let formats = gpui::Empty.into_any_element();

        #[cfg(all(feature = "kit", not(target_family = "wasm")))]
        let kit = {
            let fallback_name = rake::t!(cx, "inbox-kit");
            let localized_name = rake::t!(cx, "inbox-kit-localized");
            let placeholder = rake::t!(cx, "inbox-kit-placeholder");
            let default_select =
                Select::new(&self.kit_default).accessibility_label(fallback_name.clone());
            let localized = Select::new(&self.kit_localized)
                .accessibility_label(localized_name.clone())
                .placeholder(placeholder);

            rake::view! {
                <div class="flex flex-col gap-2">
                    <div>{fallback_name}{default_select}</div>
                    <div>{localized_name}{localized}</div>
                </div>
            }
            .into_any_element()
        };
        #[cfg(not(all(feature = "kit", not(target_family = "wasm"))))]
        let kit = gpui::Empty.into_any_element();

        rake::view! {
            <section class="size-full p-6 gap-3 bg-surface text-ink" tab-navigation={true}>
                <h1 message-key="inbox-title">{title}</h1>
                {rake::t!(cx, "inbox-language", locale = language)}
                {rake::t!(cx, "inbox-count", count = self.count)}
                {formats}
                <div class="flex flex-row flex-wrap gap-2">
                    <button
                        id="inbox-system"
                        message-key="inbox-system"
                        aria-label={system.clone()}
                        class="px-3 py-2 rounded-md border border-line focus-visible:bg-accent"
                        on:click={|_, _, cx| pick(cx, None)}
                    >
                        {system}
                    </button>
                    <button
                        id="inbox-english"
                        message-key="inbox-english"
                        aria-label={english.clone()}
                        class="px-3 py-2 rounded-md border border-line focus-visible:bg-accent"
                        on:click={|_, _, cx| pick(cx, Some("en"))}
                    >
                        {english}
                    </button>
                    <button
                        id="inbox-portuguese"
                        message-key="inbox-portuguese"
                        aria-label={portuguese.clone()}
                        class="px-3 py-2 rounded-md border border-line focus-visible:bg-accent"
                        on:click={|_, _, cx| pick(cx, Some("pt-PT"))}
                    >
                        {portuguese}
                    </button>
                </div>
                <div class="flex flex-row flex-wrap gap-2">
                    <button
                        id="inbox-add"
                        message-key="inbox-add"
                        aria-label={add.clone()}
                        class="px-3 py-2 rounded-md bg-accent focus-visible:bg-accent-focus"
                        on:click={
                            cx.listener(|this, _, _, cx| {
                                this.count = this.count.saturating_add(1);
                                cx.notify();
                            })
                        }
                    >
                        {add}
                    </button>
                    if cfg!(not(target_family = "wasm")) {
                        <button
                            id="inbox-new-window"
                            message-key="inbox-new-window"
                            aria-label={new_window.clone()}
                            class="px-3 py-2 rounded-md border border-line focus-visible:bg-accent"
                            on:click={
                                move |_, _, _| drop(open.open(Page::Inbox, OpenTarget::NewWindow))
                            }
                        >
                            {new_window}
                        </button>
                    }
                    <button
                        id="inbox-counter"
                        message-key="inbox-counter"
                        aria-label={counter_label.clone()}
                        class="px-3 py-2 rounded-md border border-line focus-visible:bg-accent"
                        on:click={
                            move |_, _, _| drop(counter.open(Page::Counter, OpenTarget::NewTab))
                        }
                    >
                        {counter_label}
                    </button>
                </div>
                {kit}
            </section>
        }
    }
}

pub(super) fn application() -> rake::App<Page> {
    rake::App::new("com.example.rake.view")
        .control(rake::control::ControlConfig::FromEnvironment)
        .theme(Colors::dark())
        .theme(rake::theme::Semantic::dark())
        .routes(Pages)
        .initial(Page::Inbox)
}

#[cfg(all(test, feature = "i18n-icu", feature = "view-test-support"))]
mod tests {
    use super::*;

    #[gpui::test]
    fn formats_and_switches_using_the_public_api(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            install(cx).unwrap();
            let amount = "1234.50".parse().unwrap();

            locale::set_mode(cx, explicit("en-GB").unwrap()).unwrap();
            assert_eq!(locale::current(cx).unwrap().to_string(), "en");
            assert_eq!(locale::format_number(cx, &amount).unwrap(), "1,234.50");
            assert_eq!(
                locale::format_date(cx, 2026, 11, 21).unwrap(),
                "Nov 21, 2026"
            );

            locale::set_mode(cx, explicit("pt-BR").unwrap()).unwrap();
            assert_eq!(locale::current(cx).unwrap().to_string(), "pt-PT");
            assert_eq!(locale::format_number(cx, &amount).unwrap(), "1234,50");
            assert_eq!(locale::format_date(cx, 2026, 11, 21).unwrap(), "21/11/2026");
            assert!(locale::format_date(cx, 2026, 2, 29).is_err());

            for (count, category, text) in [
                (1, locale::PluralCategory::One, "Uma mensagem"),
                (2, locale::PluralCategory::Other, "mensagens"),
            ] {
                assert_eq!(
                    locale::plural_category(cx, &count.into()).unwrap(),
                    category
                );
                assert!(
                    rake::t!(cx, "inbox-count", count = count)
                        .as_str()
                        .contains(text)
                );
            }

            assert_eq!(
                rake::t!(cx, "inbox-count", count = "uma").as_str(),
                "inbox-count"
            );
            assert_eq!(rake::t!(cx, "inbox-count").as_str(), "inbox-count");
            assert!(locale::set_mode(cx, explicit("ja").unwrap()).is_err());
            assert!(explicit("pt-@").is_err());
            locale::redetect(cx).unwrap();
            assert_eq!(locale::current(cx).unwrap().to_string(), "pt-PT");
        });
    }
}
