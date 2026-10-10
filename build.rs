use std::{env, fs, path::PathBuf, process::Command};

fn resource_compiler() -> PathBuf {
    if let Some(path) = env::var_os("RC") {
        return path.into();
    }
    if let Some(path) = env::var_os("WindowsSdkVerBinPath") {
        let compiler = PathBuf::from(path).join("x64/rc.exe");
        if compiler.is_file() {
            return compiler;
        }
    }
    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        let sdk = PathBuf::from(program_files).join("Windows Kits/10/bin");
        if let Ok(entries) = fs::read_dir(sdk) {
            let mut versions: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            versions.sort();
            for version in versions.into_iter().rev() {
                let compiler = version.join("x64/rc.exe");
                if compiler.is_file() {
                    return compiler;
                }
            }
        }
    }
    "rc.exe".into()
}

fn embed_brand_resources() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = format!("{},0", version.replace('.', ","));
    let icon = root
        .join("assets/brand/diskvale.ico")
        .display()
        .to_string()
        .replace('\\', "/");
    let script = output.join("diskvale.rc");
    let compiled = output.join("diskvale.res");
    fs::write(
        &script,
        format!(
            r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Diskvale - Disk Explorer\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "Diskvale\0"
      VALUE "OriginalFilename", "Diskvale.exe\0"
      VALUE "ProductName", "Diskvale\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        ),
    )
    .expect("write Diskvale resource script");
    let result = Command::new(resource_compiler())
        .args(["/nologo", "/fo"])
        .arg(&compiled)
        .arg(&script)
        .output()
        .expect("Windows SDK resource compiler (rc.exe) is required for the app icon");
    assert!(
        result.status.success(),
        "Could not compile Diskvale resources: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("cargo::rustc-link-arg-bin=canopy={}", compiled.display());
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=assets/brand/diskvale.ico");
    println!("cargo::rerun-if-env-changed=RC");
    println!("cargo::rerun-if-env-changed=WindowsSdkVerBinPath");
    // Published builds request administrator consent before main. Development
    // builds use main's token guard so Cargo's UI test harness can run normally.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        embed_brand_resources();
        println!("cargo::rustc-link-arg-bin=canopy=/MANIFEST:EMBED");
        let level = if std::env::var("PROFILE").as_deref() == Ok("release") {
            "requireAdministrator"
        } else {
            "asInvoker"
        };
        println!("cargo::rustc-link-arg-bin=canopy=/MANIFESTUAC:level='{level}' uiAccess='false'");
    }
}
