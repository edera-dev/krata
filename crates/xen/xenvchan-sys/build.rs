fn main() {
    if let Ok(lib_dir) = std::env::var("XENVCHAN_LIB_DIR") {
        println!("cargo:rustc-link-search=native={lib_dir}");
        println!("cargo:rustc-link-lib=dylib=xenvchan");
        return;
    }

    let mut config = pkg_config::Config::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
        config.statik(true);
    }

    config
        .probe("xenvchan")
        .or_else(|_| {
            let mut fallback = pkg_config::Config::new();
            if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
                fallback.statik(true);
            }
            fallback.probe("libxenvchan")
        })
        .expect("failed to locate libxenvchan with pkg-config");
}
