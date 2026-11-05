//! Weak control references and generation-checked application resource ownership.

use crate::{Capabilities, Error, Event, EventKind, Image, Menu, Request, Tray, backend};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
};

/// Native registration evidence, separate from actual pixel visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    /// Native creation/export is in progress.
    Creating,
    /// Registered with a currently available host.
    Registered,
    /// Exported while waiting for a compatible panel.
    WaitingForHost,
}

/// Snapshot of one live registration's requested and applied revisions.
#[derive(Clone, Debug)]
pub struct State {
    /// Stable item name.
    pub id: String,
    /// Unique registration generation.
    pub generation: u64,
    /// Latest admitted update.
    pub requested_revision: u64,
    /// Latest completed update.
    pub applied_revision: u64,
    /// Native registration evidence.
    pub registration: Registration,
    /// Application-requested visibility; the shell may still obscure it.
    pub requested_visible: bool,
}

/// Receipt identifying whether a later requested update superseded this one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Revision associated with this operation.
    pub revision: u64,
    /// A newer admitted revision exists.
    pub superseded: bool,
}

struct Entry {
    spec: Tray,
    state: State,
    native: Option<Rc<backend::Item>>,
}

struct Registry {
    application: String,
    generation: u64,
    alive: bool,
    entries: BTreeMap<String, Entry>,
    ingress: Arc<Ingress>,
}

pub(crate) struct Ingress {
    queue: Mutex<(VecDeque<Event>, Option<Event>, bool)>,
    changed: event_listener::Event,
}

impl Ingress {
    fn new() -> Self {
        Self {
            queue: Mutex::new((VecDeque::new(), None, false)),
            changed: event_listener::Event::new(),
        }
    }

    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "browser hosts cannot produce native callbacks")
    )]
    pub(crate) fn send(&self, mut event: Event) {
        let mut queue = self.queue.lock().expect("tray ingress mutex poisoned");

        if queue.2 {
            return;
        }
        if queue.0.len() == 256 {
            event.kind = EventKind::Overflow;
            queue.1.get_or_insert(event);
        } else {
            queue.0.push_back(event);
        }

        drop(queue);
        self.changed.notify(usize::MAX);
    }

    fn close(&self) {
        let mut queue = self.queue.lock().expect("tray ingress mutex poisoned");
        queue.2 = true;
        queue.0.clear();
        queue.1 = None;
        drop(queue);
        self.changed.notify(usize::MAX);
    }

    async fn next(&self) -> Result<Event, Error> {
        loop {
            let listener = self.changed.listen();

            {
                let mut queue = self.queue.lock().expect("tray ingress mutex poisoned");

                if queue.2 {
                    return Err(Error::Stopped);
                }
                if let Some(event) = queue.1.take().or_else(|| queue.0.pop_front()) {
                    return Ok(event);
                }
            }

            listener.await;
        }
    }
}

/// Native callback destination containing no UI context or application closure.
#[derive(Clone)]
#[cfg_attr(
    target_family = "wasm",
    expect(dead_code, reason = "browser hosts cannot produce native callbacks")
)]
pub(crate) struct Sink {
    pub(crate) id: String,
    pub(crate) generation: u64,
    ingress: Arc<Ingress>,
    menu_revision: Option<u64>,
}

impl Sink {
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "browser hosts cannot produce native menus")
    )]
    pub(crate) fn menu(&self, revision: u64) -> Self {
        Self {
            menu_revision: Some(revision),
            ..self.clone()
        }
    }

    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "browser hosts cannot produce native callbacks")
    )]
    pub(crate) fn send(&self, kind: EventKind, anchor: Option<crate::Anchor>, input: crate::Input) {
        self.ingress.send(Event {
            id: self.id.clone(),
            generation: self.generation,
            menu_revision: self.menu_revision,
            kind,
            anchor,
            input,
        });
    }
}

/// Unique local owner of all native items for an application host.
pub struct Host {
    registry: Rc<RefCell<Registry>>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("application", &self.registry.borrow().application)
            .finish_non_exhaustive()
    }
}

