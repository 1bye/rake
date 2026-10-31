//! Native agent fixture: actual mouse, focused Unicode input, keys, drag and scroll.

use rake::gpui::{self, prelude::*, *};
use std::ops::Range;

struct Demo {
    count: usize,
    keys: usize,
    ticks: usize,
    covered: bool,
    http: Option<rake::http::HttpClient>,
    http_status: String,
    network: Option<Task<()>>,
    animation: Option<Task<()>>,
    drags: usize,
    text: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    input_bounds: Bounds<Pixels>,
}

// Platform text ranges use UTF-16; the model always stores valid UTF-8 boundaries.
fn byte(text: &str, utf16: usize) -> usize {
    let mut units = 0;

    for (index, c) in text.char_indices() {
        if units + c.len_utf16() > utf16 {
            return index;
        }

        units += c.len_utf16();
    }

    text.len()
}

impl Demo {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            count: 0,
            keys: 0,
            ticks: 0,
            covered: false,
            http: None,
            http_status: "idle".into(),
            network: None,
            animation: None,
            drags: 0,
            text: String::new(),
            selection: 0..0,
            marked: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            input_bounds: Bounds::default(),
        }
    }

    fn utf16(&self, range: Range<usize>) -> Range<usize> {
        self.text[..range.start].encode_utf16().count()
            ..self.text[..range.end].encode_utf16().count()
    }

    fn range(&self, range: Range<usize>) -> Range<usize> {
        byte(&self.text, range.start.min(range.end))..byte(&self.text, range.start.max(range.end))
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str, cx: &mut Context<Self>) -> bool {
        let range = range
            .map(|r| self.range(r))
            .or_else(|| self.marked.take())
            .unwrap_or_else(|| self.selection.clone());

        if self.text.len() - range.len() + text.len() > 4096 {
            return false;
        }

        self.text.replace_range(range.clone(), text);
        let cursor = range.start + text.len();

        self.selection = cursor..cursor;
        self.marked = None;
        cx.notify();

        true
    }
}

impl EntityInputHandler for Demo {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range(range);

        *actual = Some(self.utf16(range.clone()));

