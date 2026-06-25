fn main() {
    if std::env::var_os("CARGO_FEATURE_APP").is_some() {
        tauri_build::build();
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=iconv");
    }
}
