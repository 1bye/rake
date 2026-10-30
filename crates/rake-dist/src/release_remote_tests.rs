//! Real TLS/process boundaries with disposable protocol services; no live publication.
use super::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start_service(service: &Path) -> (Server, serde_json::Value) {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/distribution_test_publisher.py");
    let mut server = Server(
        Command::new("python3")
            .arg(script)
            .arg(service)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: serde_json::Value = serde_json::from_str(&line).expect("TLS server ready");
    (server, ready)
}
fn appimage_fixture(
    root: &Path,
    version: &str,
    ready: &serde_json::Value,
    base: &str,
) -> ReleaseRequest {
    let mut request = super::tests::fixture(root, version);
    let path = request.distributions[0].join("distribution.json");
    let mut distribution: Distribution = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    fs::rename(
        request.distributions[0].join("app.tar.gz"),
        request.distributions[0].join("App.AppImage"),
    )
    .unwrap();
    let key = hex::decode(ready["public_key"].as_str().unwrap()).unwrap();
    let artifact = &mut distribution.artifacts[0];
    artifact.path = "App.AppImage".into();
    artifact.format = Format::Appimage;
    artifact.update_policy = Some(rake_dist_core::UpdateBinding {
        schema_version: 1,
        identifier: request.identifier.clone(),
        version: version.into(),
        package_version: version.into(),
        target: distribution.target.clone(),
        format: Format::Appimage,
        owner: rake_dist_core::UpdateOwner::Appimage,
        channel: "stable".into(),
        base_url: Some(base.into()),
        public_keys: BTreeMap::from([("appimage".into(), key.clone())]),
    });
    files::write_json(&path, &distribution).unwrap();
    request.public_keys.insert("appimage".into(), key);
    request.config.signers.insert(
        "appimage".into(),
        serde_json::from_value(ready["signer"].clone()).unwrap(),
    );
    request.config.required = BTreeMap::from([(distribution.target, vec![Format::Appimage])]);
    request.updates_enabled = true;
    request
}
#[test]
fn https_upload_readback_and_conflicts_never_partially_promote() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("service");
    fs::create_dir(&service).unwrap();
    let (_server, ready) = start_service(&service);
    let ca = fs::read(ready["ca"].as_str().unwrap()).unwrap();
    let certificate = ureq::tls::Certificate::from_pem(&ca).unwrap().to_owned();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::Specific(std::sync::Arc::new(vec![
                    certificate,
                ])))
                .build(),
        )
        .build()
        .into();
    let url = ready["url"].as_str().unwrap();
    let flag = AtomicBool::new(false);
    let mut first = appimage_fixture(root.path(), "1.0.0", &ready, url);
    first.config.publisher = Some(Publisher::Https {
        url: url.into(),
        public_url: url.into(),
        token_env: "UNUSED_TEST_TOKEN".into(),
    });
    let mut prepared = prepare(&first, &flag).unwrap();
    assert!(
        publish_https_with_agent(
            &first,
            &mut prepared,
            url,
            "disposable-test-token",
            &flag,
            &ureq::Agent::new_with_defaults()
        )
        .is_err(),
        "untrusted TLS CA is rejected"
    );
    assert!(
        publish_https_with_agent(&first, &mut prepared, url, "wrong-token", &flag, &agent).is_err()
    );
    assert!(!service.join("current.json").exists());
    publish_https_with_agent(
        &first,
        &mut prepared,
        url,
        "disposable-test-token",
        &flag,
        &agent,
    )
    .unwrap();
    let previous = fs::read(service.join("current.json")).unwrap();
    let mut next = appimage_fixture(root.path(), "1.0.1", &ready, url);
    next.config.publisher = first.config.publisher.clone();
    for fault in ["reject-upload", "corrupt", "conflict"] {
        fs::write(service.join(fault), "").unwrap();
        let mut prepared = prepare(&next, &flag).unwrap();
        assert!(
            publish_https_with_agent(
                &next,
                &mut prepared,
                url,
                "disposable-test-token",
                &flag,
                &agent
            )
            .is_err(),
            "fault {fault}"
        );
        assert_eq!(
            fs::read(service.join("current.json")).unwrap(),
            previous,
            "fault {fault}"
        );
        fs::remove_file(service.join(fault)).unwrap();
    }
    let mut prepared = prepare(&next, &flag).unwrap();
    publish_https_with_agent(
        &next,
        &mut prepared,
        url,
        "disposable-test-token",
        &flag,
        &agent,
    )
    .unwrap();
    let state: PublishedRelease =
        serde_json::from_slice(&fs::read(service.join("current.json")).unwrap()).unwrap();
    assert_eq!(state.version, "1.0.1");
    assert!(state.previous.is_some());
    let record = &state.files["stable/x86_64-unknown-linux-gnu/appimage.json"];
    let object = service
        .join("objects")
        .join(record.url.rsplit('/').next().unwrap());
    let signed: SignedUpdateFeed = serde_json::from_slice(&fs::read(object).unwrap()).unwrap();
    let feed: UpdateFeed = serde_json::from_str(&signed.payload).unwrap();
    assert_eq!(
        feed.releases.len(),
        2,
        "HTTPS promotion retains installed versions"
    );
    assert!(
        publish_https_with_agent(
            &first,
            &mut prepare(&first, &flag).unwrap(),
            url,
            "disposable-test-token",
            &flag,
            &agent
        )
        .is_err()
    );
}
#[test]
fn github_draft_readback_and_previous_version_gate_precede_latest_promotion() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("credentials-service");
    fs::create_dir(&service).unwrap();
    let (_server, ready) = start_service(&service);
    let url = "https://github.com/owner/test/releases/latest/download";
    let script = root.path().join("gh-fixture");
    let mut file = fs::File::create(&script).unwrap();
    writeln!(
        file,
        "#!/usr/bin/env python3\nimport json,pathlib,shutil,sys\nroot=pathlib.Path({})",
        serde_json::to_string(&root.path().to_string_lossy()).unwrap()
    )
    .unwrap();
    file.write_all(br#"
args=sys.argv[1:];assert args[0]=='release';op=args[1]
live=root/'live'
if op=='list':print(json.dumps([dict(isLatest=True,tagName=live.read_text())] if live.exists() else []))
elif op=='create':
    assert '--draft' in args and '--verify-tag' in args
    (root/args[2]).mkdir()
elif op=='upload':shutil.copy2(args[-1],root/args[2]/pathlib.Path(args[-1]).name)
elif op=='download':
    dest=pathlib.Path(args[args.index('--dir')+1]);pattern=args[args.index('--pattern')+1] if '--pattern' in args else '*'
    for source in (root/args[2]).glob(pattern):shutil.copy2(source,dest/source.name)
    if (root/'corrupt').exists() and '--pattern' not in args:(dest/'release.json').write_text('corrupted downloaded state')
elif op=='edit':
    assert '--draft=false' in args and '--latest' in args
    live.write_text(args[2])
else:raise AssertionError(args)
"#).unwrap();
    drop(file);
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let flag = AtomicBool::new(false);
    let mut first = appimage_fixture(root.path(), "1.0.0", &ready, url);
    first.config.publisher = Some(Publisher::Github {
        repository: "owner/test".into(),
        tag: "v1".into(),
    });
    let mut prepared = prepare(&first, &flag).unwrap();
    publish_github_with_program(&first, &mut prepared, "owner/test", "v1", &flag, &script).unwrap();
    assert_eq!(fs::read_to_string(root.path().join("live")).unwrap(), "v1");
    let mut next = appimage_fixture(root.path(), "1.0.1", &ready, url);
    next.config.publisher = Some(Publisher::Github {
        repository: "owner/test".into(),
        tag: "v2".into(),
    });
    fs::write(root.path().join("corrupt"), "").unwrap();
    assert!(
        publish_github_with_program(
            &next,
            &mut prepare(&next, &flag).unwrap(),
            "owner/test",
            "v2",
            &flag,
            &script
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(root.path().join("live")).unwrap(), "v1");
    assert!(
        root.path().join("v2/release.json").is_file(),
        "failed draft is retained"
    );
    fs::remove_file(root.path().join("corrupt")).unwrap();
    next.config.publisher = Some(Publisher::Github {
        repository: "owner/test".into(),
        tag: "v3".into(),
    });
    publish_github_with_program(
        &next,
        &mut prepare(&next, &flag).unwrap(),
        "owner/test",
        "v3",
        &flag,
        &script,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(root.path().join("live")).unwrap(), "v3");
    let state: PublishedRelease =
        serde_json::from_slice(&fs::read(root.path().join("v3/release.json")).unwrap()).unwrap();
    let record = &state.files["stable/x86_64-unknown-linux-gnu/appimage.json"];
    assert_eq!(
        record.url,
        "https://github.com/owner/test/releases/download/v3/stable-x86_64-unknown-linux-gnu-appimage.json"
    );
    let signed: SignedUpdateFeed = serde_json::from_slice(
        &fs::read(
            root.path()
                .join("v3/stable-x86_64-unknown-linux-gnu-appimage.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let feed: UpdateFeed = serde_json::from_str(&signed.payload).unwrap();
    assert_eq!(
        feed.releases.len(),
        2,
        "GitHub feed retains signed installed ownership"
    );
    assert!(
        publish_github_with_program(
            &first,
            &mut prepare(&first, &flag).unwrap(),
            "owner/test",
            "old",
            &flag,
            &script
        )
        .is_err()
    );
    assert!(!root.path().join("old").exists());
}

#[test]
fn directory_release_carries_signed_installed_history_without_old_build_directories() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("service");
    fs::create_dir(&service).unwrap();
    let (_server, ready) = start_service(&service);
    let url = ready["url"].as_str().unwrap();
    let flag = AtomicBool::new(false);
    let mut first = appimage_fixture(root.path(), "1.0.0", &ready, url);
    first.config.publisher = Some(Publisher::Directory {
        path: root.path().join("published"),
        public_url: url.into(),
    });
    publish(&first, &flag).unwrap();
    fs::remove_dir_all(&first.distributions[0]).unwrap();
    let mut next = appimage_fixture(root.path(), "1.0.1", &ready, url);
    next.config.publisher = first.config.publisher.clone();
    next.published_at += 8 * 86400; // Old feed expiry cannot erase authenticated installed versions.
    publish(&next, &flag).unwrap();
    let path = root
        .path()
        .join("published/stable/x86_64-unknown-linux-gnu/appimage.json");
    let envelope: SignedUpdateFeed = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let feed: UpdateFeed = serde_json::from_str(&envelope.payload).unwrap();
    assert_eq!(feed.releases.len(), 2);
    assert!(
        feed.releases
            .iter()
            .any(|release| release.version == "1.0.0")
    );
    let before = fs::read(root.path().join("published/stable/release.json")).unwrap();
    let mut third = appimage_fixture(root.path(), "1.0.2", &ready, url);
    third.config.publisher = first.config.publisher.clone();
    third.published_at = next.published_at + 1;
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"tampered history").unwrap();
    assert!(
        publish(&third, &flag)
            .unwrap_err()
            .to_string()
            .contains("history hash mismatch")
    );
    assert_eq!(
        fs::read(root.path().join("published/stable/release.json")).unwrap(),
        before
    );
}
