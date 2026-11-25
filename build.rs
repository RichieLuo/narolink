fn main() {
    println!("cargo:rerun-if-changed=assets/narolink.ico");
    #[cfg(windows)]
    winres::WindowsResource::new()
        .set_icon("assets/narolink.ico")
        .set("ProductName", "NaroLink")
        .set("FileDescription", "NaroLink")
        .set("OriginalFilename", "NaroLink.exe")
        .compile()
        .expect("embed the NaroLink icon");
}
