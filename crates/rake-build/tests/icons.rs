//! Covers: one source icon becomes portable outputs and rejects invalid input.
//! Real: PNG/ICO codecs and SVG renderer. Doubles: synthetic icons.
//! Requires: no native tools or external image resources.
#[test]
fn icons_support_png_svg_and_default_without_external_resources() {
    let root = tempfile::tempdir().unwrap();
    let generated = rake_build::generate_icons(None, root.path()).unwrap();
    assert!(generated.join("icon.ico").is_file());
    let source = root.path().join("source.svg");
    std::fs::write(&source, r#"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256"><rect width="256" height="256" fill="red"/></svg>"#).unwrap();
    rake_build::generate_icons(Some(&source), root.path()).unwrap();
    let pixel = root.path().join("pixel.png");
    image::RgbaImage::new(1, 1).save(&pixel).unwrap();
    rake_build::generate_icons(Some(&pixel), root.path()).unwrap();
    let invalid = root.path().join("invalid.png");
    image::RgbaImage::new(32, 64).save(&invalid).unwrap();
    assert!(rake_build::generate_icons(Some(&invalid), root.path()).is_err());
}
