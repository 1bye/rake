//! Restores the browser default fonts that GPUI CE supplied before window creation.

use std::borrow::Cow;

pub(super) fn install(cx: &gpui::App) -> gpui::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../../assets/fonts/IBMPlexSans-Italic.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
        Cow::Borrowed(
            include_bytes!("../../assets/fonts/IBMPlexSans-SemiBoldItalic.ttf").as_slice(),
        ),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Lilex-Bold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Lilex-BoldItalic.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Lilex-Italic.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/Lilex-Regular.ttf").as_slice()),
    ])
}
