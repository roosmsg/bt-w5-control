fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("Slint UI failed to compile");

    // Icon, version information and manifest in the exe (Explorer, Task Manager, Start menu).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=ui/icon.ico");
        println!("cargo:rerun-if-changed=app.manifest");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("ui/icon.ico")
            .set("ProductName", "BT-W5 Control")
            .set("FileDescription", "BT-W5 Control")
            .set("InternalName", "bt-w5-control")
            .set("OriginalFilename", "bt-w5-control.exe")
            .set_manifest_file("app.manifest");
        res.compile().expect("Windows resources failed to compile");
    }
}
