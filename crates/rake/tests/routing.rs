//! Covers: route codecs, independent tabs, eager operations, guards and history journaling.
//! Real: local router scheduler, retained entries, generated codec and guard decisions.
//! Doubles: browser History sink; GPUI resources are exercised separately.
//! Requires: no GPU, browser, network or external services.

use std::cell::RefCell;
use std::rc::Rc;

use futures::executor::{LocalPool, block_on};
use futures::task::LocalSpawnExt;
use rake::prelude::*;
use rake::router::*;

#[derive(Clone, Debug, PartialEq, Eq, rake::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(String),
    #[route(id = "search", path = "/search/{page}")]
    Search {
        page: u64,
        query: String,
        tag: Option<String>,
    },
    #[route(id = "new-document", path = "/documents/new")]
    NewDocument,
}

fn harness(config: RouterConfig) -> (Router<Destination>, LocalPool) {
    let router = Router::new(config);
    let pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.expect("driver");
        })
        .expect("spawn");

    (router, pool)
}

fn committed<T: Clone>(pool: &mut LocalPool, operation: Operation<T>) -> T {
    match pool.run_until(operation.wait()).expect("operation") {
        Outcome::Committed(value) => value,
        _ => panic!("expected a committed transition"),
    }
}

fn open(router: &Router<Destination>, pool: &mut LocalPool, id: &str) -> Opened {
    committed(
        pool,
        router
            .navigator()
            .open(Destination::Document(id.into()), OpenTarget::NewTab),
    )
}

fn decision(router: &Router<Destination>, pool: &mut LocalPool) -> DecisionSnapshot {
    pool.run_until_stalled();
    let decisions = router.snapshot().decisions;

    assert_eq!(decisions.len(), 1);

    decisions[0].clone()
}

#[test]
fn route_round_trips_percent_encoding_queries_and_stable_keys() {
    let document = Destination::Document("a/b + café%?#".into());
    let uri = document.to_uri().expect("URI");

    assert_eq!(Destination::from_uri(&uri).expect("decode"), document);
    assert!(uri.contains("%2F"));
    assert_eq!(
        Destination::from_uri("/documents/new").expect("static precedence"),
        Destination::NewDocument
    );
    assert_eq!(Destination::Home.encode().route, "home");

    let search = Destination::Search {
        page: 7,
        query: "a b&c".into(),
        tag: Some("α/β".into()),
    };

    assert_eq!(
        Destination::decode(&search.encode()).expect("address"),
        search
    );
    assert_eq!(
        Destination::from_uri(&search.to_uri().expect("URI")).expect("decode"),
        search
    );
    assert_eq!(
        Destination::from_uri("/search/007?query=hello").expect("typed"),
        Destination::Search {
            page: 7,
            query: "hello".into(),
            tag: None
        }
    );
}

#[test]
fn malformed_missing_unknown_and_duplicate_parameters_are_rejected() {
    for uri in [
        "/documents/%",
        "/documents/%FF",
        "/documents/",
        "/documents/..",
        "//documents/a",
        "/documents/a?extra=x",
        "/search/1",
        "/search/no?query=x",
        "/search/1?query=x&query=y",
        "/search/1?page=2&query=x",
    ] {
        assert!(Destination::from_uri(uri).is_err(), "accepted {uri}");
    }

    let mut address = Destination::Document("a".into()).encode();

    address.params.clear();

    assert!(matches!(
        Destination::decode(&address),
        Err(RouteDecodeError::MissingParameter(_))
    ));
    assert!(Destination::Document(".".into()).to_uri().is_err());
    assert!(
        Destination::Document("new".into()).to_uri().is_err(),
        "a typed destination must not encode as a different static route"
    );
}

#[test]
fn independent_tabs_retain_histories_and_open_never_deduplicates() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let entry = router.entry(a.entry_id).expect("entry");

    entry.set_dirty(true).expect("edit A");
    let second = committed(
        &mut pool,
        router.navigator().tab(a.tab_id).navigate(Destination::Home),
    );

    let b = open(&router, &mut pool, "B");
    let duplicate = open(&router, &mut pool, "A");

    committed(&mut pool, router.navigator().tab(a.tab_id).select());
    let snapshot = router.snapshot();

    assert_eq!(snapshot.tabs.len(), 3);
    assert_ne!(a.tab_id, duplicate.tab_id);
    assert_ne!(a.entry_id, duplicate.entry_id);
    assert_eq!(snapshot.selected, Some(a.tab_id));
    assert_eq!(snapshot.tabs[0].entries[0].id, a.entry_id);
    assert!(snapshot.tabs[0].entries[0].dirty);
    assert_eq!(snapshot.tabs[0].entries[1].id, second.entry_id);
    assert_eq!(snapshot.tabs[1].entries[0].id, b.entry_id);
    assert!(snapshot.decisions.is_empty());
}

