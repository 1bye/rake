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
            .unwrap_or_else(|| self.spec.tooltip.clone())
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

fn update<const MENU: bool>(
    handle: &Rc<ksni::Handle<Model<MENU>>>,
    revision: u64,
    spec: &Tray,
) -> Request<()> {
    let handle = handle.clone();
    let spec = spec.clone();
    Box::pin(async move {
        let icons = icons(spec.menu.as_ref())?;
        handle
            .update(move |model| {
                if model.revision <= revision {
                    model.spec = spec;
                    model.revision = revision;
                    model.icons = icons;
                }
            })
            .await
            .ok_or(Error::Stopped)
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
            let driver = if spec.interaction == Interaction::NativeMenu {
                Driver::Menu(start(identity, spec, sink, online.clone()).await?)
            } else {
                Driver::Events(start(identity, spec, sink, online.clone()).await?)
            };
            Ok(Self { driver, online })
        })
    }
    pub(crate) fn apply(&self, revision: u64, spec: &Tray) -> Request<()> {
        match &self.driver {
            Driver::Menu(handle) => update(handle, revision, spec),
            Driver::Events(handle) => update(handle, revision, spec),
        }
    }
    pub(crate) fn show_menu(&self) -> Result<(), Error> {
        Err(Error::Unsupported(crate::Feature::ShowMenu))
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
