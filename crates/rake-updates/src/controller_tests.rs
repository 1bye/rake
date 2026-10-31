//! Covers: observable revisions, single-operation ownership, cancellation and guarded handoff.
//! Real: controller, filesystem policy loading and explicitly owned worker threads.
//! Doubles: provider work closures. Requires: no network or native SDK.
use super::*;
use futures::FutureExt as _;

use std::{
    cell::Cell,
    fs,
    sync::mpsc,
    task::{Context, Poll},
};

fn controller() -> UpdateController {
    controller_for(UpdateOwner::Disabled)
}

fn controller_for(owner: UpdateOwner) -> UpdateController {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "rake-controller-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));

    fs::create_dir(&root).unwrap();
    let mut policy = serde_json::json!({"schema_version":1,"identifier":"dev.rake.controller","version":"1.0.0","package_version":"1.0.0","target":"x86_64-unknown-linux-gnu","format":"deb","owner":"disabled","channel":"stable","base_url":null,"public_keys":{}});
    let mut installation = serde_json::json!({"schema_version":1,"identifier":"dev.rake.controller","version":"1.0.0","package_version":"1.0.0","target":"x86_64-unknown-linux-gnu","platform":"linux","format":"deb","update_owner":"disabled","updates":{},"helpers":{},"runtimes":{},"frameworks":{},"native_groups":{}});

    policy["owner"] = serde_json::to_value(owner).unwrap();
    installation["update_owner"] = serde_json::to_value(owner).unwrap();

    if owner != UpdateOwner::Disabled {
        policy["base_url"] = serde_json::json!("https://example.org/updates");
        installation["updates"] =
            serde_json::json!({"enabled":true,"base_url":"https://example.org/updates"});
    }

    fs::write(
        root.join("rake-installation.json"),
        serde_json::to_vec(&installation).unwrap(),
    )
    .unwrap();
    let resources = rake_resources::Resources::load(&root).unwrap();
    let service = UpdateService::new(&policy.to_string(), &resources).unwrap();

    fs::remove_dir_all(root).unwrap();

    UpdateController::new(service)
}

#[test]
fn subscriptions_do_not_miss_changes_and_dispatch_does_not_spin() {
    fn send<T: Send>() {}
    send::<UpdateWork>();
    let controller = controller();
    let revision = controller.snapshot().revision;
    let changed = controller.changed(revision);

    futures::pin_mut!(changed);
    assert!(changed.as_mut().now_or_never().is_none());
    controller
        .queue(
            Job::Test(Box::new(|| Ok(Output::Empty))),
            UpdateState::Checking,
        )
        .unwrap();
    assert_eq!(
        futures::executor::block_on(changed).state,
        UpdateState::Checking
    );
    assert_eq!(
        futures::executor::block_on(controller.changed(revision)).state,
        UpdateState::Checking
    );
    let work = controller.take_work().unwrap();

    assert!(controller.take_work().is_none());
    let current = controller.snapshot().revision;

    assert!(controller.take_work().is_none());
    assert_eq!(
        controller.snapshot().revision,
        current,
        "idle driver must not wake itself"
    );
    work.run();
    assert_eq!(
        futures::executor::block_on(controller.changed(current)).state,
        UpdateState::UpToDate
    );
}

#[test]
fn cancellation_keeps_ownership_until_a_running_worker_finishes() {
    let controller = controller();
    let (started, wait_started) = mpsc::channel();
    let (finish, wait_finish) = mpsc::channel();

    controller
        .queue(
            Job::Test(Box::new(move || {
                started.send(()).unwrap();
                wait_finish.recv().unwrap();

                Ok(Output::Empty)
            })),
            UpdateState::Checking,
        )
        .unwrap();
    let work = controller.take_work().unwrap();
    let worker = std::thread::spawn(move || work.run());

    wait_started.recv().unwrap();
    controller.cancel();
    assert_eq!(controller.state(), UpdateState::Cancelling);
    assert!(matches!(
        controller.queue(Job::Check, UpdateState::Checking),
        Err(Error::Busy)
    ));
    finish.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(
        controller.state(),
        UpdateState::Idle,
        "cancelled success cannot become UpToDate"
    );
    controller
        .queue(
            Job::Test(Box::new(|| Err(Error::Provider("test failure".into())))),
            UpdateState::Checking,
        )
        .unwrap();
    controller.take_work().unwrap().run();
    assert_eq!(
        controller.state(),
        UpdateState::Failed {
            message: "update provider: test failure".into()
        }
    );
    controller
        .queue(
            Job::Test(Box::new(|| Ok(Output::Empty))),
            UpdateState::Checking,
        )
        .unwrap();
    controller.take_work().unwrap().run();
    assert_eq!(controller.state(), UpdateState::UpToDate);
}

