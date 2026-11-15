//! Large-directory native icon listing. Pass a directory as the first argument.

use rake::gpui::{
    self, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, px,
};
use std::path::PathBuf;

struct Listing {
    paths: Vec<PathBuf>,
    status: String,
}

impl Listing {
    fn new(cx: &mut Context<Self>) -> Self {
        let path = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| ".".into());
        cx.spawn(async move |entity, cx| {
            let result = blocking::unblock(move || {
                let mut paths = std::fs::read_dir(path)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<Result<Vec<_>, std::io::Error>>()?;
                paths.sort();

                Ok::<_, std::io::Error>(paths)
            })
            .await;
            let _ = entity.update(cx, |state, cx| {
                match result {
                    Ok(paths) => {
                        state.status =
                            format!("{} entries — scroll to load more icons", paths.len());
                        state.paths = paths;
                    }
                    Err(error) => state.status = error.to_string(),
                }

                cx.notify();
            });
        })
        .detach();

        Self {
            paths: Vec::new(),
            status: "Reading directory…".into(),
        }
    }
}

impl Render for Listing {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stats = rake::platform::icons::inspect(cx);
        gpui::div().size_full().flex().flex_col()
            .child(format!("{} | {} cached bitmaps, {} bytes", self.status, stats.entries, stats.bytes))
            .child(gpui::uniform_list("file-icons", self.paths.len(),
                cx.processor(|state, range: std::ops::Range<usize>, window, cx| {
                    range.map(|index| {
                        let path = &state.paths[index];
                        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        let icon = rake::platform::icons::for_path(path, px(22.), window, cx);

                        rake::view! {
                            <div id={("file", gpui::hash(path))} class="h-[30px] flex-row items-center gap-2">
                                <img src={icon} alt="" class="size-[22px]"/>
                                {label}
                            </div>
                        }
                    }).collect()
                })).flex_1())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    rake::App::new("example.file-icons")
        .root(|_, _, cx| cx.new(Listing::new).into())
        .run()?;

    Ok(())
}
