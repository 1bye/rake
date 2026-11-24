//! Inspection expansions selected by the facade or GPUI adapter.

#[doc(hidden)]
#[macro_export]
macro_rules! __rake_inspection_enabled {
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

#[doc(hidden)]
#[macro_export]
macro_rules! __rake_inspection_disabled {
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