#[test]
fn back_and_forward_reuse_instances_and_truncation_is_guarded() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let tab = router.navigator().tab(a.tab_id);
    let b = committed(&mut pool, tab.navigate(Destination::Document("B".into())));
    let b_entry = router.entry(b.entry_id).expect("B");
    let cancellation = b_entry.cancellation().expect("lifetime");

    b_entry.set_dirty(true).expect("edit B");

    assert_eq!(committed(&mut pool, tab.back()).entry_id, a.entry_id);
    assert_eq!(committed(&mut pool, tab.forward()).entry_id, b.entry_id);
    committed(&mut pool, tab.back());

    let push = tab.navigate(Destination::Home);
    let ask = decision(&router, &mut pool);

    router
        .navigator()
        .respond(ask.id, "cancel")
        .expect("cancel");

    assert_eq!(
        pool.run_until(push.wait()).expect("outcome"),
        Outcome::Cancelled(CancelReason::Decision)
    );
    assert_eq!(router.snapshot().tabs[0].entries.len(), 2);
    assert!(!cancellation.is_cancelled());

    let push = tab.navigate(Destination::Home);
    let ask = decision(&router, &mut pool);

    router
        .navigator()
        .respond(ask.id, "discard")
        .expect("discard");
    let home = committed(&mut pool, push);

    assert_ne!(home.entry_id, a.entry_id);
    assert!(router.entry(b.entry_id).is_err());
    assert!(cancellation.is_cancelled());
    assert_eq!(router.snapshot().tabs[0].entries.len(), 2);
}

#[test]
fn selecting_another_tab_does_not_expire_a_guarded_close() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let b = open(&router, &mut pool, "B");

    router
        .entry(a.entry_id)
        .expect("A")
        .set_dirty(true)
        .expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let ask = decision(&router, &mut pool);

    committed(&mut pool, router.navigator().tab(b.tab_id).select());
    committed(
        &mut pool,
        router.navigator().tab(b.tab_id).navigate(Destination::Home),
    );
    router
        .navigator()
        .respond(ask.id, "discard")
        .expect("original decision remains valid");
    committed(&mut pool, close);

    assert_eq!(router.snapshot().tabs.len(), 1);
    assert_eq!(router.snapshot().tabs[0].id, b.tab_id);
    assert_eq!(router.snapshot().tabs[0].entries.len(), 2);
}

#[test]
fn dirty_edits_expire_decisions_instead_of_authorizing_new_state() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let entry = router.entry(a.entry_id).expect("entry");

    entry.set_dirty(true).expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let old = decision(&router, &mut pool);

    entry
        .set_dirty(true)
        .expect("another edit while already dirty");

    assert_eq!(
        router.navigator().respond(old.id, "discard"),
        Err(DecisionError::Obsolete)
    );
    let new = decision(&router, &mut pool);

    assert_ne!(old.id, new.id);
    assert_eq!(
        router.navigator().respond(new.id, "unknown"),
        Err(DecisionError::UnknownChoice)
    );
    router
        .navigator()
        .respond(new.id, "cancel")
        .expect("cancel current");

    assert_eq!(
        pool.run_until(close.wait()).expect("outcome"),
        Outcome::Cancelled(CancelReason::Decision)
    );
    assert_eq!(router.snapshot().tabs.len(), 1);
}

