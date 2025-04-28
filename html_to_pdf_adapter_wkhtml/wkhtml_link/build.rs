use std::env;
use std::fs;
use std::path::PathBuf;

use hex_literal::hex;
use sha2::{Digest, Sha256};

fn main() {
    // Get build target info:
    let is_windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    // Directory with the cargo manifest that contains this build script:
    let out_dir = env::var_os("OUT_DIR").unwrap();

    let unpack_exe_installer = |data: &[u8]| {
        let installer_path = PathBuf::from(&out_dir).join("wkhtmltox-0.12.3_msvc2013.exe");
        std::fs::write(&installer_path, data)
            .expect("Failed to write downloaded installer to file");

        let status_7z = std::process::Command::new("7z")
            .arg("e") // extract (without file structure)
            .arg("-bd") // Disable progress indicator
            .arg("-y") // Assume yes on all queries
            .arg("--")
            .arg(installer_path) // The installer's path
            .arg(r"**/wkhtmltox.*") // Only interested in "wkhtmltox.dll" and "wkhtmltox.lib"
            .stdin(std::process::Stdio::null())
            .stdout(std::io::stderr())
            .current_dir(&out_dir)
            .status()
            .expect("Failed to start 7z CLI as child process to extract wkhtmltopdf installer");
        if !status_7z.success() {
            panic!(
                "7z failed to extract wkhtmltopdf installer (code: {:?})",
                status_7z.code()
            );
        }
    };
    println!("cargo::rustc-check-cfg=cfg(supported_target)");

    // Download dll from website and verify SHA256 checksum:
    // https://wkhtmltopdf.org/downloads.html
    // https://github.com/wkhtmltopdf/wkhtmltopdf/releases/0.12.3/
    type TargetInfo<'a> = (&'a str, [u8; 32], Option<&'a dyn Fn(&[u8])>);
    let (download_url, checksum, handle_data): TargetInfo<'_> = match () {
        () if is_windows && target_arch == "x86_64" => (
            "https://github.com/wkhtmltopdf/wkhtmltopdf/releases/download/0.12.3/wkhtmltox-0.12.3_msvc2013-win64.exe",
            hex!("f9fba3e365071ee06f6aa55a5d2db2e0241ebffb33c2ce213c931b28c18b0542"),
            Some(&unpack_exe_installer)
        ),
        () if is_windows && target_arch == "x86" => (
            "https://github.com/wkhtmltopdf/wkhtmltopdf/releases/download/0.12.3/wkhtmltox-0.12.3_msvc2013-win32.exe",
            hex!("1cfe83668c3279b2f4175cd7bdeb7286829a0a49b57fd4ecd225d26da49e5047"),
            Some(&unpack_exe_installer)
        ),
        () if target_os == "macos" && target_arch == "x86_64" => (
            "https://github.com/wkhtmltopdf/wkhtmltopdf/releases/download/0.12.3/wkhtmltox-0.12.3_osx-cocoa-x86-64.pkg",
            hex!("c8aa0f9456444847d08c6c772e2e0e4244813e6a2911eba6c940439df9abd0f0"),
            None
        ),
        () if target_os == "macos" && target_arch == "x86" => (
            "https://github.com/wkhtmltopdf/wkhtmltopdf/releases/download/0.12.3/wkhtmltox-0.12.3_osx-carbon-i386.pkg",
            hex!("6e4613c060eb9e5eb0bd05b0ccd85d09086ef7a1156300e53a9dfba7969b6fc0"),
            None
        ),
        () if target_arch == "aarch64" => (
            "https://github.com/wkhtmltopdf/wkhtmltopdf/releases/download/0.12.3/wkhtmltox-0.12.3_linux-generic-amd64.tar.xz",
            hex!("40bc014d0754ea44bb90e733f03e7c92862f7445ef581e3599ecc00711dddcaa"),
            None
        ),
        () if target_arch == "x86" || target_arch == "x86_64" => (
            "https://github.com/wkhtmltopdf/wkhtmltopdf/releases/download/0.12.3/wkhtmltox-0.12.3_linux-generic-i386.tar.xz",
            hex!("001af3e3030a5418367e5d0ec75cdc1deef6131f0ed51e873c474e4199b8380b"),
            None
        ),
        _ => return, // Unsupported target
    };

    let data = reqwest::blocking::get(download_url)
        .expect("failed to get wkhtmltopdf installer")
        .bytes()
        .expect("download failed while in progress")
        .to_vec();
    let sha256_hash = {
        let mut hasher = Sha256::new();
        hasher.update(data.as_slice());
        hasher.finalize()
    };
    assert_eq!(
        &sha256_hash[..],
        checksum.as_slice(),
        "invalid SHA256 checksum for downloaded v0.12.3 wkhtmltopdf"
    );
    if let Some(handle_data) = handle_data {
        handle_data(&data);
    } else {
        println!("cargo::warning=The current build target has a wkhtml library but the build script doesn't support unpacking it");
        return;
    }

    println!("cargo::rustc-cfg=supported_target");

    if env::var_os("CARGO_FEATURE_SHOULD_LINK").is_some() {
        // Copy library files to out_dir so that the out_dir is added to path when run and test commands are used.
        // This ensures that the dll file is found at runtime for those commands.
        println!(
            "cargo:rustc-link-search=native={}",
            out_dir.to_str().unwrap()
        );
    }

    // Generate compressed include macro with path to ".dll" file since the macro can't specify path's relative to env!("OUT_DIR"):
    fs::write(
        PathBuf::from(&out_dir).join("compressed.rs"),
        format!(
            r#####"include_flate::flate!(pub static WK_HTML_TO_PDF_DLL: [u8] from r####"{}"####);"#####,
            PathBuf::from(&out_dir)
                .join("wkhtmltox.dll")
                .to_str()
                .expect("the OUT_DIR path should be valid UTF-8")
        ),
    )
    .unwrap();
}
