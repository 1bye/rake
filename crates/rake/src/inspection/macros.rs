//! Facade-owned expansion gate: never consult a downstream crate's feature names.

#[cfg(feature = "devtools")]
#[doc(hidden)]
#[macro_export]
macro_rules! __rake_inspection {
    ($value:expr, |$node:ident| $enabled:expr $(,)?) => {{
        let $node = $value;
        $enabled
    }};
    ($value:expr, |$node:ident| $enabled:expr, |$plain:ident| $disabled:expr $(,)?) => {{
        let $node = $value;
        $enabled
    }};
    ($enabled:expr, $disabled:expr $(,)?) => {
        $enabled
    };
}

#[cfg(not(feature = "devtools"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __rake_inspection {
    ($value:expr, |$node:ident| $enabled:expr $(,)?) => {
        $value
    };
    ($value:expr, |$node:ident| $enabled:expr, |$plain:ident| $disabled:expr $(,)?) => {{
        let $plain = $value;
        $disabled
    }};
    ($enabled:expr, $disabled:expr $(,)?) => {
        $disabled
    };
}
