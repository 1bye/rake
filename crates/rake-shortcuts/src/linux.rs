//! X11 passive grabs and compositor-mediated Wayland portal sessions.
use super::Error;
use futures::{
    FutureExt, StreamExt,
    channel::{mpsc, oneshot},
};
use rake_tray::Accelerator;
use std::time::Duration;

pub(super) struct Registration {
    stop: Option<oneshot::Sender<()>>,
    done: Option<oneshot::Receiver<Result<(), Error>>>,
}

pub(super) async fn available() -> bool {
    if wayland() {
        ashpd::desktop::global_shortcuts::GlobalShortcuts::new()
            .await
            .is_ok()
    } else {
        blocking::unblock(|| x11rb::connect(None).is_ok()).await
    }
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland")
}

pub(super) async fn register(
    accelerator: Accelerator,
    sender: mpsc::Sender<()>,
) -> Result<Registration, Error> {
    let (stop, stopped) = oneshot::channel();
    let (done, finished) = oneshot::channel();
    let (ready, registered) = oneshot::channel();
    let registration = Registration {
        stop: Some(stop),
        done: Some(finished),
    };
    let portal = wayland();

    std::thread::Builder::new()
        .name("rake-shortcut".into())
        .spawn(move || {
            let result = async_io::block_on(async move {
                if portal {
                    portal_worker(accelerator, sender, stopped, ready).await
                } else {
                    x11_worker(accelerator, sender, stopped, ready).await
                }
            });
            let _ = done.send(result);
        })
        .map_err(|error| Error::Native(error.to_string()))?;

    registered
        .await
        .map_err(|_| Error::Native("shortcut worker stopped during registration".into()))??;

    Ok(registration)
}

impl Registration {
    pub(super) async fn unregister(&mut self) -> Result<(), Error> {
        self.stop.take();

        if let Some(done) = self.done.take() {
            done.await.map_err(|_| {
                Error::Native("shortcut worker stopped without cleanup receipt".into())
            })??;
        }

        Ok(())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Closing the sender wakes the worker, which owns and releases all OS resources.
        self.stop.take();
    }
}

async fn portal_worker(
    accelerator: Accelerator,
    mut sender: mpsc::Sender<()>,
    stopped: oneshot::Receiver<()>,
    ready: oneshot::Sender<Result<(), Error>>,
) -> Result<(), Error> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};

    let connection = match ashpd::zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(_) => {
            let _ = ready.send(Err(Error::Unsupported));
            return Ok(());
        }
    };
    // A dedicated connection also releases sessions if setup is cancelled midway.
    let mut ready = Some(ready);
    let result = {
        let operation = async {
            let portal = GlobalShortcuts::with_connection(connection.clone())
                .await
                .map_err(portal_error)?;
            let session = portal
                .create_session(Default::default())
                .await
                .map_err(portal_error)?;
            let mut events = portal
                .receive_activated()
                .await
                .map_err(portal_error)?
                .boxed();
            let trigger = portal_trigger(&accelerator);
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let id = format!(
                "rake-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let shortcut = NewShortcut::new(&id, "Activate application shortcut")
                .preferred_trigger(Some(trigger.as_str()));
            let response = portal
                .bind_shortcuts(&session, &[shortcut], None, Default::default())
                .await
                .map_err(portal_error)?
                .response()
                .map_err(portal_error)?;

            if !response
                .shortcuts()
                .iter()
                .any(|shortcut| shortcut.id() == id)
            {
                return Err(Error::PermissionDenied);
            }

            let _ = ready.take().expect("registration receipt").send(Ok(()));

            while let Some(event) = events.next().await {
                if event.shortcut_id() == id {
                    let _ = sender.try_send(());
                }
            }

            session.close().await.map_err(portal_error)
        }
        .fuse();
        let stopped = stopped.fuse();
        futures::pin_mut!(operation, stopped);
        futures::select! {
            result = operation => result,
            _ = stopped => Ok(()),
        }
    };
    // Disconnecting a private D-Bus client closes all its portal sessions, including
    // a pending authorization dialog. It cannot affect another Rake registration.
    connection
        .close()
        .await
        .map_err(|error| Error::Native(error.to_string()))?;

    if let Some(ready) = ready {
        let _ = ready.send(result);

        return Ok(());
    }

    result
}

