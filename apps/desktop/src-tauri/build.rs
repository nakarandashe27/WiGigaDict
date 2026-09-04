fn main() {
    // The catalog signing key is injected through the environment at build time. Without
    // this, cargo keeps the stale binary when the key changes and the rebuild silently
    // does nothing.
    println!("cargo:rerun-if-env-changed=WIGIGADICT_CATALOG_PUBLIC_KEY");
    tauri_build::build()
}
