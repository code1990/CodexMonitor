#[cfg_attr(
    any(
        target_os = "ios",
        target_os = "android",
        not(feature = "dictation-native")
    ),
    path = "stub.rs"
)]
#[cfg_attr(
    all(
        not(any(target_os = "ios", target_os = "android")),
        feature = "dictation-native"
    ),
    path = "real.rs"
)]
mod imp;

pub(crate) use imp::*;
