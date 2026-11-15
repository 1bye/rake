# Single-instance applications

Opt in with `App::new("com.example.app").single_instance()` before `run()`.
Use the same stable application ID for every launch. The policy is per user
and has no effect on the web.

A subsequent process forwards its launch arguments and exits before creating
a window. URLs and existing file paths reach `services.platform().next_open_url()`;
files become absolute `file:` URLs using the launching process's working directory.
Keep one consumer and validate schemes and destinations before opening content.
Non-URL arguments do not enter the URL inbox.

Use `.on_reopen(|cx| { /* restore or open a window */ })` for native reopen
events, including macOS dock activation. Every forwarded launch also invokes this
callback, even when it has no arguments. Without a callback, Rake activates the
running app with `cx.activate(true)`. The callback receives the GPUI application.
Single-letter URL schemes are treated as file paths, including missing Windows
drive-letter paths; they never enter the inbox as application URLs.

The primary holds a kernel lock on an owner-only file. The file remains after
exit, but crashes release the lock automatically. Never delete lock files while
an application is running. Forwarding uses the existing private socket directory
or owner-only local named pipes, with bounded frames and a five-second timeout.
A failed forward returns a startup error rather than opening another instance.