#[test]
fn dropping_work_or_a_driver_cancels_without_executing_provider_code() {
    let controller = controller();

    controller
        .queue(
            Job::Test(Box::new(|| panic!("unstarted provider must not execute"))),
            UpdateState::Checking,
        )
        .unwrap();
    drop(controller.take_work().unwrap());
    assert_eq!(controller.state(), UpdateState::Idle);
    controller
        .queue(
            Job::Test(Box::new(|| panic!("cancelled provider must not execute"))),
            UpdateState::Checking,
        )
        .unwrap();
    controller.cancel();
    assert!(controller.take_work().is_none());
    controller.queue(Job::Check, UpdateState::Checking).unwrap();
    let mut driver = Box::pin(controller.run(|work| async move {
        let _work = work;

        std::future::pending::<()>().await;
    }));

    assert!(matches!(
        driver
            .as_mut()
            .poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
        Poll::Pending
    ));
    drop(driver);
    assert_eq!(controller.state(), UpdateState::Idle);
}

#[test]
fn restart_approval_cancellation_and_owner_drop_never_bypass_guards() {
    let controller = controller();
    let called = Rc::new(Cell::new(false));

    let info = UpdateInfo {
        version: "1.0.1".into(),
    };

    let create = || {
        let called = Rc::clone(&called);

        RestartHandoff::new(move || {
            called.set(true);

            Ok(())
        })
    };

    let handoff = controller.observe_handoff(create(), info.clone());

    assert_eq!(
        controller.state(),
        UpdateState::AwaitingApproval(info.clone())
    );
    assert!(matches!(handoff.commit(false), Err(Error::Declined)));
    assert!(!called.get());
    assert_eq!(controller.state(), UpdateState::Idle);
    let handoff = controller.observe_handoff(create(), info.clone());

    controller.cancel();
    controller.queue(Job::Check, UpdateState::Checking).unwrap();
    assert!(matches!(handoff.commit(true), Err(Error::Cancelled)));
    assert_eq!(
        controller.state(),
        UpdateState::Checking,
        "a stale handoff cannot overwrite new state"
    );
    assert!(!called.get());
    controller.cancel();
    let handoff = controller.observe_handoff(create(), info.clone());

    assert_eq!(
        handoff.commit(true).unwrap(),
        crate::RestartDisposition::ExitHost
    );
    assert!(called.get());
    assert_eq!(controller.state(), UpdateState::Restarting(info.clone()));
    called.set(false);
    let handoff = controller.observe_handoff(create(), info);

    drop(controller);
    assert!(matches!(handoff.commit(true), Err(Error::Cancelled)));
    assert!(!called.get());
}

#[test]
fn disabled_and_managed_installations_cannot_enqueue_self_updates() {
    let controller = controller();

    assert_eq!(controller.state(), UpdateState::Disabled);
    assert!(controller.check().is_err());
    assert!(controller.download().is_err());
    assert!(controller.restart_handoff(vec![]).is_err());
    assert!(controller.take_work().is_none());
    let managed = controller_for(UpdateOwner::Apt);

    assert_eq!(
        managed.state(),
        UpdateState::ManagedExternally(UpdateOwner::Apt)
    );
    assert!(managed.check().is_err());
    assert!(managed.download().is_err());
    assert!(managed.restart_handoff(vec![]).is_err());
    assert!(managed.take_work().is_none());
}

#[test]
fn controller_service_accessor_preserves_the_owned_binding() {
    let controller = controller();

    assert_eq!(controller.binding(), controller.service().binding());
}
