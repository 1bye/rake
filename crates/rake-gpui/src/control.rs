//! Control transport startup for managed applications.
pub use rake_core::control::*;

mod facade;

#[cfg(target_family = "wasm")]
pub use facade::WebBridge;
#[cfg(any(unix, windows))]
pub use facade::local;
pub use facade::{CommandContext, Commands, RuntimeClient};

use crate::router::Route;

#[cfg(any(feature = "devtools", test))]
pub(crate) fn inspection_enabled(config: ControlConfig) -> bool {
    #[cfg(not(feature = "devtools"))]
    {
        let _ = config;

        false
    }

    #[cfg(feature = "devtools")]
    {
        if !matches!(config, ControlConfig::FromEnvironment) {
            return false;
        }

        #[cfg(any(unix, windows))]
        {
            std::env::var("RAKE_DEV_CONTROL").as_deref() == Ok("1")
        }
        #[cfg(target_family = "wasm")]
        {
            WebBridgeConfig::from_document().is_ok_and(|config| config.is_some())
        }

        #[cfg(not(any(unix, windows, target_family = "wasm")))]
        {
            false
        }
    }
}

pub(crate) async fn run_host<R: Route>(
    config: ControlConfig,
    client: RuntimeClient<R>,
    application_id: String,
) {
    #[cfg(any(unix, windows, target_family = "wasm"))]
    run_host_client(config, client.into_host(), application_id).await;

    #[cfg(not(any(unix, windows, target_family = "wasm")))]
    {
        let _ = (config, client, application_id);

        futures::future::pending::<()>().await;
    }
}

#[cfg(any(unix, windows, target_family = "wasm"))]
pub(crate) async fn run_host_client(
    config: ControlConfig,
    client: HostClient,
    application_id: String,
) {
    if matches!(config, ControlConfig::FromEnvironment) {
        #[cfg(any(unix, windows))]
        if std::env::var("RAKE_DEV_CONTROL").as_deref() == Ok("1") {
            match local::LocalServer::bind_host(&client, &application_id, &local::directory()) {
                Ok(server) => {
                    if let Err(error) = server.run_host(client).await {
                        crate::error!(
                            error_type = std::any::type_name_of_val(&error),
                            "Development control stopped"
                        );
                    }
                }
                Err(error) => crate::error!(
                    error_type = std::any::type_name_of_val(&error),
                    "Development control could not start"
                ),
            }
        }

        #[cfg(target_family = "wasm")]
        match WebBridgeConfig::from_document() {
            Ok(Some(config)) => match WebBridge::new(config) {
                Ok(bridge) => bridge.run_host(client, &application_id).await,
                Err(error) => crate::error!(
                    error_type = std::any::type_name_of_val(&error),
                    "Development bridge could not start"
                ),
            },
            Ok(None) => {}
            Err(error) => crate::error!(
                error_type = std::any::type_name_of_val(&error),
                "Invalid development bridge configuration"
            ),
        }
    }

    futures::future::pending::<()>().await;
}
