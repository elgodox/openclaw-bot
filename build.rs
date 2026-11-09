// Gives the Windows program file its icon and version details.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "OpenClaw Bot")
            .set("FileDescription", "OpenClaw Bot")
            .set("CompanyName", "elgodox")
            .set("OriginalFilename", "OpenClaw Bot.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no se pudo incrustar el icono: {e}");
        }
    }
}
