fn main() {
    #[cfg(target_os = "windows")]
    {
        if std::path::Path::new("resources/icons/icon.ico").exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon("resources/icons/icon.ico");
            res.compile().unwrap();
        }
    }
}
