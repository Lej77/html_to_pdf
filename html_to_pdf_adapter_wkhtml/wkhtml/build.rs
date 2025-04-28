use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};

trait PathExt {
    fn detect_change(self) -> Self;
    fn detect_folder(self) -> Self;
}
impl<T> PathExt for T
where
    T: AsRef<Path>,
{
    fn detect_change(self) -> Self {
        println!("cargo:rerun-if-changed={}", self.as_ref().display());
        self
    }
    fn detect_folder(self) -> Self {
        let path = self.as_ref().detect_change();
        for path in path.read_dir().expect("Couldn't get items from folder") {
            let path = path
                .expect("Couldn't get an item from a folder")
                .path()
                .detect_change();
            if path.is_dir() {
                path.detect_folder();
            }
        }
        self
    }
}

/// Parse cargo output as `json`. Returns the executable paths found in the parsed output.
pub fn get_executables(mut cargo_process: Child) -> Result<Vec<String>, Option<i32>> {
    use serde::Deserialize;
    use std::io::{self, BufRead};

    #[derive(Deserialize, Debug)]
    #[serde(tag = "reason", rename_all = "kebab-case")]
    enum CargoMessage {
        CompilerArtifact(CargoArtifact),
    }

    #[derive(Deserialize, Debug)]
    struct CargoArtifact {
        /// Path to the built executable.
        executable: Option<String>,
    }

    let mut reader = io::BufReader::new(
        cargo_process
            .stdout
            .as_mut()
            .expect("Failed to open stdout for Cargo."),
    );
    let mut buffer = Default::default();
    let mut first = true;

    let mut executables = Vec::new();
    loop {
        if reader
            .read_line(&mut buffer)
            .map(|v| v == 0)
            .unwrap_or(true)
        {
            break;
        }

        if first && buffer.trim().starts_with("Blocking") {
            panic!("Cargo would block!\nCargo message:{}", buffer);
        }
        first = false;

        let result = serde_json::from_str::<CargoMessage>(&buffer);

        // eprintln!("\n\nCargo stdout: {}\nParsed cargo message: {:?}\n\n", buffer, result);

        if let Ok(CargoMessage::CompilerArtifact(value)) = result {
            if let Some(path) = value.executable {
                executables.push(path);
            }
        }

        buffer.clear();
    }

    let status = cargo_process.wait();
    if status
        .as_ref()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        Ok(executables)
    } else {
        Err(status.ok().and_then(|s| s.code()))
    }
}

fn main() {
    // Activate re-run detection:
    "build.rs".detect_change();

    // Currently can only interact with wkhtml on windows, program will gracefully return an error at runtime on other platforms.
    let is_windows = env::var_os("CARGO_CFG_WINDOWS").is_some();
    if !is_windows {
        println!(
            "cargo:warning=Currently only windows libraries are linked correctly to wkHtmlToPdf."
        );
        return;
    }

    let out_dir = env::var_os("OUT_DIR").unwrap();

    if env::var_os("CARGO_FEATURE_SHOULD_LINK").is_none() {
        // Build another program that uses the "wkhtml-link" crate.
        // The ".dll" file and the "wkhtml-link" program are going to be included in the program that is going to be built.

        // Directory with the cargo manifest that contains this build script:
        let src_dir = env::var_os("CARGO_MANIFEST_DIR").unwrap();

        // Detect changes to the wkhtml_link crate that is a dependency to the wkhtml_runner crate:
        PathBuf::from(&src_dir)
            .join("../wkhtml_link")
            .detect_folder();

        // The directory for the program we want to build:
        let crate_dir = PathBuf::from(&src_dir)
            .join("../wkhtml_runner")
            .detect_folder();

        use std::process::Command;

        let cargo = env::var_os("CARGO").unwrap();
        let target = env::var_os("TARGET").unwrap();
        let profile = env::var_os("PROFILE").unwrap();

        let mut command = Command::new(cargo);
        command
            .current_dir(&crate_dir)
            .arg("build")
            .arg("--target")
            .arg(target)
            // Set a specific target directory so that cargo doesn't block trying to lock a global build target folder:
            .arg("--target-dir")
            .arg(PathBuf::from(&out_dir).join("runner_target"))
            .arg("--message-format")
            .arg("json");
        if profile == "release" {
            command.arg("--release");
        }
        let process = command
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to invoke Cargo to build wkhtml_runner.");

        // Parse cargo's output and find the generated executable.
        let exes = match get_executables(process) {
            Ok(exes) => exes,
            Err(code) => {
                panic!("Cargo build of wkhtml_runner exited with an error (code: {code:?})")
            }
        };
        if exes.len() != 1 {
            panic!("Cargo invocation produced incorrect number of binaries. Expected 1 but found {}.\nFound binaries: {:#?}", exes.len(), exes);
        }
        let exe = &exes[0];

        // Copy the executables into out_dir so that it can be included by the normal code:
        fs::copy(exe, PathBuf::from(&out_dir).join("wkhtml_runner.exe")).unwrap();
    } else {
        // Ensure there is an empty file to include:
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(PathBuf::from(&out_dir).join("wkhtml_runner.exe"))
            .unwrap();
    }
}
