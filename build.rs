#[cfg(target_os = "windows")]
#[path = "src/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");

    #[cfg(target_os = "windows")]
    embed_windows_icon();
}

#[cfg(target_os = "windows")]
fn embed_windows_icon() {
    use std::{env, fs, path::PathBuf};

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let icon_path = out_dir.join("vaultforge-icon.ico");
    let rc_path = out_dir.join("vaultforge-icon.rc");

    let raster = icon::icon_raster();
    let image = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba)
        .expect("icon raster dimensions must match pixel data length");
    image
        .save_with_format(&icon_path, image::ImageFormat::Ico)
        .expect("failed to write VaultForge icon");

    let manifest_path = out_dir.join("vaultforge.manifest");
    fs::write(&manifest_path, MANIFEST).expect("failed to write VaultForge manifest");

    let version = |name: &str| env::var(name).ok().and_then(|value| value.parse::<u16>().ok()).unwrap_or(0);
    let (major, minor, patch) = (
        version("CARGO_PKG_VERSION_MAJOR"),
        version("CARGO_PKG_VERSION_MINOR"),
        version("CARGO_PKG_VERSION_PATCH"),
    );
    let semver = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let escape = |path: &std::path::Path| path.display().to_string().replace('\\', "\\\\");

    // Icon, manifest (id 1, type 24) and version info — the last is what
    // Explorer's Properties dialog and antivirus reputation checks read.
    let script = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "JunkfoodJon"
      VALUE "FileDescription", "VaultForge"
      VALUE "FileVersion", "{semver}"
      VALUE "InternalName", "VaultForge"
      VALUE "LegalCopyright", "Copyright (c) 2026 JunkfoodJon. PolyForm Noncommercial License 1.0.0."
      VALUE "OriginalFilename", "VaultForge.exe"
      VALUE "ProductName", "VaultForge"
      VALUE "ProductVersion", "{semver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = escape(&icon_path),
        manifest = escape(&manifest_path),
    );
    fs::write(&rc_path, script).expect("failed to write VaultForge resource script");

    if let Err(err) = embed_resource::compile(&rc_path, embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=Windows resources (icon, manifest, version info) were not embedded: {err}");
    }
}

/// asInvoker: VaultForge never needs Administrator. Per-monitor-v2 DPI, long
/// path support (deep node_modules/target trees), the modern common-controls
/// look for native dialogs, and Windows 10/11 as the supported OS.
#[cfg(target_os = "windows")]
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="JunkfoodJon.VaultForge" version="1.0.0.0" processorArchitecture="*"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;