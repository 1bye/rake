use crate::{
    Anchor, CheckState, CoordinateSpace, Error, EventKind, HostPolicy, Input, Interaction,
    ItemKind, Menu, Point, Registration, Request, Tray, host::Sink,
};
use ksni::TrayMethods;
use std::{
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use super::mnemonic;
fn name(image: Option<&crate::Image>) -> String {
    image
        .and_then(|image| image.name.clone())
        .unwrap_or_default()
}
fn pixmaps(image: Option<&crate::Image>) -> Vec<ksni::Icon> {
    image
        .into_iter()
        .flat_map(|image| &image.representations)
        .map(|rep| {
            let mut argb = Vec::with_capacity(rep.rgba.len());
            for rgba in rep.rgba.as_chunks::<4>().0 {
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
fn markup(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

type Icons = BTreeMap<String, (String, Vec<u8>)>;
struct Model<const MENU: bool> {
    identity: String,
    spec: Tray,
    sink: Sink,
    revision: u64,
    online: Arc<AtomicBool>,
    icons: Icons,
}

impl<const MENU: bool> Model<MENU> {
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

impl<const MENU: bool> ksni::Tray for Model<MENU> {
    const MENU_ON_ACTIVATE: bool = MENU;
    fn id(&self) -> String {
        self.identity.clone()
    }
    fn title(&self) -> String {
        self.spec
            .linux
            .as_ref()
            .map(|linux| linux.title.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if self.spec.tooltip.is_empty() {
                    self.spec.id.clone()
                } else {
                    self.spec.tooltip.clone()
                }
            })
    }
    fn category(&self) -> ksni::Category {
        match self
            .spec
            .linux
            .as_ref()
            .map(|l| l.category)
            .unwrap_or_default()
        {
            crate::linux::Category::ApplicationStatus => ksni::Category::ApplicationStatus,
            crate::linux::Category::Communications => ksni::Category::Communications,
            crate::linux::Category::SystemServices => ksni::Category::SystemServices,
            crate::linux::Category::Hardware => ksni::Category::Hardware,
        }
    }
    fn icon_theme_path(&self) -> String {
        self.spec
            .linux
            .as_ref()
            .map(|l| l.icon_theme_path.clone())
            .unwrap_or_default()
    }
    fn icon_name(&self) -> String {
        name(self.spec.image.as_ref())
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.spec.image.as_ref())
    }
    fn overlay_icon_name(&self) -> String {
        name(self.spec.linux.as_ref().and_then(|l| l.overlay.as_ref()))
    }
    fn overlay_icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.spec.linux.as_ref().and_then(|l| l.overlay.as_ref()))
    }
    fn attention_icon_name(&self) -> String {
        name(self.spec.linux.as_ref().and_then(|l| l.attention.as_ref()))
    }
    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.spec.linux.as_ref().and_then(|l| l.attention.as_ref()))
    }
    fn attention_movie_name(&self) -> String {
        self.spec
            .linux
            .as_ref()
            .map(|l| l.attention_movie.clone())
            .unwrap_or_default()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        if let Some(tooltip) = self
            .spec
            .linux
            .as_ref()
            .and_then(|linux| linux.tooltip.as_ref())
        {
            ksni::ToolTip {
                title: tooltip.title.clone(),
                description: markup(&tooltip.body),
                icon_name: name(tooltip.image.as_ref()),
                icon_pixmap: pixmaps(tooltip.image.as_ref()),
            }
        } else {
            ksni::ToolTip {
                title: self.spec.tooltip.clone(),
                ..Default::default()
            }
        }
    }
    fn status(&self) -> ksni::Status {
        if !self.spec.visible {
            return ksni::Status::Passive;
        }
        match self
            .spec
            .linux
            .as_ref()
            .map(|linux| linux.status)
            .unwrap_or_default()
        {
            crate::linux::Status::Active => ksni::Status::Active,
            crate::linux::Status::Passive => ksni::Status::Passive,
            crate::linux::Status::NeedsAttention => ksni::Status::NeedsAttention,
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
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.spec
            .menu
            .as_ref()
            .map(|menu| project(menu, &self.sink.menu(self.revision), &self.icons))
            .unwrap_or_default()
    }
    fn menu_about_to_show(&mut self) {
        self.emit(EventKind::MenuOpened, None);
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

fn icons(menu: Option<&Menu>) -> Result<Icons, Error> {
    let mut result = BTreeMap::new();
    if let Some(menu) = menu {
        for item in &menu.items {
            if let Some(image) = &item.image {
                let png = if image.representations.is_empty() {
                    Vec::new()
                } else {
                    image.to_png()?
                };
                result.insert(
                    item.id.clone(),
                    (image.name.clone().unwrap_or_default(), png),
                );
            }
            if let ItemKind::Submenu(menu) = &item.kind {
                result.extend(icons(Some(menu))?);
            }
        }
    }
    Ok(result)
}

fn project<const MENU: bool>(
    menu: &Menu,
    sink: &Sink,
    images: &Icons,
) -> Vec<ksni::MenuItem<Model<MENU>>> {
    use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
    menu.items
        .iter()
        .map(|item| {
            let label = mnemonic(&item.label, item.access_key, '_');
            let shortcut = item
                .accelerator
                .as_ref()
                .map(|value| crate::Accelerator::parse(value))
                .transpose()
                .expect("menu was validated")
                .map(|key| {
                    let mut keys = Vec::new();
                    if key.control {
                        keys.push("Control".into());
                    }
                    if key.alt {
                        keys.push("Alt".into());
                    }
                    if key.shift {
                        keys.push("Shift".into());
                    }
                    if key.super_key {
                        keys.push("Super".into());
                    }
                    keys.push(key.key);
                    vec![keys]
                })
                .unwrap_or_default();
            let (icon_name, icon_data) = images.get(&item.id).cloned().unwrap_or_default();
            let source = sink.clone();
            let id = item.id.clone();
            let activate = Box::new(move |_: &mut Model<MENU>| {
                source.send(
                    EventKind::MenuSelected(id.clone()),
                    None,
                    Input {
                        activation_token: ksni::current_xdg_activation_token(),
                        ..Input::default()
                    },
                );
            });
            match &item.kind {
                ItemKind::Separator => ksni::MenuItem::Separator,
                ItemKind::Submenu(submenu) => SubMenu {
                    label,
                    enabled: item.enabled,
                    visible: item.visible,
                    icon_name,
                    icon_data,
                    submenu: project(submenu, sink, images),
                    ..Default::default()
                }
                .into(),
                ItemKind::Check(checked) => CheckmarkItem {
                    label,
                    enabled: item.enabled,
                    visible: item.visible,
                    checked: *checked == CheckState::On,
                    icon_name,
                    icon_data,
                    shortcut,
                    activate,
                    ..Default::default()
                }
                .into(),
                ItemKind::Radio { selected, .. } => RadioGroup {
                    selected: if *selected { 0 } else { usize::MAX },
                    options: vec![RadioItem {
                        label,
                        enabled: item.enabled,
                        visible: item.visible,
                        icon_name,
                        icon_data,
                        shortcut,
                        ..Default::default()
                    }],
                    select: Box::new(move |model, _| activate(model)),
                }
                .into(),
                _ => StandardItem {
                    label,
                    enabled: item.enabled && !matches!(item.kind, ItemKind::Header),
                    visible: item.visible,
                    icon_name,
                    icon_data,
                    shortcut,
                    activate,
                    ..Default::default()
                }
                .into(),
            }
        })
        .collect()
}

enum Driver {
    Menu(Rc<ksni::Handle<Model<true>>>),
    Events(Rc<ksni::Handle<Model<false>>>),
}
pub(crate) struct Item {
    driver: Driver,
    online: Arc<AtomicBool>,
    sink: Sink,
}

async fn start<const MENU: bool>(
    identity: String,
    spec: Tray,
    sink: Sink,
    online: Arc<AtomicBool>,
) -> Result<Rc<ksni::Handle<Model<MENU>>>, Error> {
    let wait = spec.host_policy == HostPolicy::Wait;
    let sandboxed = spec.linux.as_ref().is_some_and(|linux| linux.sandboxed);
    let icons = icons(spec.menu.as_ref())?;
    let model = Model {
        identity,
        spec,
        sink,
        revision: 0,
        online,
        icons,
    };
    let handle = model
        .disable_dbus_name(sandboxed)
        .assume_sni_available(wait)
        .spawn()
        .await
        .map_err(|e| crate::native("register SNI", e))?;
    Ok(Rc::new(handle))
}

fn stopped(online: &AtomicBool, sink: &Sink) -> Error {
    if online.swap(false, Ordering::AcqRel) {
        sink.send(EventKind::HostChanged(false), None, Input::default());
    }

    Error::Stopped
}

fn update<const MENU: bool>(
    handle: &Rc<ksni::Handle<Model<MENU>>>,
    revision: u64,
    spec: &Tray,
    online: &Arc<AtomicBool>,
    sink: &Sink,
) -> Request<()> {
    let handle = handle.clone();
    let online = online.clone();
    let sink = sink.clone();
    let spec = spec.clone();
    Box::pin(async move {
        let icons = icons(spec.menu.as_ref())?;
        if handle.is_closed() {
            return Err(stopped(&online, &sink));
        }
        // Upstream acknowledges local model processing, but does not expose
        // property/menu signal export errors. Never describe this as a wire ack.
        let updated = handle
            .update(move |model| {
                if model.revision <= revision {
                    model.spec = spec;
                    model.revision = revision;
                    model.icons = icons;
                }
            })
            .await;
        if updated.is_none() || handle.is_closed() {
            return Err(stopped(&online, &sink));
        }
        Ok(())
    })
}

impl Item {
    pub(crate) fn check_thread() -> Result<(), Error> {
        Ok(())
    }
    pub(crate) fn create(application: &str, spec: &Tray, sink: Sink) -> Request<Self> {
        let spec = spec.clone();
        let identity = format!("{application}.{}", spec.id);
        let online = Arc::new(AtomicBool::new(true));
        Box::pin(async move {
            let events = sink.clone();
            let driver = if spec.interaction == Interaction::NativeMenu {
                Driver::Menu(start(identity, spec, sink, online.clone()).await?)
            } else {
                Driver::Events(start(identity, spec, sink, online.clone()).await?)
            };
            Ok(Self {
                driver,
                online,
                sink: events,
            })
        })
    }
    pub(crate) fn apply(&self, revision: u64, spec: &Tray) -> Request<()> {
        match &self.driver {
            Driver::Menu(handle) => update(handle, revision, spec, &self.online, &self.sink),
            Driver::Events(handle) => update(handle, revision, spec, &self.online, &self.sink),
        }
    }
    pub(crate) fn show_menu(&self) -> Result<(), Error> {
        Err(Error::Unsupported(crate::Feature::ShowMenu))
    }
    pub(crate) fn registration(&self) -> Registration {
        let closed = match &self.driver {
            Driver::Menu(handle) => handle.is_closed(),
            Driver::Events(handle) => handle.is_closed(),
        };
        if !closed && self.online.load(Ordering::Acquire) {
            Registration::Registered
        } else {
            Registration::WaitingForHost
        }
    }
    pub(crate) fn anchor(&self) -> Result<Option<Anchor>, Error> {
        Ok(None)
    }
    pub(crate) fn popup_anchor(&self) -> Result<Option<crate::PopupAnchor>, Error> {
        Ok(None)
    }
    pub(crate) fn shutdown(&self) -> Request<()> {
        let shutdown = match &self.driver {
            Driver::Menu(handle) => handle.shutdown(),
            Driver::Events(handle) => handle.shutdown(),
        };
        Box::pin(async move {
            shutdown.await;
            Ok(())
        })
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}

#[cfg(all(feature = "sni-tests", unix))]
pub(super) async fn probe() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Mutex;
    use zbus::{
        Connection, Proxy,
        zvariant::{OwnedValue, Value},
    };
    struct Watcher(Arc<Mutex<Vec<String>>>);
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: &str) {
            self.0
                .lock()
                .expect("watcher mutex poisoned")
                .push(service.into());
        }
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }
    }
    let waiting_sink = crate::host::probe_sink("waiting");
    let pixel = crate::Image::new(crate::ImageRepresentation::rgba(1, 1, vec![255; 4])?);
    let waiting = Item::create(
        "dev.rake",
        &Tray::new("waiting").icon(pixel.clone()),
        waiting_sink.clone(),
    )
    .await?;
    if waiting.registration() != Registration::WaitingForHost {
        return Err("missing watcher was reported registered".into());
    }
    if crate::host::probe_next(&waiting_sink).await?.kind != EventKind::HostChanged(false) {
        return Err("missing watcher was not observable".into());
    }
    waiting.shutdown().await?;
    if Item::create(
        "dev.rake",
        &Tray::new("required")
            .icon(pixel)
            .host_policy(HostPolicy::Require),
        crate::host::probe_sink("required"),
    )
    .await
    .is_ok()
    {
        return Err("required registration succeeded without a watcher".into());
    }
    let registrations = Arc::new(Mutex::new(Vec::new()));
    let watcher = zbus::connection::Builder::session()?
        .name("org.kde.StatusNotifierWatcher")?
        .serve_at("/StatusNotifierWatcher", Watcher(registrations.clone()))?
        .build()
        .await?;
    let connection = Connection::session().await?;
    let sink = crate::host::probe_sink("sni-probe");
    let image = crate::Image::new(crate::ImageRepresentation::rgba(
        1,
        1,
        vec![10, 20, 30, 128],
    )?);
    let spec = Tray::new("probe")
        .icon(image.clone())
        .menu(Menu::new().item(crate::MenuItem::new("open", "Open & _ inspect")))
        .host_policy(HostPolicy::Require);
    let first = Item::create("dev.rake", &spec, sink.clone()).await?;
    let second = Item::create(
        "dev.rake",
        &Tray::new("other").icon(image),
        crate::host::probe_sink("sni-second"),
    )
    .await?;
    let names = registrations
        .lock()
        .expect("watcher mutex poisoned")
        .clone();
    if names.len() != 2 || names[0] == names[1] {
        return Err("SNI registrations are not independent".into());
    }
    let proxy = Proxy::new(
        &connection,
        names[0].as_str(),
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
    )
    .await?;
    let id: String = proxy.get_property("Id").await?;
    if id != "dev.rake.probe" {
        return Err("SNI identity mismatch".into());
    }
    let pixmap: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("IconPixmap").await?;
    if pixmap[0].2 != [128, 10, 20, 30] {
        return Err("SNI RGBA/ARGB conversion mismatch".into());
    }
    let path: zbus::zvariant::OwnedObjectPath = proxy.get_property("Menu").await?;
    let menu = Proxy::new(
        &connection,
        names[0].as_str(),
        path.as_str(),
        "com.canonical.dbusmenu",
    )
    .await?;
    type Layout = (
        u32,
        (
            i32,
            std::collections::HashMap<String, OwnedValue>,
            Vec<OwnedValue>,
        ),
    );
    let layout: Layout = menu
        .call("GetLayout", &(0i32, -1i32, Vec::<String>::new()))
        .await?;
    let child: zbus::zvariant::Structure = layout.1.2[0].try_clone()?.try_into()?;
    let native_id: i32 = child.fields()[0].downcast_ref()?;
    let _: () = menu
        .call("Event", &(native_id, "clicked", Value::from(0i32), 0u32))
        .await?;
    let selected = crate::host::probe_next(&sink).await?;
    if selected.kind != EventKind::MenuSelected("open".into()) {
        return Err("DBusMenu selection was not delivered".into());
    }
    let updated = spec.tooltip("Updated <plain> & tooltip");
    use futures::StreamExt as _;
    let mut title_changed = proxy.receive_signal("NewTitle").await?;
    first.apply(1, &updated).await?;
    title_changed
        .next()
        .await
        .ok_or("SNI NewTitle signal stream ended")?;
    // SNI uses its own NewTitle signal rather than PropertiesChanged; ordinary
    // zbus property caching does not invalidate itself for that protocol signal.
    let fresh = Proxy::new(
        &connection,
        names[0].as_str(),
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
    )
    .await?;
    let title: String = fresh.get_property("Title").await?;
    if title != "Updated <plain> & tooltip" {
        return Err("SNI update was not exported".into());
    }
    watcher
        .release_name("org.kde.StatusNotifierWatcher")
        .await?;
    if crate::host::probe_next(&sink).await?.kind != EventKind::HostChanged(false) {
        return Err("watcher loss was not observed".into());
    }
    watcher
        .request_name("org.kde.StatusNotifierWatcher")
        .await?;
    if crate::host::probe_next(&sink).await?.kind != EventKind::HostChanged(true) {
        return Err("watcher recovery was not observed".into());
    }
    first.shutdown().await?;
    second.shutdown().await?;
    watcher.close().await?;
    connection.close().await?;
    Ok(())
}

#[cfg(all(test, feature = "sni-tests", unix))]
mod tests {
    use super::*;
    use futures::FutureExt;

    #[test]
    fn repeated_stopped_updates_emit_host_loss_only_on_transition() {
        let online = AtomicBool::new(true);
        let sink = crate::host::probe_sink("host-loss");

        for _ in 0..3 {
            assert!(matches!(stopped(&online, &sink), Error::Stopped));
        }

        assert!(!online.load(Ordering::Acquire));
        let event = crate::host::probe_next(&sink)
            .now_or_never()
            .expect("host loss must be queued")
            .expect("event sink must remain open");
        assert_eq!(event.kind, EventKind::HostChanged(false));
        assert!(crate::host::probe_next(&sink).now_or_never().is_none());

        online.store(true, Ordering::Release);
        assert!(matches!(stopped(&online, &sink), Error::Stopped));
        let event = crate::host::probe_next(&sink)
            .now_or_never()
            .expect("a later host loss must be queued")
            .expect("event sink must remain open");
        assert_eq!(event.kind, EventKind::HostChanged(false));
        assert!(crate::host::probe_next(&sink).now_or_never().is_none());
    }
}
