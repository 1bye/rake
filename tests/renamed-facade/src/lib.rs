//! Covers: renamed facade data, instrumentation, and native/Wasm entry expansion.
//! Real: macros, public facade, GPUI compile boundary.
//! Doubles: none.
//! Requires: a compiler and the selected target libraries; no GPU execution.

use framework::prelude::*;

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
