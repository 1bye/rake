//! Covers: renamed facade data, instrumentation, and native/Wasm entry expansion.
//! Real: macros, public facade, GPUI compile boundary.
//! Doubles: none.
//! Requires: a compiler and the selected target libraries; no GPU execution.

use framework::gpui;
use framework::prelude::*;

#[derive(gpui::IntoElement)]
struct ComponentPanel {
    id: gpui::ElementId,
    title: gpui::SharedString,
    header: gpui::AnyElement,
    footer: Option<gpui::AnyElement>,
    children: Vec<gpui::AnyElement>,
}

impl gpui::RenderOnce for ComponentPanel {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl gpui::IntoElement {
        framework::view! {
            <section id={self.id}>
                {self.header}
                <h2>{self.title}</h2>
                <div style={|s| s.children(self.children)}/>
                if let Some(footer) = self.footer {
                    {footer}
                }
            </section>
        }
    }
}

/// Typed components and their children also work through a renamed Rake dependency.
pub fn component_markup() -> impl gpui::IntoElement {
    framework::view! {
        <self::ComponentPanel id={"panel".into()} title={"Components".into()} header={<h2>Markup through a renamed facade</h2>} footer={None}>
            <p>A typed child</p>
            <button on:click={|_, _, _| {}}>"Accept"</button>
        </self::ComponentPanel>
    }
}

/// Paragraph composition and inline run helpers resolve through the renamed facade.
pub fn paragraph_markup(labels: &[&str]) -> impl gpui::IntoElement {
    framework::view! {
        <p wrap={true} max-lines={2} ellipsis={true} style={|s| s.w(gpui::px(240.))}>
            <strong>"Names: "</strong>
            for (index, label) in labels.iter().enumerate() {
                if index > 0 { ", " }
                <span class="italic underline">{label}</span>
            }
            <br/><em>"One composed paragraph."</em>
        </p>
    }
}

/// Overlay components are ordinary typed tags through a renamed facade, too.
pub fn overlay_markup(
    state: framework::overlay::OverlayHostState,
    anchor: framework::overlay::AnchorState,
    popup: framework::overlay::OverlayState,
    dialog: framework::overlay::OverlayState,
) -> impl gpui::IntoElement {
    use framework::overlay::{Dialog, OverlayAnchor, OverlayHost, Popover};
    use gpui::IntoElement;

    let open_popup = popup.clone();

    let body = framework::view! {
        <OverlayAnchor state={anchor.clone()}>
            <button on:click={move |_, window, cx| { open_popup.open(window, cx); }}>"Actions"</button>
        </OverlayAnchor>
    }.into_any_element();

    framework::view! {
        <OverlayHost state={state} body={body}>
            <Popover state={popup} anchor={anchor}>
                <p>"Controls"</p>
            </Popover>
            <Dialog state={dialog} title={"Documents".into()} footer={None}>
                <p>"Retained content can be mounted here."</p>
            </Dialog>
        </OverlayHost>
    }
}

gpui::actions!(
    renamed_view,
    [
        /// Confirms the example action through the renamed facade.
        Accept,
    ]
);

/// Keyboard helpers and namespaced actions resolve through the renamed facade.
pub fn keyboard_markup(focus: &gpui::FocusHandle, disabled: bool) -> impl gpui::IntoElement {
    framework::view! {
        <section focus={&focus} key-context="Renamed" tab-navigation={true}
            role={gpui::accesskit::Role::Group} on:Accept={|_, _, _| {}}>
            <button disabled={disabled} tabindex={0} aria-keyshortcuts="Control+Enter"
                on:click={|_, window, cx| window.dispatch_action(Box::new(Accept), cx)}>
                "Accept"
            </button>
        </section>
    }
}

/// Markup resolves GPUI and style paths through the renamed facade as well.
pub fn markup() -> impl framework::gpui::IntoElement {
    framework::view! {
        <div class="flex flex-col gap-2 font-bold" style={|s| s.p_4()}>
            <h1>"Renamed facade"</h1>
            for index in 0..2usize {
                <p id={("row", index)}>{format!("Row {index}")}</p>
            }
            <button on:click={|_, _, _| {}}>"Save"</button>
        </div>
    }
}

/// Retained views and handles resolve through the renamed facade on both targets.
pub fn retained_markup<V: framework::gpui::Render>(
    editor: &framework::gpui::Entity<V>,
    focus: &framework::gpui::FocusHandle,
    scroll: &framework::gpui::ScrollHandle,
) -> impl framework::gpui::IntoElement {
    framework::view! {
        <div focus={&focus} scroll={&scroll}>
            <view entity={&editor}/>
        </div>
    }
}

