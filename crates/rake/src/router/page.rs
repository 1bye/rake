//! Route entry points and exhaustive, statically dispatched page registration.
use super::{MountError, PageContext, Route, RouteViews};
use gpui::{AnyView, App, Entity, Render, Window};

/// Constructs a route entry's retained GPUI entity on the UI thread.
///
/// Implement this alongside [`Render`] for route entry points. Ordinary reusable
/// components only need `Render`. The destination carries typed route parameters;
/// the context identifies this specific history entry and its services.
/// Construction runs once per new or restored entry, not on history traversal
/// or rendering. The existing router owns disposal, guards and scoped work.
pub trait Page<R: Route>: Render + Sized {
    /// Creates the entity for this visit, without retaining the router itself.
    ///
    /// # Errors
    /// Return service, parameter or initialization failures as [`MountError`].
    /// Failed mounting leaves the current history and screen intact.
    fn mount(
        destination: &R,
        page: PageContext<R>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>, MountError>;
}

struct RegisteredPages<F>(F);

impl<R, F> RouteViews<R> for RegisteredPages<F>
where
    R: Route,
    F: Fn(&R, PageContext<R>, &mut Window, &mut App) -> Result<AnyView, MountError> + 'static,
{
    fn mount(
        &self,
        destination: &R,
        page: PageContext<R>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<AnyView, MountError> {
        self.0(destination, page, window, cx)
    }
}

#[doc(hidden)]
pub fn __routes<R: Route>(
    mount: impl Fn(&R, PageContext<R>, &mut Window, &mut App) -> Result<AnyView, MountError> + 'static,
) -> impl RouteViews<R> {
    RegisteredPages(mount)
}

#[doc(hidden)]
pub fn __mount_page<R: Route, P: Page<R>>(
    destination: &R,
    page: PageContext<R>,
    window: &mut Window,
    cx: &mut App,
) -> Result<AnyView, MountError> {
    P::mount(destination, page, window, cx).map(Into::into)
}

/// Maps exhaustive destination patterns to types implementing [`router::Page`](crate::router::Page).
///
/// Each arm calls the selected page's constructor through static dispatch. This
/// composes with [`App::routes`](crate::App::routes) and custom `RouteViews` hosts;
/// it does not add another routing runtime or change entry retention. Use `..`
/// for parameters; the page receives the complete typed destination in `mount`.
///
/// ```
/// use rake::gpui::{self, AppContext, Context, IntoElement, Render};
/// use rake::prelude::*;
/// use rake::router::MountError;
/// #[derive(Clone, rake::Route)]
/// enum Destination {
///     #[route(id = "home", path = "/")]
///     Home,
///     #[route(id = "document", path = "/documents/{id}")]
///     Document(u64),
/// }
/// struct Screen;
/// impl Render for Screen {
///     fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
///         gpui::div()
///     }
/// }
/// impl Page<Destination> for Screen {
///     fn mount(_: &Destination, _: PageContext<Destination>, _: &mut gpui::Window,
///         cx: &mut gpui::App) -> Result<gpui::Entity<Self>, MountError> {
///         Ok(cx.new(|_| Screen))
///     }
/// }
/// let app = App::new("example.pages").routes(rake::routes! {
///     Destination::Home => Screen,
///     Destination::Document(..) => Screen,
/// });
/// ```
///
/// A registered component must implement `Page`, not only `Render`:
///
/// ```compile_fail
/// #[derive(Clone, rake::Route)]
/// enum Destination { #[route(id = "home", path = "/")] Home }
/// let _ = rake::routes! { Destination::Home => rake::gpui::Empty };
/// ```
///
/// Adding a destination requires updating registration; there is no implicit fallback:
///
/// ```compile_fail
/// # use rake::gpui::{self, AppContext};
/// # use rake::prelude::*;
/// # use rake::router::MountError;
/// #[derive(Clone, rake::Route)]
/// enum Destination {
///     #[route(id = "home", path = "/")] Home,
///     #[route(id = "settings", path = "/settings")] Settings,
/// }
/// # impl Page<Destination> for gpui::Empty {
/// #     fn mount(_: &Destination, _: PageContext<Destination>, _: &mut gpui::Window,
/// #         cx: &mut gpui::App) -> Result<gpui::Entity<Self>, MountError> {
/// #         Ok(cx.new(|_| gpui::Empty))
/// #     }
/// # }
/// let _ = rake::routes! { Destination::Home => gpui::Empty };
/// ```
#[macro_export]
macro_rules! routes {
    ($($pattern:pat $(if $guard:expr)? => $page:ty),+ $(,)?) => {
        $crate::router::__routes(move |__rake_destination, __rake_context, __rake_window, __rake_cx| {
            match __rake_destination {
                $($pattern $(if $guard)? => $crate::router::__mount_page::<_, $page>(
                    __rake_destination, __rake_context, __rake_window, __rake_cx
                ),)+
            }
        })
    };
}