        Some(self.text[range].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16(self.selection.clone()),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|r| self.utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.replace(range, text, cx) {
            return;
        }

        let end = self.selection.end;
        let start = end.saturating_sub(text.len());

        self.marked = Some(start..end);

        if let Some(selected) = selected {
            self.selection = start + byte(&self.text[start..end], selected.start)
                ..start + byte(&self.text[start..end], selected.end);
        }
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(self.input_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.count;
        let keys = self.keys;
        let drags = self.drags;
        let ticks = self.ticks;
        let covered = self.covered;
        let http_status = self.http_status.clone();
        let text = self.text.clone();
        let entity = cx.entity();
        let painted = entity.clone();
        let focus = self.focus.clone();

        let input = canvas(
            move |bounds, _, cx| entity.update(cx, |this, _| this.input_bounds = bounds),
            move |bounds, _, window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, painted), cx)
            },
        )
        .absolute()
        .size_full();

        rake::view! {
            <div id="agent-root" class="size-full flex flex-col p-4 gap-2" style={|s|s.bg(rgb(0x172330)).text_color(rgb(0xffffff))}
                on:key-down={cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "f2" { this.keys += 1; cx.notify(); }
                    if event.keystroke.key == "f6" { this.covered = !this.covered;cx.notify(); }
                    if event.keystroke.key == "f3" { window.resize(size(px(700.),px(740.))); }
                    if event.keystroke.key == "f4" {
                        this.ticks = 0;
                        this.animation = Some(cx.spawn(async move |view, cx| {
                            for _ in 0..24 {
                                cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
                                if view.update(cx, |this,cx| { this.ticks+=1;cx.notify(); }).is_err() { break; }
                            }
                        }));
                    }
                    if event.keystroke.key == "f5" && let Some(http)=this.http.clone() {
                        let url=std::env::var("RAKE_AGENT_HTTP_URL").unwrap_or_else(|_|"http://127.0.0.1:1".into());
                        this.network=Some(cx.spawn(async move |view,cx| {
                            let status=match http.get(&url).send().await {
                                Ok(response)=>{let status=response.status();let _=response.bytes().await;status.to_string()},
                                Err(error)=>error.to_string(),
                            };
                            rake::warn!(status=%status,"Agent HTTP diagnostic");
                            let _=view.update(cx,|this,cx|{this.http_status=status;cx.notify();});
                        }));
                    }
                    if event.keystroke.modifiers.platform && event.keystroke.key == "a" { this.selection = 0..this.text.len(); cx.stop_propagation(); }
                })}>
                <h1>"Native agent workflow"</h1>
                <div id="agent-button-stack" class="relative">
                <button id="agent-increment" aria-label="Increment counter" class="w-full p-2 rounded-md" style={|s|s.bg(rgb(0x2857a4))}
                    on:click={cx.listener(|this, _, _, cx| {this.count+=1;cx.notify();})}>"Increment"</button>
                if covered {
                    <div id="agent-cover" class="absolute inset-0 flex items-center justify-center" style={|s|s.bg(rgb(0x665533))}>"Cover (pointer interception unknown)"</div>
                }
                </div>
                <button id="agent-disabled" disabled={true} class="px-2 py-1">"Disabled target"</button>
                <p id="agent-count">{format!("Count: {count}")}</p>
                <div id="agent-input" role={gpui::accesskit::Role::TextInput} aria-label="Message" focus={&self.focus}
                    class="relative p-2 rounded-md" style={|s|s.bg(rgb(0x304766))}
                    on:mouse-down={cx.listener(|this, _: &MouseDownEvent, window, cx|window.focus(&this.focus,cx))}>
                    {if text.is_empty() { "Click and type".into() } else { text.clone() }}
                    {input}
                </div>
                <p id="agent-echo">{format!("Text: {text}")}</p>
                <p id="agent-http">{format!("HTTP: {http_status}")}</p>
                <p id="agent-keys">{format!("Keys: {keys}; ticks: {ticks}")}</p>
                <div id="agent-drag" class="p-3 rounded-md" style={|s|s.bg(rgb(0x395b4b))}
                    on:mouse-move={cx.listener(|this, event: &MouseMoveEvent, _, cx| {if event.dragging() {this.drags+=1;cx.notify();}})}>
                    {format!("Drags: {drags}")}
                </div>
                <div id="agent-scroll" scroll={&self.scroll} class="flex-1 min-h-0 overflow-y-scroll flex flex-col gap-2">
                    for row in 1..=40 {
                        <p id={format!("agent-row-{row}")} class="p-3" style={|s|s.bg(rgb(0x243749))}>{format!("Row {row}")}</p>
                    }
                </div>
            </div>
        }
    }
}

// Router mode exercises multiple inspected windows in the same native runtime.
#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "demo", path = "/")]
    Demo,
}

struct Routes;

impl rake::prelude::RouteViews<Destination> for Routes {
    fn mount(
        &self,
        _: &Destination,
        _: rake::prelude::PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, rake::router::MountError> {
        Ok(cx.new(Demo::new).into())
    }
}

fn main() -> Result<(), rake::AppError> {
    let app = rake::App::new("com.example.rake.agent")
        .control(rake::control::ControlConfig::FromEnvironment)
        .window_options(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(120.), px(120.)),
                size: size(px(640.), px(680.)),
            })),
            ..Default::default()
        });

    if std::env::var_os("RAKE_AGENT_ROUTER").is_some() {
        app.routes(Routes)
            .workspace_chrome(false)
            .initial(Destination::Demo)
            .run()
    } else {
        app.root(|services, _, cx| {
            cx.new(|cx| {
                let mut demo = Demo::new(cx);

                demo.http = Some(services.http());

                demo
            })
            .into()
        })
        .run()
    }
}
