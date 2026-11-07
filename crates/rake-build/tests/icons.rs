//! Covers: one source icon becomes portable outputs and rejects invalid input.
//! Real: PNG/ICO codecs and SVG renderer. Doubles: synthetic icons.
//! Requires: no native tools or external image resources.

#[test]
fn default_icon_uses_rake_brand_and_custom_icons_override_it() {
    let root = tempfile::tempdir().unwrap();
    let generated = rake_build::generate_icons(None, root.path()).unwrap();

    for name in ["icon.png", "icon.ico"] {
        let icon = image::open(generated.join(name)).unwrap().into_rgba8();

        assert_eq!(icon.dimensions(), (256, 256));
        assert_eq!(icon.get_pixel(128, 128).0, [230, 82, 53, 255]);
        assert_eq!(icon.get_pixel(128, 54).0, [255, 244, 222, 255]);
        assert_eq!(icon.get_pixel(0, 0).0[3], 0);
    }

    let custom = root.path().join("custom.png");

    image::RgbaImage::from_pixel(16, 16, image::Rgba([20, 40, 200, 255]))
        .save(&custom)
        .unwrap();
    rake_build::generate_icons(Some(&custom), root.path()).unwrap();

    for name in ["icon.png", "icon.ico"] {
        let icon = image::open(generated.join(name)).unwrap().into_rgba8();

        assert_eq!(icon.get_pixel(128, 128).0, [20, 40, 200, 255]);
    }
}

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
