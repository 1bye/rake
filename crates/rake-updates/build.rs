//! Builds the optional owned native lifecycle bridge.
fn main() {
    #[cfg(feature = "sparkle")]
    sparkle();

    #[cfg(feature = "native-lifecycle")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=native/macos_lifecycle.m");
        let mut native = cc::Build::new();

        if std::env::var_os("CARGO_FEATURE_NATIVE_TESTS").is_some() {
            native.define("RAKE_NATIVE_LIFECYCLE_TEST", None);
        }

        native
            .file("native/macos_lifecycle.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("rake_macos_lifecycle");
        println!("cargo:rustc-link-lib=framework=Cocoa");
    }
}

#[cfg(feature = "sparkle")]
fn sparkle() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    println!("cargo:rerun-if-env-changed=RAKE_SPARKLE_FRAMEWORK");
    println!("cargo:rerun-if-changed=native/sparkle.m");
    let framework=std::path::PathBuf::from(std::env::var_os("RAKE_SPARKLE_FRAMEWORK").expect("use rake bundle or set RAKE_SPARKLE_FRAMEWORK to the pinned Sparkle.framework for local SDK development"));
    let parent = framework.parent().expect("Sparkle framework parent");

    cc::Build::new()
        .file("native/sparkle.m")
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .flag("-F")
        .flag(parent.to_str().expect("SDK path must be UTF-8"))
        .compile("rake_sparkle");
    println!("cargo:rustc-link-search=framework={}", parent.display());
    println!("cargo:rustc-link-lib=framework=Sparkle");
}