#[test]
fn save_preparation_revalidates_dirty_state_before_disposal() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let entry = router.entry(a.entry_id).expect("entry");
    let saved = Rc::new(RefCell::new(0));
    let handle = entry.clone();
    let writes = saved.clone();

    entry
        .on_dispose(move |context| {
            let handle = handle.clone();
            let writes = writes.clone();

            async move {
                if !context.dirty {
                    return GuardResult::Allow;
                }

                GuardResult::Ask(DecisionRequest {
                    message: "Save changes?".into(),
                    choices: vec![
                        DecisionChoice::proceed("save", "Save").prepare(move || {
                            let handle = handle.clone();
                            let writes = writes.clone();

                            async move {
                                *writes.borrow_mut() += 1;
                                handle
                                    .set_dirty(false)
                                    .map_err(|error| GuardReason(error.to_string()))
                            }
                        }),
                        DecisionChoice::cancel("cancel", "Cancel"),
                    ],
                })
            }
        })
        .expect("guard");
    entry.set_dirty(true).expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let ask = decision(&router, &mut pool);

    router.navigator().respond(ask.id, "save").expect("save");
    committed(&mut pool, close);

    assert_eq!(*saved.borrow(), 1);
    assert!(router.snapshot().tabs.is_empty());
}

#[test]
fn earlier_approvals_are_rechecked_after_a_later_guard_waits() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");

    let b = committed(
        &mut pool,
        router
            .navigator()
            .tab(a.tab_id)
            .navigate(Destination::Document("B".into())),
    );

    let first = router.entry(a.entry_id).expect("A");

    first.set_dirty(true).expect("edit A");
    router
        .entry(b.entry_id)
        .expect("B")
        .set_dirty(true)
        .expect("edit B");
    let close = router.navigator().tab(a.tab_id).close();
    let first_ask = decision(&router, &mut pool);

    router
        .navigator()
        .respond(first_ask.id, "discard")
        .expect("approve A");
    let second_ask = decision(&router, &mut pool);

    assert_eq!(second_ask.entry_id, b.entry_id);
    first.set_dirty(true).expect("edit A after approval");
    router
        .navigator()
        .respond(second_ask.id, "discard")
        .expect("approve B");
    let recheck = decision(&router, &mut pool);

    assert_eq!(recheck.entry_id, a.entry_id);
    router
        .navigator()
        .respond(recheck.id, "cancel")
        .expect("cancel");
    assert!(matches!(
        pool.run_until(close.wait()).expect("outcome"),
        Outcome::Cancelled(_)
    ));
    assert_eq!(router.snapshot().tabs[0].entries.len(), 2);
}

#[test]
fn operations_are_eager_and_dropping_observers_does_not_cancel() {
    let (router, mut pool) = harness(RouterConfig::default());

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);

    let id = operation.id();

    assert_eq!(
        router.operation(id).expect("queued"),
        OperationState::Queued
    );
    drop(operation);
    pool.run_until_stalled();

    assert_eq!(
        router.operation(id).expect("committed"),
        OperationState::Committed
    );
    assert_eq!(router.snapshot().tabs.len(), 1);
}

#[test]
fn current_tab_is_fixed_before_async_access_guards() {
    let (router, mut pool) = harness(RouterConfig::default());

    router
        .entry_guard(|context| async move {
            if context.destination == Destination::Home {
                GuardResult::Ask(DecisionRequest {
                    message: "Enter home?".into(),
                    choices: vec![DecisionChoice::proceed("allow", "Allow")],
                })
            } else {
                GuardResult::Allow
            }
        })
        .expect("access guard");
    let a = open(&router, &mut pool, "A");
    let b = open(&router, &mut pool, "B");

    committed(&mut pool, router.navigator().tab(a.tab_id).select());
    let navigate = router
        .navigator()
        .open(Destination::Home, OpenTarget::CurrentTab);

    let ask = decision(&router, &mut pool);

    committed(&mut pool, router.navigator().tab(b.tab_id).select());
    router.navigator().respond(ask.id, "allow").expect("allow");
    let entered = committed(&mut pool, navigate);

    assert_eq!(entered.tab_id, a.tab_id);
    assert_eq!(router.snapshot().selected, Some(b.tab_id));
    assert_eq!(router.snapshot().tabs[0].entries.len(), 2);
    assert_eq!(router.snapshot().tabs[1].entries.len(), 1);
}

