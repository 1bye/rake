//! Typed application services with explicit composition and shared local ownership.
//!
//! Registries are immutable snapshots: adding a service returns a new registry.
//! Existing values retain their `Rc` identity across snapshots and routed views.
//! No service needs `Send` or `Sync`; applications own interior mutability.
//!
//! ```
//! use rake::services::ServiceRegistry;
//! use std::cell::Cell;
//! struct Counter(Cell<u64>);
//! let services = ServiceRegistry::new().with(Counter(Cell::new(0))).unwrap();
//! services.get::<Counter>().unwrap().0.set(4);
//! assert_eq!(services.clone().get::<Counter>().unwrap().0.get(), 4);
//! ```

use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::rc::Rc;

use crate::diagnostics::Collector;
use crate::http::HttpClient;

/// Missing or duplicate concrete service registration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServiceError {
    /// No value was registered for the named concrete Rust type.
    #[error("application service is not registered: {0}")]
    Missing(&'static str),
    /// Replacing a shared service implicitly is prohibited.
    #[error("application service is already registered: {0}")]
    Duplicate(&'static str),
}

/// One application composition. Clones share service values, not later registrations.
#[derive(Clone, Default)]
pub struct ServiceRegistry(Rc<HashMap<TypeId, Rc<dyn Any>>>);

impl ServiceRegistry {
    /// Creates an empty composition.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one concrete local service. Existing snapshots remain unchanged.
    pub fn with<T: 'static>(mut self, service: T) -> Result<Self, ServiceError> {
        let id = TypeId::of::<T>();

        if self.0.contains_key(&id) {
            return Err(ServiceError::Duplicate(type_name::<T>()));
        }

        Rc::make_mut(&mut self.0).insert(id, Rc::new(service));

        Ok(self)
    }

    /// Resolves the shared concrete value without copying the service itself.
    pub fn get<T: 'static>(&self) -> Result<Rc<T>, ServiceError> {
        self.0
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|value| value.downcast().ok())
            .ok_or(ServiceError::Missing(type_name::<T>()))
    }
}

/// Standard services and the application's typed composition.
#[derive(Clone)]
pub struct Services {
    http: HttpClient,
    collector: Collector,
    registry: ServiceRegistry,
}

impl Services {
    /// Constructs standard service handles with an explicit application composition.
    pub fn new(http: HttpClient, collector: Collector, registry: ServiceRegistry) -> Self {
        Self {
            http,
            collector,
            registry,
        }
    }

    /// Returns the shared configured HTTP transport handle.
    pub fn http(&self) -> HttpClient {
        self.http.clone()
    }

    /// Returns the bounded diagnostic collector.
    pub fn diagnostics(&self) -> Collector {
        self.collector.clone()
    }

    /// Resolves the same typed service used by command handlers and other views.
    pub fn get<T: 'static>(&self) -> Result<Rc<T>, ServiceError> {
        self.registry.get()
    }

    /// Captures this composition; shared service values retain their identity.
    pub fn registry(&self) -> ServiceRegistry {
        self.registry.clone()
    }
}