impl Host {
    /// Creates a resource owner on the native UI thread, without registering icons.
    /// # Errors
    /// Rejects an invalid application name or unavailable native backend/thread.
    pub fn new(application: &str) -> Result<Self, Error> {
        crate::identifier(application)?;
        backend::check_thread()?;

        Ok(Self {
            registry: Rc::new(RefCell::new(Registry {
                application: application.into(),
                generation: 0,
                alive: true,
                entries: BTreeMap::new(),
                ingress: Arc::new(Ingress::new()),
            })),
        })
    }

    /// Returns weak local control access; it does not retain this host.
    pub fn collection(&self) -> Collection {
        Collection {
            registry: Rc::downgrade(&self.registry),
        }
    }

    /// Stops admission immediately and returns an awaitable native cleanup request.
    pub fn shutdown(&self) -> Request<()> {
        let items = {
            let mut registry = self.registry.borrow_mut();
            registry.alive = false;
            registry.ingress.close();

            std::mem::take(&mut registry.entries)
                .into_values()
                .filter_map(|entry| entry.native)
                .collect::<Vec<_>>()
        };

        Box::pin(async move {
            for item in items {
                item.shutdown().await?;
            }

            Ok(())
        })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Admission stops synchronously even if the host cannot await native cleanup.
        drop(self.shutdown());
    }
}

/// Weak collection of application-owned native items.
#[derive(Clone)]
pub struct Collection {
    registry: Weak<RefCell<Registry>>,
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collection").finish_non_exhaustive()
    }
}

impl Collection {
    fn registry(&self) -> Result<Rc<RefCell<Registry>>, Error> {
        let registry = self.registry.upgrade().ok_or(Error::Stopped)?;

        if !registry.borrow().alive {
            return Err(Error::Stopped);
        }

        Ok(registry)
    }

    /// Queries backend operation support without creating a native item.
    pub fn capabilities(&self) -> Capabilities {
        backend::capabilities()
    }

    /// Returns a control reference for a live stable name.
    /// # Errors
    /// Reports a stopped owner or absent/removed item.
    pub fn get(&self, id: &str) -> Result<Handle, Error> {
        let registry = self.registry()?;
        let generation = registry
            .borrow()
            .entries
            .get(id)
            .ok_or(Error::Removed)?
            .state
            .generation;

        Ok(Handle {
            collection: self.clone(),
            id: id.into(),
            generation,
        })
    }

    /// Returns an owned snapshot of weak control references.
    /// # Errors
    /// Reports a stopped owner.
    pub fn items(&self) -> Result<Vec<Handle>, Error> {
        let registry = self.registry()?;
        let keys = registry
            .borrow()
            .entries
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        keys.iter().map(|id| self.get(id)).collect()
    }

    /// Reserves identity immediately and creates a native item when driven.
    /// Dropping a pending creation rolls back its reservation and native resources.
    pub fn create(&self, spec: Tray) -> Request<Handle> {
        let pending = (|| {
            spec.validate()?;
            backend::validate(&spec)?;
            let registry = self.registry()?;
            let mut state = registry.borrow_mut();

            if state.entries.contains_key(&spec.id) {
                return Err(Error::Duplicate(spec.id.clone()));
            }

            state.generation = state
                .generation
                .checked_add(1)
                .ok_or_else(|| Error::InvalidInput("registration generation exhausted".into()))?;
            let generation = state.generation;
            let sink = Sink {
                id: spec.id.clone(),
                generation,
                ingress: state.ingress.clone(),
                menu_revision: None,
            };
            let handle = Handle {
                collection: self.clone(),
                id: spec.id.clone(),
                generation,
            };
            let native = backend::create(&state.application, &spec, sink);
            let status = State {
                id: spec.id.clone(),
                generation,
                requested_revision: 0,
                applied_revision: 0,
                registration: Registration::Creating,
                requested_visible: spec.visible,
            };
            state.entries.insert(
                spec.id.clone(),
                Entry {
                    spec,
                    state: status,
                    native: None,
                },
            );

            Ok((handle, native))
        })();

        let (handle, native) = match pending {
            Ok(value) => value,
            Err(error) => return crate::ready(Err(error)),
        };
        let guard = Pending {
            handle: handle.clone(),
            committed: false,
        };

        Box::pin(async move {
            let mut guard = guard;
            let native = native.await?;
            let registry = handle.collection.registry()?;
            let mut registry = registry.borrow_mut();
            let entry = registry
                .entries
                .get_mut(&handle.id)
                .filter(|entry| entry.state.generation == handle.generation)
                .ok_or(Error::Removed)?;
            entry.state.registration = native.registration();
            entry.native = Some(Rc::new(native));
            guard.committed = true;

            Ok(handle)
        })
    }

