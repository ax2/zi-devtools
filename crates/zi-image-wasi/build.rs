fn main() {
    if std::env::var("TARGET").as_deref() == Ok("wasm32-wasip1") {
        // Studio's existing resource-WASI Store allows 256 MiB while compute
        // workers stay capped at 64 MiB by the shared target configuration.
        println!("cargo:rustc-link-arg=--max-memory=268435456");
    }
}
