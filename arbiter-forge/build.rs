fn main() {
    slint_build::compile("ui/forge.slint").unwrap();

    // Embed ui/icon.ico as a Windows PE resource so the exe shows the correct
    // icon in Explorer, the taskbar, and the Alt+Tab switcher.
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("ui/icon.ico");
        res.compile().expect("Failed to embed Windows icon resource");
    }
}
