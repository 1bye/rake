//! Real native endpoint, private discovery, runtime dispatch, framing and cleanup.
#![cfg(any(unix, windows))]

use futures::{executor::LocalPool, task::LocalSpawnExt};
use rake::{
    control::{
        local::{self, LocalServer},
        *,
    },
    router::{NoRoute, Router, RouterConfig},
};
use std::{io, time::Duration};

#[test]
fn native_endpoint_round_trip_bounds_and_discovery_lifetime() {
    #[cfg(unix)]
    let base = std::path::PathBuf::from("/tmp");
    #[cfg(windows)]
    let base = std::env::temp_dir();
    let directory = base.join(format!(
        "rake-local-{}",
        &uuid::Uuid::new_v4().to_string()[..8]
    ));
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();
    let server = LocalServer::bind(&client, "endpoint-test", &directory).unwrap();
    let instances = local::discover(&directory).unwrap();
    assert_eq!(instances.len(), 1);
    let instance = instances[0].clone();
    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();
    let (stop, stopped) = futures::channel::oneshot::channel::<()>();
    pool.spawner()
        .spawn_local(async move {
            let _ = futures::future::select(Box::pin(server.run(client)), stopped).await;
        })
        .unwrap();

    let (done, finished) = futures::channel::oneshot::channel();
    let worker = std::thread::spawn(move || {
        let reply = rake_client::query(&instance, Query::Info {}, Duration::from_secs(3)).unwrap();
        assert!(matches!(reply, Reply::Info(_)));
        let rake_client::Endpoint::Local { path } = &instance.endpoint else {
            panic!("expected local endpoint")
        };
        let request = Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: instance.info.runtime_id,
            key: None,
            message: Message::Query(Query::Info {}),
        };
        assert_eq!(
            local::exchange(path, &request, Duration::ZERO)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );

        #[cfg(windows)]
        futures::executor::block_on(async {
            use futures::io::{AsyncReadExt, AsyncWriteExt};
            let mut stream = rake_client::pipe::Stream::connect(path).await.unwrap();
            // Send one byte beyond the frame limit, with no terminating newline.
            let work = async {
                let _ = stream.write_all(&vec![b'x'; MAX_MESSAGE_BYTES + 1]).await;
                let mut reply = Vec::new();
                let _ = stream.read_to_end(&mut reply).await;
                assert!(reply.is_empty(), "oversized request received a response");
            };
            assert!(
                matches!(
                    futures::future::select(
                        Box::pin(work),
                        Box::pin(async_io::Timer::after(Duration::from_secs(7)))
                    )
                    .await,
                    futures::future::Either::Left(_)
                ),
                "oversized frame did not close"
            );
        });

        done.send(()).unwrap();
    });
    pool.run_until(finished).unwrap();
    worker.join().unwrap();
    stop.send(()).unwrap();
    pool.run_until_stalled();
    assert!(local::discover(&directory).unwrap().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}
