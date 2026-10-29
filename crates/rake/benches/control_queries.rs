//! Measures targeted query cost with unrelated retained histories and managed work.
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::control::{Message, PROTOCOL_VERSION, Query, Request};
use rake::router::{OpenOptions, Router, RouterConfig};
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "document", path = "/documents/{id}")]
    Document(String),
}

fn main() {
    let router = Router::new(RouterConfig::default());
    let mut pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();

    for tab in 0..128 {
        let opened = pool
            .run_until(
                router
                    .navigator()
                    .open_with(
                        Destination::Document(format!("{tab}-{}", "x".repeat(512))),
                        OpenOptions::new_tab(),
                    )
                    .wait(),
            )
            .unwrap();

        let rake::router::Outcome::Committed(opened) = opened else {
            panic!("fixture open")
        };

        for entry in 0..4 {
            pool.run_until(
                router
                    .navigator()
                    .tab(opened.tab_id)
                    .navigate(Destination::Document(format!(
                        "{entry}-{}",
                        "x".repeat(512)
                    )))
                    .wait(),
            )
            .unwrap();
        }
    }

    for index in 0..256 {
        router
            .app()
            .tasks()
            .spawn(format!("task-{index}"), futures::future::pending())
            .unwrap();
    }

    let client = router.client();

    for (name, query) in [
        ("windows", Query::Windows {}),
        ("operations", Query::Operations {}),
        ("decisions", Query::Decisions {}),
        (
            "filtered tasks",
            Query::Tasks {
                owner: Some(rake::tasks::TaskOwner::Tab("tab-1".parse().unwrap())),
            },
        ),
    ] {
        let request = Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: router.runtime_id(),
            key: None,
            message: Message::Query(query),
        };

        let mut samples = Vec::new();

        for _ in 0..5 {
            let started = Instant::now();

            for _ in 0..100 {
                black_box(client.request(black_box(request.clone())).unwrap());
            }

            samples.push(started.elapsed().as_secs_f64() * 1e6 / 100.);
        }

        samples.sort_by(f64::total_cmp);
        println!("{name}: {:.1} us", samples[2]);
    }
}
