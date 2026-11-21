macro_rules! with_view_context {
    ($window:ident, $cx:ident, $body:block) => {
        #[cfg(feature = "experimental-reactive")]
        {
            struct ContextRoot;
            impl rake::gpui::Render for ContextRoot {
                fn render(
                    &mut self,
                    _: &mut rake::gpui::Window,
                    _: &mut rake::gpui::Context<Self>,
                ) -> impl rake::gpui::IntoElement {
                    rake::gpui::div()
                }
            }
            let mut app = rake::gpui::TestAppContext::single();
            let (_, context) = app.add_window_view(|_, _| ContextRoot);
            context.update(|$window, $cx| {
                let _ = (&$window, &$cx);
                $body
            });
        }
        #[cfg(not(feature = "experimental-reactive"))]
        $body
    };
}
