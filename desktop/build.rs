fn main() {
    println!("cargo:rerun-if-changed=assets/narolink.ico");
    #[cfg(windows)]
    winres::WindowsResource::new()
        .set_icon("assets/narolink.ico")
        .set("ProductName", "Naro")
        .set("FileDescription", "Naro")
        .set("OriginalFilename", "Naro.exe")
        .compile()
        .expect("embed the Naro icon");
}