fn portal_trigger(accelerator: &Accelerator) -> String {
    let mut trigger = String::new();

    for (enabled, name) in [
        (accelerator.control, "CTRL+"),
        (accelerator.alt, "ALT+"),
        (accelerator.shift, "SHIFT+"),
        (accelerator.super_key, "LOGO+"),
    ] {
        if enabled {
            trigger.push_str(name);
        }
    }

    trigger.push_str(&accelerator.key);

    trigger
}

fn portal_error(error: ashpd::Error) -> Error {
    match error {
        ashpd::Error::PortalNotFound(_) => Error::Unsupported,
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => Error::PermissionDenied,
        error => Error::Native(error.to_string()),
    }
}

async fn x11_worker(
    accelerator: Accelerator,
    mut sender: mpsc::Sender<()>,
    stopped: oneshot::Receiver<()>,
    ready: oneshot::Sender<Result<(), Error>>,
) -> Result<(), Error> {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{ConnectionExt, GrabMode, ModMask},
        },
    };

    let setup = || {
        let (connection, screen) = x11rb::connect(None).map_err(|_| Error::Unsupported)?;
        let root = connection.setup().roots[screen].root;
        let key = accelerator
            .key
            .chars()
            .next()
            .filter(|key| key.is_ascii())
            .ok_or_else(|| {
                Error::InvalidInput("X11 global shortcuts require an ASCII key".into())
            })? as u32;
        let first = connection.setup().min_keycode;
        let count = connection.setup().max_keycode - first + 1;
        let mapping = connection
            .get_keyboard_mapping(first, count)
            .map_err(x11_error)?
            .reply()
            .map_err(x11_error)?;
        let keycode = mapping
            .keysyms
            .chunks(usize::from(mapping.keysyms_per_keycode))
            .position(|symbols| symbols.first() == Some(&key))
            .map(|index| first + index as u8)
            .ok_or_else(|| {
                Error::InvalidInput("key is absent from the current X11 layout".into())
            })?;
        let modifiers = u16::from(accelerator.control) * u16::from(ModMask::CONTROL)
            | u16::from(accelerator.alt) * u16::from(ModMask::M1)
            | u16::from(accelerator.shift) * u16::from(ModMask::SHIFT)
            | u16::from(accelerator.super_key) * u16::from(ModMask::M4);
        let modifier_mapping = connection
            .get_modifier_mapping()
            .map_err(x11_error)?
            .reply()
            .map_err(x11_error)?;
        let num_lock_code = mapping
            .keysyms
            .chunks(usize::from(mapping.keysyms_per_keycode))
            .position(|symbols| symbols.contains(&0xff7f))
            .map(|index| first + index as u8);
        let num_lock = modifier_mapping
            .keycodes
            .chunks(usize::from(modifier_mapping.keycodes_per_modifier()))
            .position(|keys| num_lock_code.is_some_and(|key| keys.contains(&key)))
            .map_or(0, |index| 1_u16 << index);
        let mut locks = vec![
            0,
            u16::from(ModMask::LOCK),
            num_lock,
            num_lock | u16::from(ModMask::LOCK),
        ];
        locks.sort_unstable();
        locks.dedup();

        for lock in &locks {
            connection
                .grab_key(
                    false,
                    root,
                    (modifiers | lock).into(),
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(x11_error)?
                .check()
                .map_err(|error| match error {
                    x11rb::errors::ReplyError::X11Error(ref native)
                        if native.error_kind == x11rb::protocol::ErrorKind::Access =>
                    {
                        Error::Conflict
                    }
                    error => x11_error(error),
                })?;
        }

        Ok::<_, Error>((connection, root, keycode, modifiers, locks))
    };
    let (connection, root, keycode, modifiers, locks) = match setup() {
        Ok(state) => state,
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    let _ = ready.send(Ok(()));
    let stopped = stopped.fuse();
    futures::pin_mut!(stopped);

    loop {
        let tick = async_io::Timer::after(Duration::from_millis(10)).fuse();
        futures::pin_mut!(tick);
        futures::select! { _ = stopped => break, _ = tick => {} }

        while let Some(event) = connection.poll_for_event().map_err(x11_error)? {
            if matches!(event, Event::KeyPress(ref event) if event.detail == keycode) {
                let _ = sender.try_send(());
            }
        }
    }

    for lock in locks {
        connection
            .ungrab_key(keycode, root, (modifiers | lock).into())
            .map_err(x11_error)?
            .check()
            .map_err(x11_error)?;
    }

    connection.flush().map_err(x11_error)
}

fn x11_error(error: impl std::fmt::Display) -> Error {
    Error::Native(error.to_string())
}
