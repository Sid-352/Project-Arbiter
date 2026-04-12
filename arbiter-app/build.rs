fn main() {
    // Embed the icon.ico from the forge project as a Windows PE resource 
    // so the arbiter.exe service shows the correct icon in Task Manager.
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        // Since we are in arbiter-app/, we go up one level then into forge/ui/
        res.set_icon("../arbiter-forge/ui/icon.ico");
        res.compile().expect("Failed to embed Windows icon resource");
    }
}