#[test]
fn concurrent_reveals_reuse_only_matching_current_entries() {
    let (router, mut pool) = harness(RouterConfig::default());
    let navigator = router.navigator();
    let first = navigator.reveal(Destination::Home, RevealScope::Window);
    let second = navigator.reveal(Destination::Home, RevealScope::Application);
    let a = committed(&mut pool, first);
    let b = committed(&mut pool, second);

    assert_eq!(a.opened.tab_id, b.opened.tab_id);
    assert!(!a.reused);
    assert!(b.reused);
    assert_eq!(router.snapshot().tabs.len(), 1);

    committed(
        &mut pool,
        navigator
            .tab(a.opened.tab_id)
            .navigate(Destination::Document("A".into())),
    );
    let new = committed(
        &mut pool,
        navigator.reveal(Destination::Home, RevealScope::Window),
    );

    assert!(!new.reused);
    assert_ne!(new.opened.tab_id, a.opened.tab_id);
}

#[test]
fn retention_limits_block_without_silent_eviction() {
    let (router, mut pool) = harness(RouterConfig {
        history_capacity: Some(2),
        ..RouterConfig::default()
    });

    let a = open(&router, &mut pool, "A");
    let tab = router.navigator().tab(a.tab_id);

    committed(&mut pool, tab.navigate(Destination::Home));
    let blocked = pool
        .run_until(tab.navigate(Destination::NewDocument).wait())
        .expect("outcome");

    assert_eq!(blocked, Outcome::Blocked(BlockReason::HistoryCapacity));
    assert_eq!(router.snapshot().tabs[0].entries[0].id, a.entry_id);
    committed(&mut pool, tab.back());
    committed(&mut pool, tab.navigate(Destination::NewDocument));
    assert_eq!(router.snapshot().tabs[0].entries.len(), 2);
}

#[test]
fn explicit_modes_invalid_options_and_unsupported_targets_are_distinct() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");

    let b = committed(
        &mut pool,
        router.navigator().open_with(
            Destination::Home,
            OpenOptions::new_tab()
                .activation(Activation::PreserveSelection)
                .mode(TabMode::Live),
        ),
    );

    assert_eq!(router.snapshot().selected, Some(a.tab_id));
    assert_eq!(router.snapshot().tabs[0].mode, TabMode::Foreground);
    assert_eq!(router.snapshot().tabs[1].mode, TabMode::Live);
    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .open_with(
                    Destination::Home,
                    OpenOptions {
                        target: OpenTarget::CurrentTab,
                        activation: Activation::Activate,
                        mode: Some(TabMode::Live)
                    }
                )
                .wait()
        ),
        Err(OperationError::InvalidOptions)
    ));
    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .open(Destination::Home, OpenTarget::NewWindow)
                .wait()
        ),
        Err(OperationError::UnsupportedCapability(OpenTarget::NewWindow))
    ));
    committed(
        &mut pool,
        router
            .navigator()
            .tab(b.tab_id)
            .set_mode(TabMode::Foreground),
    );
    assert_eq!(router.snapshot().tabs[1].mode, TabMode::Foreground);
}

#[test]
fn queued_and_waiting_operations_can_be_cancelled_explicitly() {
    let (router, mut pool) = harness(RouterConfig::default());

    let queued = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);

    assert!(router.cancel(queued.id()).expect("cancel queued"));
    assert_eq!(
        pool.run_until(queued.wait()).expect("outcome"),
        Outcome::Cancelled(CancelReason::Requested)
    );
    assert!(router.snapshot().tabs.is_empty());

    let a = open(&router, &mut pool, "A");

    router
        .entry(a.entry_id)
        .expect("entry")
        .set_dirty(true)
        .expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let id = close.id();
    let ask = decision(&router, &mut pool);

    assert!(router.cancel(id).expect("cancel waiting"));
    assert_eq!(
        pool.run_until(close.wait()).expect("outcome"),
        Outcome::Cancelled(CancelReason::Requested)
    );
    assert_eq!(
        router.navigator().respond(ask.id, "discard"),
        Err(DecisionError::Obsolete)
    );
    assert!(!router.cancel(id).expect("too late"));
}

#[test]
fn completed_retention_and_admission_are_bounded() {
    let (router, mut pool) = harness(RouterConfig {
        completed_operations: 1,
        pending_operations: 1,
        ..RouterConfig::default()
    });

    let first = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);

    let rejected = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);

    assert_eq!(
        pool.run_until(rejected.wait()),
        Err(OperationError::QueueFull)
    );
    let id = first.id();

    committed(&mut pool, first);
    committed(
        &mut pool,
        router
            .navigator()
            .open(Destination::Home, OpenTarget::NewTab),
    );

    assert_eq!(router.operation(id), Err(OperationError::OperationExpired));
    assert_eq!(
        router.operation("op-999999".parse().expect("id")),
        Err(OperationError::UnknownOperation)
    );
}

