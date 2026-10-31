//! Route entry points and exhaustive, statically dispatched page registration.
use super::{MountError, PageContext, PreparedPage, Route, RouteViews};
use crate::loading::LoadContext;
use futures::future::LocalBoxFuture;
use gpui::{AnyView, App, Entity, Render, Window};
use std::error::Error;
use std::future::Future;

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

/// A route page whose data is loaded before its retained GPUI entity is mounted.
///
/// Loading runs after entry guards and before a physical NewWindow host is
/// created. The router owns the returned data until [`Self::mount`] consumes it;
/// cancellation or a failed mount drops it without changing the current entry.
pub trait LoadedPage<R: Route>: Render + Sized {
    /// Owned value passed from [`Self::load`] into [`Self::mount`].
    type Data: 'static;
    /// Source failure returned by the route loader.
    type Error: Error + 'static;

    /// Loads this route's typed data on the local application runtime.
    fn load(
        destination: R,
        context: LoadContext,
    ) -> impl Future<Output = Result<Self::Data, Self::Error>> + 'static;

    /// Constructs the retained entity from the owned loaded value.
    fn mount(
        destination: &R,
        data: Self::Data,
        page: PageContext<R>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>, MountError>;
}

struct RegisteredPages<F>(F);

struct PreparedPages<F, G> {
    prepare: F,
    mount: G,
}

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
pub fn __routes_prepared<R: Route>(
    prepare: impl Fn(
        &R,
        LoadContext,
    ) -> futures::future::LocalBoxFuture<'static, Result<PreparedPage, MountError>>
    + 'static,
    mount: impl Fn(
        &R,
        PreparedPage,
        PageContext<R>,
        &mut Window,
        &mut App,
    ) -> Result<AnyView, MountError>
    + 'static,
) -> impl RouteViews<R> {
    PreparedPages { prepare, mount }
}

impl<R, F, G> RouteViews<R> for PreparedPages<F, G>
where
    R: Route,
    F: Fn(
            &R,
            LoadContext,
        ) -> futures::future::LocalBoxFuture<'static, Result<PreparedPage, MountError>>
        + 'static,
    G: Fn(&R, PreparedPage, PageContext<R>, &mut Window, &mut App) -> Result<AnyView, MountError>
        + 'static,
{
    fn prepare(
        &self,
        destination: &R,
        context: LoadContext,
    ) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
        (self.prepare)(destination, context)
    }

    fn mount(
        &self,
        destination: &R,
        page: PageContext<R>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<AnyView, MountError> {
        (self.mount)(destination, PreparedPage::empty(), page, window, cx)
    }

    fn mount_prepared(
        &self,
        destination: &R,
        prepared: PreparedPage,
        page: PageContext<R>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<AnyView, MountError> {
        (self.mount)(destination, prepared, page, window, cx)
    }
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

#[doc(hidden)]
pub fn __prepare_loaded_page<R: Route, P: LoadedPage<R>>(
    destination: &R,
    context: LoadContext,
) -> futures::future::LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
    let destination = destination.clone();

    Box::pin(async move {
        P::load(destination, context)
            .await
            .map(PreparedPage::from_value)
            .map_err(|error| MountError(format!("route load failed: {error}")))
    })
}

#[doc(hidden)]
pub fn __mount_loaded_page<R: Route, P: LoadedPage<R>>(
    destination: &R,
    prepared: PreparedPage,
    page: PageContext<R>,
    window: &mut Window,
    cx: &mut App,
) -> Result<AnyView, MountError> {
    let data = prepared.downcast::<P::Data>()?;

    P::mount(destination, data, page, window, cx).map(Into::into)
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
    ($($tokens:tt)*) => {
        $crate::__rake_routes_parse!(@parse [] $($tokens)*)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __rake_routes_parse {
    (@parse [$($arms:tt)*]) => {
        $crate::__rake_routes_emit!([$($arms)*])
    };
    (@parse [$($arms:tt)*]
        $pattern:pat $(if $guard:expr)? => loaded($page:ty), $($rest:tt)*) => {
        $crate::__rake_routes_parse! {
            @parse [$($arms)* ($pattern $(if $guard)? => loaded($page)),]
            $($rest)*
        }
    };
    (@parse [$($arms:tt)*]
        $pattern:pat $(if $guard:expr)? => loaded($page:ty),) => {
        $crate::__rake_routes_parse! {
            @parse [$($arms)* ($pattern $(if $guard)? => loaded($page)),]
        }
    };
    (@parse [$($arms:tt)*]
        $pattern:pat $(if $guard:expr)? => loaded($page:ty)) => {
        $crate::__rake_routes_parse! {
            @parse [$($arms)* ($pattern $(if $guard)? => loaded($page)),]
        }
    };
    (@parse [$($arms:tt)*]
        $pattern:pat $(if $guard:expr)? => $page:ty, $($rest:tt)*) => {
        $crate::__rake_routes_parse! {
            @parse [$($arms)* ($pattern $(if $guard)? => $page),]
            $($rest)*
        }
    };
    (@parse [$($arms:tt)*]
        $pattern:pat $(if $guard:expr)? => $page:ty,) => {
        $crate::__rake_routes_parse! {
            @parse [$($arms)* ($pattern $(if $guard)? => $page),]
        }
    };
    (@parse [$($arms:tt)*]
        $pattern:pat $(if $guard:expr)? => $page:ty) => {
        $crate::__rake_routes_parse! {
            @parse [$($arms)* ($pattern $(if $guard)? => $page),]
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __rake_routes_emit {
    ([$(($pattern:pat $(if $guard:expr)? => $($spec:tt)+)),+ $(,)?]) => {
        $crate::router::__routes_prepared(
            move |__rake_destination, __rake_load| {
                match __rake_destination {
                    $($pattern $(if $guard)? => $crate::__rake_route_prepare!(
                        __rake_destination, __rake_load, $($spec)+
                    ),)+
                }
            },
            move |__rake_destination, __rake_prepared, __rake_context, __rake_window, __rake_cx| {
                match __rake_destination {
                    $($pattern $(if $guard)? => $crate::__rake_route_mount!(
                        __rake_destination,
                        __rake_prepared,
                        __rake_context,
                        __rake_window,
                        __rake_cx,
                        $($spec)+
                    ),)+
                }
            },
        )
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __rake_route_prepare {
    ($destination:expr, $context:expr, loaded($page:ty)) => {
        $crate::router::__prepare_loaded_page::<_, $page>($destination, $context)
    };
    ($destination:expr, $context:expr, $page:ty) => {
        Box::pin(async { Ok($crate::router::PreparedPage::empty()) })
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __rake_route_mount {
    ($destination:expr, $prepared:expr, $context:expr, $window:expr, $cx:expr, loaded($page:ty)) => {
        $crate::router::__mount_loaded_page::<_, $page>(
            $destination,
            $prepared,
            $context,
            $window,
            $cx,
        )
    };
    ($destination:expr, $prepared:expr, $context:expr, $window:expr, $cx:expr, $page:ty) => {
        $crate::router::__mount_page::<_, $page>($destination, $context, $window, $cx)
    };
}
