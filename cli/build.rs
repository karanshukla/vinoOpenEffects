include!("../buildinfo/version.rs");

fn main() {
    println!(
        "cargo:rustc-env=VINOOPENEFFECTS_VERSION={}",
        release_version()
    );
}