#[test]
fn unstarted_runtime_drops_pending_work_without_an_rc_cycle() {
    let router = Router::<Destination>::new(RouterConfig::default());

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);

    drop(router);

    assert_eq!(
        block_on(operation.wait()),
        Err(OperationError::RuntimeShutdown)
    );
}

#[test]
fn browser_visits_and_tab_histories_have_different_scopes() {
    let (router, mut pool) = harness(RouterConfig::default());
    let journal = Rc::new(RefCell::new(Vec::new()));
    let sink = journal.clone();

    router.on_history(move |change| {
        sink.borrow_mut().push(change);

        Ok(())
    });
    let a = open(&router, &mut pool, "A");

    let checkpoint = match &journal.borrow()[0] {
        HistoryChange::Replace(checkpoint, _) => checkpoint.clone(),
        _ => panic!("initial replace"),
    };

    let b = committed(
        &mut pool,
        router.navigator().open_with(
            Destination::Document("B".into()),
            OpenOptions::new_tab().activation(Activation::PreserveSelection),
        ),
    );

    committed(
        &mut pool,
        router.navigator().tab(b.tab_id).navigate(Destination::Home),
    );

    assert_eq!(
        journal.borrow().len(),
        1,
        "hidden mutations do not affect the URL"
    );
    committed(&mut pool, router.navigator().tab(b.tab_id).select());
    assert!(matches!(
        journal.borrow().last(),
        Some(HistoryChange::Push(..))
    ));
    let before = journal.borrow().len();

    committed(&mut pool, router.navigator().tab(b.tab_id).select());
    assert_eq!(
        journal.borrow().len(),
        before,
        "same selection adds no visit"
    );
    let restored = committed(&mut pool, router.navigator().restore(checkpoint));

    assert_eq!(restored.opened.entry_id, a.entry_id);
    assert!(restored.reused);
    assert_eq!(router.snapshot().tabs.len(), 2);
    assert_eq!(router.snapshot().tabs[1].entries.len(), 2);
    assert!(
        matches!(journal.borrow().last(), Some(HistoryChange::Replace(..))),
        "traversal cannot push a feedback visit"
    );
}

#[test]
fn stale_browser_hints_reconstruct_destinations_and_never_close_other_tabs() {
    let (first, mut first_pool) = harness(RouterConfig::default());
    let a = open(&first, &mut first_pool, "A");

    let checkpoint = BrowserCheckpoint {
        version: 1,
        runtime_id: first.snapshot().runtime_id,
        tab_id: a.tab_id,
        entry_id: a.entry_id,
        destination: Destination::Document("A".into()).encode(),
    };

    let (second, mut pool) = harness(RouterConfig::default());
    let b = open(&second, &mut pool, "B");
    let restored = committed(&mut pool, second.navigator().restore(checkpoint));

    assert_ne!(first.snapshot().runtime_id, second.snapshot().runtime_id);
    assert!(!restored.reused);
    assert_ne!(restored.opened.tab_id, b.tab_id);
    assert_eq!(second.snapshot().tabs.len(), 2);
    assert_eq!(
        second.snapshot().tabs[1].entries[0].destination,
        Destination::Document("A".into()).encode()
    );
}

#[test]
fn newer_browser_traversals_expire_older_pending_decisions() {
    let (router, mut pool) = harness(RouterConfig::default());

    router
        .entry_guard(|context| async move {
            if context.destination == Destination::Home {
                GuardResult::Ask(DecisionRequest {
                    message: "Enter?".into(),
                    choices: vec![DecisionChoice::proceed("allow", "Allow")],
                })
            } else {
                GuardResult::Allow
            }
        })
        .expect("guard");
    open(&router, &mut pool, "A");
    let older = router.navigator().restore_destination(Destination::Home);
    let obsolete = decision(&router, &mut pool);

    let newer = router
        .navigator()
        .restore_destination(Destination::Document("B".into()));

    committed(&mut pool, newer);

    assert_eq!(
        pool.run_until(older.wait()).expect("older"),
        Outcome::Cancelled(CancelReason::Superseded)
    );
    assert_eq!(
        router.navigator().respond(obsolete.id, "allow"),
        Err(DecisionError::Obsolete)
    );
    assert_eq!(router.snapshot().tabs.len(), 2);
    assert!(router.snapshot().decisions.is_empty());
}

