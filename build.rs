fn main() {
    slint_build::compile("ui/main.slint").expect("Slint UI compilation failed");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        if let (Ok(sdk), Ok(version)) = (
            std::env::var("WindowsSdkDir"),
            std::env::var("WindowsSDKVersion"),
        ) {
            res.set_toolkit_path(&format!(
                "{}bin/{}/x64",
                sdk,
                version.trim_matches(['/', '\\'])
            ));
        }
        res.set_icon("assets/snapline.ico");
        res.set("ProductName", "Snapline");
        res.set("FileDescription", "Snapline · 晾图");
        res.set("LegalCopyright", "Copyright © 2026 Snapline contributors");
        res.set_manifest(r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0"><assemblyIdentity version="1.0.0.0" name="Snapline" type="win32"/><trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security></trustInfo><application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings><dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness><longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware></windowsSettings></application></assembly>"#);
        res.compile().expect("Windows resources compilation failed");
    }
}
