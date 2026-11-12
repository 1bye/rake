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