#[test]
fn denied_browser_navigation_replaces_the_url_with_the_visible_destination() {
    let (router, mut pool) = harness(RouterConfig::default());

    router
        .entry_guard(|context| async move {
            if context.destination == Destination::Home {
                GuardResult::Deny(GuardReason("access denied".into()))
            } else {
                GuardResult::Allow
            }
        })
        .expect("guard");
    let changes = Rc::new(RefCell::new(Vec::new()));
    let sink = changes.clone();

    router.on_history(move |change| {
        sink.borrow_mut().push(change);

        Ok(())
    });
    let a = open(&router, &mut pool, "A");

    let outcome = pool
        .run_until(
            router
                .navigator()
                .restore_destination(Destination::Home)
                .wait(),
        )
        .expect("outcome");

    assert!(matches!(outcome, Outcome::Blocked(BlockReason::Guard(_))));
    assert_eq!(router.snapshot().selected, Some(a.tab_id));
    assert!(
        matches!(changes.borrow().last(), Some(HistoryChange::Replace(_, uri)) if uri == "/documents/A")
    );
    assert_eq!(router.snapshot().tabs.len(), 1);
}

#[test]
fn edits_during_async_save_require_a_new_decision() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let entry = router.entry(a.entry_id).expect("entry");
    let (finish_save, saved) = futures::channel::oneshot::channel();
    let wait = Rc::new(RefCell::new(Some(saved)));

    entry
        .on_dispose(move |context| {
            let wait = wait.clone();

            async move {
                if !context.dirty {
                    return GuardResult::Allow;
                }

                GuardResult::Ask(DecisionRequest {
                    message: "Save?".into(),
                    choices: vec![
                        DecisionChoice::proceed("save", "Save").prepare(move || {
                            let wait = wait.borrow_mut().take();

                            async move {
                                if let Some(wait) = wait {
                                    wait.await.map_err(|_| GuardReason("save failed".into()))?;
                                }

                                Ok(())
                            }
                        }),
                        DecisionChoice::cancel("cancel", "Cancel"),
                    ],
                })
            }
        })
        .expect("guard");
    entry.set_dirty(true).expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let first = decision(&router, &mut pool);

    router
        .navigator()
        .respond(first.id, "save")
        .expect("start save");
    pool.run_until_stalled();
    entry
        .set_dirty(true)
        .expect("new draft revision during save");
    finish_save.send(()).expect("finish save");
    let second = decision(&router, &mut pool);

    assert_ne!(first.id, second.id);
    assert!(entry.is_dirty().expect("new edit remains dirty"));
    router
        .navigator()
        .respond(second.id, "cancel")
        .expect("cancel");
    assert!(matches!(
        pool.run_until(close.wait()).expect("outcome"),
        Outcome::Cancelled(_)
    ));
}

#[test]
fn cancelling_a_queued_conflict_does_not_wait_for_an_earlier_guard() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");

    router
        .entry(a.entry_id)
        .expect("entry")
        .set_dirty(true)
        .expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let ask = decision(&router, &mut pool);
    let queued = router.navigator().tab(a.tab_id).navigate(Destination::Home);

    router.cancel(queued.id()).expect("cancel queued conflict");

    assert_eq!(
        pool.run_until(queued.wait()).expect("outcome"),
        Outcome::Cancelled(CancelReason::Requested)
    );
    router
        .navigator()
        .respond(ask.id, "cancel")
        .expect("cancel close");
    assert!(matches!(
        pool.run_until(close.wait()).expect("outcome"),
        Outcome::Cancelled(_)
    ));
}

#[test]
fn browser_write_failures_are_visible_without_rolling_back_a_commit() {
    let (router, mut pool) = harness(RouterConfig::default());

    router.on_history(|_| Err("history storage unavailable".into()));
    open(&router, &mut pool, "A");

    assert_eq!(router.snapshot().tabs.len(), 1);
    assert_eq!(
        router.snapshot().history_error.as_deref(),
        Some("history storage unavailable")
    );
    router.on_history(|_| Ok(()));
    open(&router, &mut pool, "B");
    assert!(router.snapshot().history_error.is_none());
}