    /// Waits for one validated event. Keep one application-owned consumer.
    /// # Errors
    /// Reports owner shutdown; native ingress overflow is a visible event.
    pub async fn next_event(&self) -> Result<Event, Error> {
        let ingress = self.registry()?.borrow().ingress.clone();

        loop {
            let event = ingress.next().await?;
            let registry = self.registry()?;
            let mut registry = registry.borrow_mut();
            let Some(entry) = registry
                .entries
                .get_mut(&event.id)
                .filter(|entry| entry.state.generation == event.generation)
            else {
                continue;
            };

            if event
                .menu_revision
                .is_some_and(|revision| revision != entry.state.applied_revision)
            {
                continue;
            }

            if let EventKind::MenuSelected(id) = &event.kind
                && !entry
                    .spec
                    .menu
                    .as_ref()
                    .is_some_and(|menu| menu.actionable(id))
            {
                continue;
            }
            if let EventKind::HostChanged(online) = event.kind {
                entry.state.registration = if online {
                    Registration::Registered
                } else {
                    Registration::WaitingForHost
                };
            }

            return Ok(event);
        }
    }
}

struct Pending {
    handle: Handle,
    committed: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.committed {
            drop(self.handle.remove());
        }
    }
}

/// Weak control reference to one registration generation.
#[derive(Clone, Debug)]
pub struct Handle {
    collection: Collection,
    id: String,
    generation: u64,
}

impl Handle {
    /// Stable application-scoped item name.
    pub fn id(&self) -> &str {
        &self.id
    }

    fn with<T>(&self, f: impl FnOnce(&mut Entry) -> Result<T, Error>) -> Result<T, Error> {
        let registry = self.collection.registry()?;
        let mut registry = registry.borrow_mut();
        let entry = registry
            .entries
            .get_mut(&self.id)
            .filter(|e| e.state.generation == self.generation)
            .ok_or(Error::Removed)?;

        f(entry)
    }

    /// Returns registration and revision evidence.
    /// # Errors
    /// Reports removed registrations or stopped hosts.
    pub fn state(&self) -> Result<State, Error> {
        self.with(|entry| Ok(entry.state.clone()))
    }

    /// Returns an owned snapshot of the latest admitted specification.
    /// # Errors
    /// Reports a removed registration or stopped host.
    pub fn specification(&self) -> Result<Tray, Error> {
        self.with(|entry| Ok(entry.spec.clone()))
    }

    /// Exercises AppKit target/action without synthesizing pointer input; native-test fixture only.
    /// # Errors
    /// Reports an absent native menu, invalid index or stopped/removed owner.
    #[cfg(all(feature = "native-tests", target_os = "macos"))]
    pub fn probe_menu_selection(&self, index: usize) -> Result<(), Error> {
        let native = self.with(|entry| entry.native.clone().ok_or(Error::HostUnavailable))?;
        native.probe_menu_selection(index)
    }

