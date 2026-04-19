fn main() {
    // Embed the icon.ico from the forge project as a Windows PE resource 
    // so the arbiter.exe service shows the correct icon in Task Manager,
    // Explorer, and the Alt+Tab switcher.
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        // Since we are in arbiter-app/, we go to arbiter-data/
        // This sets the icon for the .exe file itself.
        res.set_icon("../arbiter-data/icon.ico");
        res.compile().expect("Failed to embed Windows icon resource");
    }
}
