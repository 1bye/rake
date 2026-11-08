use super::{Hooks, Kind};

#[test]
#[should_panic(expected = "hook order/type changed at slot 0")]
fn hook_kind_changes_are_diagnosed() {
    let mut hooks = Hooks::default();
    hooks.next(Kind::State, || 0_i32);
    hooks.cursor = 0;
    hooks.mounted = true;
    hooks.next(Kind::Ref, || 0_i32);
}

#[test]
#[should_panic(expected = "hook order/type changed at slot 0")]
fn hook_type_changes_are_diagnosed() {
    let mut hooks = Hooks::default();
    hooks.next(Kind::State, || 0_i32);
    hooks.cursor = 0;
    hooks.mounted = true;
    hooks.next(Kind::State, String::new);
}

#[test]
#[should_panic(expected = "hook count increased at slot 1")]
fn adding_a_conditional_hook_is_diagnosed() {
    let mut hooks = Hooks::default();
    hooks.next(Kind::State, || 0_i32);
    hooks.mounted = true;
    hooks.next(Kind::State, || 0_i32);
}

#[cfg(feature = "devtools")]
#[test]
fn inspection_bounds_strings_and_leaves_opaque_values_unformatted() {
    struct Opaque;
    let mut hooks = Hooks::default();
    hooks.next(Kind::State, || 7_i32);
    hooks.next(Kind::Ref, || "é".repeat(200));
    hooks.next(Kind::Ref, || Opaque);
    let (metadata, truncated) = super::inspection::capture(&hooks);
    assert!(truncated);
    assert_eq!(metadata[0].value, Some(serde_json::json!(7)));
    assert_eq!(
        metadata[1].value.as_ref().unwrap().as_str().unwrap().len(),
        256
    );
    assert_eq!(metadata[2].value, None);
}

#[cfg(feature = "devtools")]
#[test]
fn hook_state_is_attached_to_the_authored_component_boundary() {
    use crate::gpui::{self, AppContext, IntoElement, Render};
    use crate::inspection::Store;
    use crate::router::WindowId;

    #[crate::component]
    fn InspectedCounter(cx: &mut crate::Scope) -> impl IntoElement {
        let (count, _) = cx.use_state(|| 42_i32);
        crate::view! {
            <p>{count.to_string()}</p>
        }
    }

    struct Host(Store);

    impl Render for Host {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            crate::view! {
                <div>
                    <InspectedCounter id="counter" />
                </div>
            }
            .root(self.0.clone(), 0, WindowId(1))
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let store = Store::default();
    store.attach();
    let window = cx.add_window(|_, _| Host(store.clone()));
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let snapshot = store.snapshot();
    let component = snapshot
        .elements
        .iter()
        .find(|element| element.semantic_id.as_deref() == Some("counter"))
        .unwrap();
    let hook = &component.view.as_ref().unwrap().hooks[0];
    assert_eq!(hook.kind, "state");
    assert_eq!(hook.value, Some(serde_json::json!(42)));
}
