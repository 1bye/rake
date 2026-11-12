//! Covers: facade feature exports without GPUI.
//! Real: public configuration API.
//! Doubles: none.
//! Requires: launch-at-login feature; never accesses the user's login settings.
#![cfg(feature = "launch-at-login")]
use rake::login_item::{Error, LaunchAtLogin, Status};

#[test]
fn portable_feature_exports_the_complete_contract() {
    let login = LaunchAtLogin::new("dev.rake.consumer")
        .unwrap()
        .args(["--background"])
        .unwrap();

    let _: LaunchAtLogin = login;
    let _: Status = Status::Unsupported("test".into());

    assert!(matches!(
        LaunchAtLogin::new("../unsafe"),
        Err(Error::InvalidId)
    ));
}
