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
        res.set("FileDescription", "Arbiter Orchestration Engine");
        res.set("ProductName", "Project Arbiter");
        res.set("OriginalFilename", "arbiter.exe");
        res.set("LegalCopyright", "Copyright (c) 2026");
        res.set("ProductVersion", "0.1.6.0");
        res.set("FileVersion", "0.1.6.0");
        res.set_manifest(r#"
        <assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
          <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
            <security>
              <requestedPrivileges>
                <requestedExecutionLevel level="asInvoker" uiAccess="false" />
              </requestedPrivileges>
            </security>
          </trustInfo>
        </assembly>
        "#);
        res.compile().expect("Failed to embed Windows icon resource");
    }
}
