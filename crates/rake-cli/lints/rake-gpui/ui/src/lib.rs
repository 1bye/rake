//! UI fixtures compiled against the real pinned GPUI, without opening a window.
#![allow(dead_code, unused_must_use)]

use gpui::{Context, Task};

fn dropped_tasks(cx: &mut Context<View>) {
    cx.spawn(async |_, _| {}); //~ gpui_dropped_task
    let _ = cx.spawn(async |_, _| {}); //~ gpui_dropped_task
    Task::ready(()); //~ gpui_dropped_task
    scheduler::Task::ready(()); //~ gpui_dropped_task

    cx.spawn(async |_, _| {}).detach();
    let saved = cx.spawn(async |_, _| {});
    let _kept_until_scope_end = cx.spawn(async |_, _| {});

    let _ = &saved;
    &saved;
    saved.detach();
}

fn returned_task() -> Task<()> {
    Task::ready(())
}

struct TaskLookalike;

impl TaskLookalike {
    fn ready() -> Self {
        Self
    }
}

fn unrelated() {
    TaskLookalike::ready();
    let _ = TaskLookalike::ready();
}

struct View;

use gpui::prelude::*;
use gpui::{App, AppContext, Entity, InteractiveElement, IntoElement, Render, Window, div};
use std::time::Duration;

impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _child = cx.new(|_| View); //~ gpui_render_new
        std::thread::sleep(Duration::ZERO); //~ gpui_ui_blocking
        let _ = std::fs::read("data"); //~ gpui_ui_blocking
        smol::block_on(async {}); //~ gpui_ui_blocking
        futures::executor::block_on(async {}); //~ gpui_ui_blocking

        // Constructing a future or closure does not execute its body on the UI thread.
        let _future = async { std::thread::sleep(Duration::ZERO) };
        let _later = || std::thread::sleep(Duration::ZERO);

        div()
            .id("test")
            .on_click(cx.listener(|_, _, _, _| {
                std::thread::sleep(Duration::ZERO); //~ gpui_ui_blocking
                let _ = std::fs::File::open("data"); //~ gpui_ui_blocking
            }))
            .on_action(cx.listener(|_, _: &gpui::NoAction, _, _| {
                smol::block_on(async {}); //~ gpui_ui_blocking
            }))
    }
}

fn constructors(cx: &mut App) -> Entity<View> {
    cx.new(|_| View)
}

fn reentrant_updates(entity: Entity<View>, other: Entity<View>, cx: &mut App) {
    entity.update(cx, |_, cx| {
        entity.update(cx, |_, _| {}); //~ gpui_reentrant_update
        cx.entity().update(cx, |_, _| {}); //~ gpui_reentrant_update
        other.update(cx, |_, _| {});
        let _not_called = || entity.update(cx, |_, _| {});
        cx.notify();
    });
    entity.update(cx, |_, _| {});
    // Delayed work runs after the outer borrow ends.
    entity.update(cx, |_, cx| {
        cx.spawn(async |entity, cx| {
            entity.update(cx, |_, _| {}).ok();
        })
        .detach();
    });
}

struct Unrelated;

impl Unrelated {
    fn render(&self) {
        std::thread::sleep(Duration::ZERO);
        smol::block_on(async {});
    }
}

fn handler_boundaries(cx: &mut Context<View>) {
    let _listener = cx.listener(|_, _: &(), _, cx| {
        let _worker = || std::fs::read("deferred");
        cx.spawn(async |_, _| std::thread::sleep(Duration::ZERO))
            .detach();
        let _ = cx.new(|_| View);
    });
}

fn direct_handlers(cx: &mut Context<View>) {
    use std::fs::read as read_file;
    use std::thread::sleep as pause;

    let _ = div().on_action(|_: &gpui::NoAction, _, _| {
        let _ = read_file("data"); //~ gpui_ui_blocking
        pause(Duration::ZERO); //~ gpui_ui_blocking
        futures::executor::block_on(async {}); //~ gpui_ui_blocking
    });
    let _ = cx.listener(|_, _: &(), _, _| {
        read_file("data").ok(); //~ gpui_ui_blocking
    });
}

impl Unrelated {
    fn construct_in_unrelated_render(&self, cx: &mut Context<View>) {
        let _ = cx.new(|_| View);
    }
}

mod lookalike {
    struct Task;

    impl Task {
        fn ready() -> Self {
            Self
        }
    }

    fn discarded() {
        Task::ready();
        let _ = Task::ready();
    }
}
