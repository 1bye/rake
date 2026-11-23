//! Shared application services with a GPUI platform handle.

use crate::{
    diagnostics::Collector,
    http::{HttpClient, HttpConfig},
    platform::Platform,
};

pub use rake_core::services::ServiceError;
use std::rc::Rc;

/// One application composition. Clones share service values and standard handles.
#[derive(Clone, Default)]
pub struct ServiceRegistry(rake_core::services::ServiceRegistry);

impl ServiceRegistry {
    /// Creates an empty composition.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one concrete service to this snapshot.
    pub fn with<T: 'static>(self, service: T) -> Result<Self, ServiceError> {
        self.0.with(service).map(Self)
    }

    /// Resolves the shared concrete value without copying it.
    pub fn get<T: 'static>(&self) -> Result<Rc<T>, ServiceError> {
        self.0.get()
    }

    /// Returns the standard application HTTP handle, when present.
    pub fn http(&self) -> Result<HttpClient, ServiceError> {
        self.0.http()
    }

    /// Returns the standard application diagnostic collector, when present.
    pub fn diagnostics(&self) -> Result<Collector, ServiceError> {
        self.0.diagnostics()
    }

    /// Returns shared host services, if this registry came from [`Services`].
    pub fn platform(&self) -> Result<Platform, ServiceError> {
        self.0.platform().map(Platform::from_core)
    }

    /// Reconstructs the standard service view represented by this registry.
    pub fn standard_services(&self) -> Result<Services, ServiceError> {
        self.0.standard_services().map(Services::from_core)
    }

    pub(crate) fn from_core(core: rake_core::services::ServiceRegistry) -> Self {
        Self(core)
    }

    pub(crate) fn into_core(self) -> rake_core::services::ServiceRegistry {
        self.0
    }
}

/// Standard services and the application's typed composition.
#[derive(Clone)]
pub struct Services(rake_core::services::Services);

impl Services {
    /// Constructs standard service handles with an explicit application composition.
    pub fn new(http: HttpClient, collector: Collector, registry: ServiceRegistry) -> Self {
        Self::with_platform(http, collector, registry, Platform::default())
    }

    pub(crate) fn with_platform(
        http: HttpClient,
        collector: Collector,
        registry: ServiceRegistry,
        platform: Platform,
    ) -> Self {
        Self(rake_core::services::Services::with_platform(
            http,
            collector,
            registry.into_core(),
            platform.into_core(),
        ))
    }

    /// Builds the HTTP service and installs it with the supplied collector.
    /// # Errors
    /// Returns the HTTP configuration error when the client cannot be constructed.
    pub fn from_config(
        config: HttpConfig,
        collector: Collector,
        registry: ServiceRegistry,
    ) -> Result<Self, crate::http::Error> {
        let http = HttpClient::with_collector(config, collector.clone())?;

        Ok(Self::new(http, collector, registry))
    }

    /// Returns shared platform services bound to the GPUI host.
    pub fn platform(&self) -> Platform {
        Platform::from_core(self.0.platform())
    }

    pub(crate) fn from_core(services: rake_core::services::Services) -> Self {
        Self(services)
    }

    pub(crate) fn into_core(self) -> rake_core::services::Services {
        self.0
    }

    /// Returns the shared configured HTTP transport handle.
    pub fn http(&self) -> HttpClient {
        self.0.http()
    }

    /// Returns the bounded diagnostic collector.
    pub fn diagnostics(&self) -> Collector {
        self.0.diagnostics()
    }

    /// Resolves the shared concrete service.
    pub fn get<T: 'static>(&self) -> Result<Rc<T>, ServiceError> {
        self.0.get()
    }

    /// Captures this composition without changing shared value identities.
    pub fn registry(&self) -> ServiceRegistry {
        ServiceRegistry::from_core(self.0.registry())
    }
}