    /// Returns live AppKit metrics and native visibility.
    /// # Errors
    /// Reports unsupported platforms, removal or host shutdown.
    pub fn macos_metrics(&self) -> Result<crate::macos::Metrics, Error> {
        #[cfg(target_os = "macos")]
        {
            let native = self.with(|entry| entry.native.clone().ok_or(Error::HostUnavailable))?;
            Ok(native.metrics())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Error::Unsupported(crate::Feature::ItemLength))
        }
    }

    /// Explicitly opens the attached native menu when the backend permits it.
    /// # Errors
    /// Reports unsupported programmatic display, stopped/removed items or native failure.
    pub fn show_menu(&self) -> Result<(), Error> {
        let native = self.with(|entry| entry.native.clone().ok_or(Error::HostUnavailable))?;
        native.show_menu()
    }

    /// Replaces the complete specification without changing identity.
    pub fn update(&self, spec: Tray) -> Request<Applied> {
        let prepared = self.with(|entry| {
            if spec.id != self.id {
                return Err(Error::InvalidInput("updates cannot rename an item".into()));
            }

            spec.validate()?;
            backend::validate(&spec)?;
            if spec.interaction != entry.spec.interaction { return Err(Error::InvalidInput("primary interaction is fixed for a registration; recreate the item to change it".into())); }
            if spec.linux.as_ref().is_some_and(|l| l.sandboxed) != entry.spec.linux.as_ref().is_some_and(|l| l.sandboxed) {
                return Err(Error::InvalidInput("SNI bus-name policy is fixed for a registration".into()));
            }
            let native = entry.native.clone().ok_or(Error::HostUnavailable)?;
            let revision = entry
                .state
                .requested_revision
                .checked_add(1)
                .ok_or_else(|| Error::InvalidInput("tray revision exhausted".into()))?;
            let apply = native.apply(revision, &spec);
            entry.state.requested_revision = revision;
            entry.state.requested_visible = spec.visible;
            entry.spec = spec;

            Ok((revision, apply))
        });
        let (revision, apply) = match prepared {
            Ok(value) => value,
            Err(error) => return crate::ready(Err(error)),
        };
        let handle = self.clone();

        Box::pin(async move {
            apply.await?;
            handle.with(|entry| {
                entry.state.applied_revision = entry.state.applied_revision.max(revision);

                Ok(Applied {
                    revision,
                    superseded: entry.state.requested_revision > revision,
                })
            })
        })
    }

    fn change(&self, f: impl FnOnce(&mut Tray)) -> Request<Applied> {
        match self.with(|entry| Ok(entry.spec.clone())) {
            Ok(mut spec) => {
                f(&mut spec);
                self.update(spec)
            }
            Err(error) => crate::ready(Err(error)),
        }
    }

    /// Replaces the icon source.
    pub fn set_icon(&self, image: Image) -> Request<Applied> {
        self.change(|s| s.image = Some(image))
    }
    /// Replaces plain tooltip text.
    pub fn set_tooltip(&self, tooltip: &str) -> Request<Applied> {
        self.change(|s| s.tooltip = tooltip.into())
    }
    /// Replaces the native menu description.
    pub fn set_menu(&self, menu: Menu) -> Request<Applied> {
        self.change(|s| s.menu = Some(menu))
    }
    /// Requests presentation; user shell policy can still hide the item.
    pub fn show(&self) -> Request<Applied> {
        self.change(|s| s.visible = true)
    }
    /// Requests hiding and stops item-owned transient presentation.
    pub fn hide(&self) -> Request<Applied> {
        self.change(|s| s.visible = false)
    }

    /// Queries current geometry evidence without inventing an empty rectangle.
    /// # Errors
    /// Reports a removed item, stopped host or native query failure.
    pub fn anchor(&self) -> Result<Option<crate::Anchor>, Error> {
        self.with(|entry| {
            entry
                .native
                .as_ref()
                .ok_or(Error::HostUnavailable)?
                .anchor()
        })
    }

    /// Removes this registration immediately; completion awaits native cleanup.
    /// Repeated removal of this generation is harmless and cannot remove a replacement.
    pub fn remove(&self) -> Request<()> {
        let registry = match self.collection.registry() {
            Ok(value) => value,
            Err(error) => return crate::ready(Err(error)),
        };
        let native = {
            let mut registry = registry.borrow_mut();

            if !registry
                .entries
                .get(&self.id)
                .is_some_and(|e| e.state.generation == self.generation)
            {
                return crate::ready(Ok(()));
            }

            registry
                .entries
                .remove(&self.id)
                .and_then(|entry| entry.native)
        };

        match native {
            Some(native) => native.shutdown(),
            None => crate::ready(Ok(())),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Covers: stale native events, weak ownership and cancelled reservations.
    //! Real: registry/ingress and public weak control handles.
    //! Doubles: native resources are absent; no native display claims.
    //! Requires: no desktop session.
    use super::*;

    fn host() -> Host {
        Host {
            registry: Rc::new(RefCell::new(Registry {
                application: "dev.rake.test".into(),
                generation: 0,
                alive: true,
                entries: BTreeMap::new(),
                ingress: Arc::new(Ingress::new()),
            })),
        }
    }

    fn reserve(host: &Host, generation: u64) -> Sink {
        let mut registry = host.registry.borrow_mut();
        let spec = Tray::new("main");
        registry.entries.insert(
            "main".into(),
            Entry {
                spec,
                state: State {
                    id: "main".into(),
                    generation,
                    requested_revision: 0,
                    applied_revision: 0,
                    registration: Registration::Creating,
                    requested_visible: true,
                },
                native: None,
            },
        );

        Sink {
            id: "main".into(),
            generation,
            ingress: registry.ingress.clone(),
            menu_revision: None,
        }
    }

    #[test]
    fn weak_handles_do_not_extend_owner_lifetime() {
        let host = host();
        reserve(&host, 1);
        let collection = host.collection();
        let handle = collection.get("main").expect("reserved item exists");
        drop(host);
        assert!(matches!(handle.state(), Err(Error::Stopped)));
        assert!(matches!(collection.items(), Err(Error::Stopped)));
    }

    #[test]
    fn old_removal_and_events_cannot_target_recreated_item() {
        let host = host();
        let old_sink = reserve(&host, 1);
        let collection = host.collection();
        let old = collection.get("main").expect("first registration exists");
        futures::executor::block_on(old.remove()).expect("remove first registration");
        let new_sink = reserve(&host, 2);
        futures::executor::block_on(old.remove()).expect("old removal is harmless");
        assert_eq!(
            collection
                .get("main")
                .expect("replacement survives")
                .state()
                .expect("live state")
                .generation,
            2
        );
        old_sink.send(EventKind::Activate, None, crate::Input::default());
        new_sink.send(EventKind::SecondaryActivate, None, crate::Input::default());
        let event = futures::executor::block_on(collection.next_event()).expect("current event");
        assert_eq!(event.kind, EventKind::SecondaryActivate);
        assert_eq!(event.generation, 2);
    }

    #[test]
    fn cancelled_creation_releases_only_its_own_reservation() {
        let host = host();
        reserve(&host, 1);
        let collection = host.collection();
        let pending = Pending {
            handle: collection.get("main").expect("pending reservation"),
            committed: false,
        };
        reserve(&host, 2);
        drop(pending);
        assert_eq!(collection.items().expect("owner is live").len(), 1);
    }

    #[test]
    fn bounded_ingress_reports_overflow_and_shutdown_wakes_waiters() {
        let host = host();
        let sink = reserve(&host, 1);

        for _ in 0..300 {
            sink.send(EventKind::Activate, None, crate::Input::default());
        }

        let collection = host.collection();
        assert_eq!(
            futures::executor::block_on(collection.next_event())
                .expect("overflow event")
                .kind,
            EventKind::Overflow
        );
        drop(host.shutdown());
        assert!(matches!(
            futures::executor::block_on(collection.next_event()),
            Err(Error::Stopped)
        ));
    }

    #[test]
    fn old_menu_projection_cannot_dispatch_a_rebound_item() {
        let host = host();
        let sink = reserve(&host, 1);
        {
            let mut registry = host.registry.borrow_mut();
            let entry = registry
                .entries
                .get_mut("main")
                .expect("registration exists");
            entry.spec =
                Tray::new("main").menu(Menu::new().item(crate::MenuItem::new("open", "Open")));
            entry.state.applied_revision = 1;
        }
        sink.menu(0).send(
            EventKind::MenuSelected("open".into()),
            None,
            crate::Input::default(),
        );
        sink.menu(1).send(
            EventKind::MenuSelected("open".into()),
            None,
            crate::Input::default(),
        );
        let event = futures::executor::block_on(host.collection().next_event())
            .expect("current projection selection");
        assert_eq!(event.menu_revision, Some(1));
    }
}
