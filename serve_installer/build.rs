fn main() {
    println!("cargo:rustc-link-arg=/MANIFEST:embed");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:serve_installer.exe.manifest");
}
