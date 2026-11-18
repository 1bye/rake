//! Opt-in real-keychain probe. Run write, read in a new process, then delete.
//! Uses only dev.rake.secrets-17-test; always run delete after testing.
#[cfg(feature = "gpui-fast")]
use gpui_fast_platform as host;
#[cfg(not(feature = "gpui-fast"))]
use gpui_platform as host;
use rake::platform::{Platform, Secret};

fn main() {
    let operation = std::env::args().nth(1).expect("write, read or delete");

    host::application().run(move |cx| {
        let secrets = Platform::default()
            .secrets("dev.rake.secrets-17-test")
            .unwrap();

        let request = match operation.as_str() {
            "write" => secrets.write(
                "restart-probe",
                Secret::new(b"rake-test-token".to_vec()),
                cx,
            ),
            "read" => {
                let read = secrets.read("restart-probe", cx);

                Box::pin(async move {
                    let secret = read.await?.expect("test entry missing after restart");

                    assert!(secret.expose() == b"rake-test-token", "test entry differs");

                    Ok(())
                })
            }
            "absent" => {
                let read = secrets.read("restart-probe", cx);

                Box::pin(async move {
                    assert!(read.await?.is_none(), "test entry was not deleted");

                    Ok(())
                })
            }
            "delete" => secrets.delete("restart-probe", cx),
            _ => panic!("expected write, read or delete"),
        };

        cx.spawn(async move |cx| {
            let result = request.await;

            cx.update(|cx| cx.quit());

            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(1);
            }
        })
        .detach();
    });
}
