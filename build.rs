fn main() {
    if cfg!(target_os = "windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource.set("ProductName", "Secular Nightreign");
        resource.set("FileDescription", "Secular mod installer and launcher");
        resource.set("LegalCopyright", "GhostResident");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