/// Generated routing paths also resolve through a renamed facade.
#[derive(Clone, framework::Route)]
pub enum Destination {
    /// Root screen.
    #[route(id = "home", path = "/")]
    Home,
    /// A typed document ID.
    #[route(id = "document", path = "/documents/{id}")]
    Document(u64),
}

/// A consumer intentionally has no direct Serde or tracing dependency.
#[framework::data]
pub struct Document {
    /// Stable test identifier.
    pub id: u64,
}

/// Exercises explicit display fields and asynchronous instrumentation.
#[framework::instrument(skip_all, fields(id = %id))]
pub async fn document(id: u64) -> Document {
    framework::info!(id, "Document created");

    Document { id }
}

/// Both schemas and command paths resolve through the renamed facade.
#[framework::data(schema)]
#[derive(framework::Command)]
#[command(id = "document.describe", output = Description, capability = Headless)]
#[serde(deny_unknown_fields)]
pub struct Describe {
    /// Document key accepted by the runtime.
    pub document: String,
}

/// A generated result schema needs no direct Serde or Schemars dependency.
#[framework::data(schema)]
pub struct Description {
    /// Echoed document key.
    pub document: String,
}

#[framework::app]
fn app() -> App {
    App::new("com.example.renamed")
        .root(|_, _, cx| framework::gpui::AppContext::new(cx, |_| framework::gpui::Empty).into())
}

impl Default for ComponentPanel {
    fn default() -> Self {
        Self {
            id: "defaults".into(),
            title: "Panel".into(),
            header: gpui::IntoElement::into_any_element(gpui::div()),
            footer: None,
            children: Vec::new(),
        }
    }
}

/// Shorthand, defaults, booleans and match arms also resolve through a renamed facade.
pub fn syntax_markup(id: gpui::ElementId, selected: Option<String>) -> impl gpui::IntoElement {
    framework::view! {
        <self::ComponentPanel {id} {..Default::default()}>
            <main>
                <h2 ellipsis>
                    match selected {
                        Some(label) if !label.is_empty() => { <strong>{label}</strong> }
                        _ => { "Nothing selected" }
                    }
                </h2>
            </main>
        </self::ComponentPanel>
    }
}

#[cfg(test)]
mod tests {
    use super::Document;

    #[test]
    fn commands_use_the_renamed_data_schema_and_runtime_facades() {
        use framework::control::ApplicationCommand;
        let descriptor = super::Describe::descriptor();

        assert_eq!(descriptor.id, "document.describe");
        assert_eq!(
            descriptor.arguments["properties"]["document"]["type"],
            "string"
        );
        assert_eq!(
            descriptor.result["properties"]["document"]["type"],
            "string"
        );
        let router = framework::router::Router::<super::Destination>::new(Default::default());

        router
            .client()
            .commands()
            .register::<super::Describe, _, _>(|args, _| async move {
                Ok(super::Description {
                    document: args.document,
                })
            })
            .unwrap();
        assert!(matches!(
            router
                .navigator()
                .dispatch(
                    super::Describe {
                        document: "A".into()
                    },
                    "from-gui".into()
                )
                .unwrap(),
            framework::control::Reply::Submitted { .. }
        ));
    }

    #[test]
    fn data_uses_the_renamed_serde_facade() {
        let document = Document { id: 42 };
        let encoded = framework::serde_json::to_string(&document).expect("encode");
        let decoded: Document = framework::serde_json::from_str(&encoded).expect("decode");

        assert_eq!(decoded.id, 42);
    }

    #[test]
    fn route_uses_the_renamed_facade() {
        use framework::router::Route;

        let route = super::Destination::from_uri("/documents/42").expect("decode");

        assert_eq!(route.to_uri().expect("encode"), "/documents/42");
        assert_eq!(route.encode().route, "document");
    }

    #[test]
    fn local_work_uses_only_the_renamed_facade() {
        use framework::prelude::*;
        let router = Router::<super::Destination>::new(RouterConfig::default());
        let state = std::rc::Rc::new(std::cell::Cell::new(0));

        let task = router
            .app()
            .tasks()
            .spawn("export", async move {
                state.set(42);

                Ok(())
            })
            .expect("submit");

        assert_eq!(
            task.snapshot().expect("state").state,
            framework::tasks::TaskState::Queued
        );
    }
}