#[test]
fn a_closed_target_never_redirects_to_another_tab() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, "A");
    let b = open(&router, &mut pool, "B");

    committed(&mut pool, router.navigator().tab(a.tab_id).close());
    let stale = pool.run_until(
        router
            .navigator()
            .tab(a.tab_id)
            .navigate(Destination::Home)
            .wait(),
    );

    assert_eq!(stale, Err(OperationError::ClosedTarget));
    assert_eq!(router.snapshot().tabs[0].id, b.tab_id);
    assert_eq!(router.snapshot().tabs[0].entries.len(), 1);
}

#[test]
fn newer_browser_traversal_cancels_an_unresolved_application_guard() {
    let (router, mut pool) = harness(RouterConfig::default());

    router
        .entry_guard(|context| async move {
            if context.destination == Destination::Home {
                futures::future::pending::<()>().await;
            }

            GuardResult::Allow
        })
        .expect("guard");
    open(&router, &mut pool, "A");
    let older = router.navigator().restore_destination(Destination::Home);

    pool.run_until_stalled();
    let newer = router
        .navigator()
        .restore_destination(Destination::Document("B".into()));

    pool.run_until_stalled();
    assert_eq!(router.operation(newer.id()), Ok(OperationState::Committed));
    assert_eq!(
        pool.run_until(older.wait()).expect("older"),
        Outcome::Cancelled(CancelReason::Superseded)
    );
    assert_eq!(router.snapshot().tabs.len(), 2);
}

#[test]
fn closing_a_reveal_target_expires_its_pending_access_decision() {
    let (router, mut pool) = harness(RouterConfig::default());
    let prompt = Rc::new(std::cell::Cell::new(false));
    let enabled = prompt.clone();

    router
        .entry_guard(move |_| {
            let enabled = enabled.clone();

            async move {
                if enabled.get() {
                    GuardResult::Ask(DecisionRequest {
                        message: "Access?".into(),
                        choices: vec![DecisionChoice::proceed("allow", "Allow")],
                    })
                } else {
                    GuardResult::Allow
                }
            }
        })
        .expect("guard");
    let a = open(&router, &mut pool, "A");

    prompt.set(true);
    let reveal = router
        .navigator()
        .reveal(Destination::Document("A".into()), RevealScope::Window);

    let ask = decision(&router, &mut pool);

    committed(&mut pool, router.navigator().tab(a.tab_id).close());
    pool.run_until_stalled();
    assert_eq!(
        router.operation(reveal.id()),
        Ok(OperationState::Failed(OperationError::ClosedTarget))
    );
    assert_eq!(
        pool.run_until(reveal.wait()),
        Err(OperationError::ClosedTarget)
    );
    assert_eq!(
        router.navigator().respond(ask.id, "allow"),
        Err(DecisionError::Obsolete)
    );
    assert!(router.snapshot().decisions.is_empty());
}

#[test]
fn browser_admission_failure_repairs_the_url_without_disrupting_a_pending_close() {
    let (router, mut pool) = harness(RouterConfig {
        pending_operations: 1,
        ..RouterConfig::default()
    });

    let changes = Rc::new(RefCell::new(Vec::new()));
    let sink = changes.clone();

    router.on_history(move |change| {
        sink.borrow_mut().push(change);

        Ok(())
    });
    let a = open(&router, &mut pool, "A");

    router
        .entry(a.entry_id)
        .expect("entry")
        .set_dirty(true)
        .expect("edit");
    let close = router.navigator().tab(a.tab_id).close();
    let ask = decision(&router, &mut pool);
    let restore = router.navigator().restore_destination(Destination::Home);

    assert_eq!(
        pool.run_until(restore.wait()),
        Err(OperationError::QueueFull)
    );
    assert!(
        matches!(changes.borrow().last(), Some(HistoryChange::Replace(_, uri)) if uri == "/documents/A")
    );
    router
        .navigator()
        .respond(ask.id, "cancel")
        .expect("unrelated decision remains valid");
    assert_eq!(
        pool.run_until(close.wait()).expect("close"),
        Outcome::Cancelled(CancelReason::Decision)
    );
}
