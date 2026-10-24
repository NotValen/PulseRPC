fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/pulserpc.ico");
        res.set("ProductName", "PulseRPC");
        res.set(
            "FileDescription",
            "PulseRPC — automatic Discord Rich Presence",
        );
        res.set("LegalCopyright", "MIT License");
        res.set("OriginalFilename", "pulserpc.exe");
        res.set_manifest_file("assets/pulserpc.manifest");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/pulserpc.ico");
    println!("cargo:rerun-if-changed=assets/pulserpc.manifest");
}
