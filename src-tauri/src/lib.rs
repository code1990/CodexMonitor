#[cfg(feature = "app")]
mod app_lib;

#[cfg(feature = "app")]
pub use app_lib::run;
