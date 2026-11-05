use crate::{
    Anchor, CoordinateSpace, Error, EventKind, HostPolicy, Input, Point, Registration, Request,
    Tray, host::Sink,
};
use ksni::TrayMethods;
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct Model {
    spec: Tray,
    sink: Sink,
    revision: u64,
    online: Arc<AtomicBool>,
}

impl Model {
    fn emit(&self, kind: EventKind, point: Option<Point>) {
        self.sink.send(
            kind,
            point.map(|point| Anchor {
                rect: None,
                point: Some(point),
                space: CoordinateSpace::HostHint,
            }),
            Input {
                activation_token: ksni::current_xdg_activation_token(),
                ..Input::default()
            },
        );
    }
}

impl ksni::Tray for Model {
    fn id(&self) -> String {
        self.spec.id.clone()
    }
    fn title(&self) -> String {
        self.spec.tooltip.clone()
    }
    fn icon_name(&self) -> String {
        self.spec
            .image
            .as_ref()
            .and_then(|i| i.name.clone())
            .unwrap_or_default()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.spec
            .image
            .iter()
            .flat_map(|image| image.representations.iter())
            .map(|rep| {
                let mut argb = Vec::with_capacity(rep.rgba.len());

                for rgba in rep.rgba.chunks_exact(4) {
                    argb.extend_from_slice(&[rgba[3], rgba[0], rgba[1], rgba[2]]);
                }

                ksni::Icon {
                    width: rep.width as i32,
                    height: rep.height as i32,
                    data: argb,
                }
            })
            .collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.spec.tooltip.clone(),
            ..Default::default()
        }
    }
    fn status(&self) -> ksni::Status {
        if self.spec.visible {
            ksni::Status::Active
        } else {
            ksni::Status::Passive
        }
    }
    fn activate(&mut self, x: i32, y: i32) {
        self.emit(
            EventKind::Activate,
            Some(Point {
                x: x.into(),
                y: y.into(),
            }),
        );
    }
    fn secondary_activate(&mut self, x: i32, y: i32) {
        self.emit(
            EventKind::SecondaryActivate,
            Some(Point {
                x: x.into(),
                y: y.into(),
            }),
        );
    }
    fn scroll(&mut self, delta: i32, orientation: ksni::Orientation) {
        self.emit(
            EventKind::Scroll {
                delta,
                horizontal: matches!(orientation, ksni::Orientation::Horizontal),
            },
            None,
        );
    }
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Release);
        self.emit(EventKind::HostChanged(true), None);
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Release);
        self.emit(EventKind::HostChanged(false), None);
        true
    }
}

pub(crate) struct Item {
    handle: Rc<ksni::Handle<Model>>,
    online: Arc<AtomicBool>,
}

impl Item {
    pub(crate) fn check_thread() -> Result<(), Error> {
        Ok(())
    }
    pub(crate) fn create(application: &str, spec: &Tray, sink: Sink) -> Request<Self> {
        let spec = spec.clone();
        let identity = format!("{application}.{}", spec.id);
        // The exported stable ID is namespaced; callback IDs remain application-local.
        let online = Arc::new(AtomicBool::new(true));
        let observed = online.clone();

        Box::pin(async move {
            let wait = spec.host_policy == HostPolicy::Wait;
            let mut model = Model {
                spec,
                sink,
                revision: 0,
                online: observed,
            };
            model.spec.id = identity;
            let handle = model
                .assume_sni_available(wait)
                .spawn()
                .await
                .map_err(|e| crate::native("register SNI", e))?;

            Ok(Self {
                handle: Rc::new(handle),
                online,
            })
        })
    }
    pub(crate) fn apply(&self, revision: u64, spec: &Tray) -> Request<()> {
        let handle = self.handle.clone();
        let spec = spec.clone();

        Box::pin(async move {
            handle
                .update(move |model| {
                    if model.revision <= revision {
                        let native_id = model.spec.id.clone();
                        model.spec = spec;
                        model.spec.id = native_id;
                        model.revision = revision;
                    }
                })
                .await
                .ok_or(Error::Stopped)
        })
    }
    pub(crate) fn registration(&self) -> Registration {
        if self.online.load(Ordering::Acquire) {
            Registration::Registered
        } else {
            Registration::WaitingForHost
        }
    }
    pub(crate) fn anchor(&self) -> Result<Option<Anchor>, Error> {
        Ok(None)
    }
    pub(crate) fn shutdown(&self) -> Request<()> {
        let shutdown = self.handle.shutdown();

        Box::pin(async move {
            shutdown.await;
            Ok(())
        })
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        drop(self.handle.shutdown());
    }
}
