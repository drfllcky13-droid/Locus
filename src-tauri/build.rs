fn main() {
    // Export the GPU-preference symbols declared in main.rs: an executable exports nothing
    // unless the linker is told to.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:NvOptimusEnablement");
        println!("cargo:rustc-link-arg-bins=/EXPORT:AmdPowerXpressRequestHighPerformance");
    }
    tauri_build::build()
}
