//! Provides a typed API to the `dotnet` CLI tool used to build .NET applications.
//!
//! # Building a self contained .NET Core application
//!
//! To build a self contained .NET Core application and cleaning up after the following steps should be preformed:
//!
//! 1. dotnet publish -c Release -r win10-x64 --self-contained true -o "%OUT_DIR%/%DEFINED_NAME_HERE%/"
//! 2. dotnet clean -c Release -r win10-x64
//! 3. The "%OUT_DIR%/%DEFINED_NAME_HERE%/" folder can now be compressed into "%OUT_DIR%/%DEFINED_NAME_HERE%.tar"
//! 4. The "%OUT_DIR%/%DEFINED_NAME_HERE%/" folder can now be deleted.
//!
//! # Caching builds
//!
//! To make this process more efficient the build should only be done when we can detect that something has changed.
//! This can be done by checking the last modified time for all source files in the .NET Core project and if any of
//! them was changed after the last ".tar" file was made then we should re-build the project.
//!
//! Note that the ".tar" should only be re-used if the `dotnet` command line arguments is the same as when it was built.
//! If for example the `DotNetRuntimeIdentifier` has been changed then a new build must be made. This means that we must
//! keep track of which arguments were used in the last build. This could be serialized into a separate file named something
//! like "%OUT_DIR%/%DEFINED_NAME_HERE%.nfo"

use std::borrow::Cow;
use std::convert::AsMut;
use std::iter;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::process::Command;

#[macro_use]
mod helper_macros;

pub trait DotNetCommandLineOption {
    fn value(&self) -> &str;
    fn flag() -> &'static str;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetFrameWork(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetFrameWork, "--framework");

/// Specifies the OS and architecture.
///
/// For more info, see: [.NET Runtime Identifier (RID) catalog - .NET |
/// Microsoft Learn](https://learn.microsoft.com/en-us/dotnet/core/rid-catalog)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetRuntimeIdentifier(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetRuntimeIdentifier, "--runtime");
impl DotNetRuntimeIdentifier {
    /// Attempts to determine the [dotnet architecture command line argument](https://docs.microsoft.com/en-us/dotnet/core/rid-catalog) from
    /// [Rust configuration options](https://doc.rust-lang.org/reference/conditional-compilation.html).
    ///
    /// This will match the architecture of the currently running program.
    #[allow(unreachable_code)]
    pub fn from_config() -> Option<Self> {
        macro_rules! from_cfg {
            ( $( $value:literal if [$($cfg_token:tt)*] ),* $(,)? ) => {
                $(
                    #[cfg($($cfg_token)*)]
                    return Some($value.into());
                )*
            };
        }
        #[cfg(windows)]
        {
            // Windows:
            from_cfg! {
                "win-x64" if [target_arch = "x86_64"],
                "win-x86" if [target_arch = "x86"],
                "win-arm" if [target_arch = "arm"],
                "win-arm64" if [target_arch = "aarch64"],
            }
        }
        #[cfg(unix)]
        {
            from_cfg! {
                // macOS:
                "osx-x64" if [any(target_os = "macos", target_vendor = "apple", target_arch = "x86_64")],
                "osx-arm64" if [any(target_os = "macos", target_vendor = "apple")],
                // Linux
                "linux-musl-x64" if [all(target_arch = "x86_64", target_env = "musl")],
                "linux-x64" if [target_arch = "x86_64"],
                "linux-arm" if [target_arch = "arm"],
            }
        }

        None
    }

    /// Attempts to determine the [dotnet architecture command line argument](https://docs.microsoft.com/en-us/dotnet/core/rid-catalog) from
    /// a [rust target triple](https://doc.rust-lang.org/cargo/appendix/glossary.html#target). A target triple has the format
    /// `"<arch><sub>-<vendor>-<sys>-<abi>"` but some triples don't include all of that information and will have less than 3 `'-'` characters.
    ///
    /// Use `rustc --print target-list` or `rustup target list` to list possible target triples or check [here](https://forge.rust-lang.org/platform-support.html)
    /// for information about what targets are most supported.
    pub fn from_target_triple(target_triple: &str) -> Option<Self> {
        let mut split = target_triple.split('-');
        let arch = split.next()?;

        let rest: Vec<_> = split.collect();
        let is_windows = rest.iter().any(|v| *v == "windows");
        let target_abi = rest.last().copied();

        macro_rules! from_expr {
            ( $( $value:literal if $check:expr ),* $(,)? ) => {
                $(
                    if $check {
                        return Some($value.into());
                    }
                )*
            };
        }

        if is_windows {
            // Windows:
            from_expr! {
                "win-x64" if arch.starts_with("x86_64"),
                "win-x86" if arch.starts_with("i686"),
                "win-arm" if arch.starts_with("arm"),
                "win-arm64" if arch.starts_with("aarch64"),
            }
        } else {
            let is_apple = rest.iter().any(|v| *v == "apple");

            if is_apple {
                // macOS:
                from_expr! {
                    "osx" if arch.starts_with("i686"),
                    "osx-x64" if arch.starts_with("x86_64"),
                    "osx-arm64" if arch.starts_with("aarch64"),
                }
            } else {
                // Linux
                from_expr! {
                    "linux-musl-x64" if arch.starts_with("x86_64") && target_abi == Some("musl"),
                    "linux-musl-arm64" if arch.starts_with("aarch64") && target_abi == Some("musl"),
                    "linux-x64" if arch.starts_with("x86_64"),
                    "linux-arm" if arch.starts_with("arm"),
                    "linux-arm64" if arch.starts_with("aarch64"),
                }
            }
        }

        None
    }

    /// Attempts to determine the [dotnet architecture command line argument](https://docs.microsoft.com/en-us/dotnet/core/rid-catalog) from
    /// [Rust build script environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-build-scripts).
    ///
    /// When used in a build script this will match the architecture of the program that is going to be built.
    pub fn from_build_env_vars() -> Option<Self> {
        let target_triple = std::env::var("TARGET").ok()?;
        Self::from_target_triple(&target_triple)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetConfiguration(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetConfiguration, "--configuration");
impl DotNetConfiguration {
    pub fn debug() -> Self {
        "Debug".into()
    }
    pub fn release() -> Self {
        "Release".into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetVerbosity(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetVerbosity, "--verbosity");
impl DotNetVerbosity {
    pub fn quiet() -> Self {
        "quiet".into()
    }
    pub fn minimal() -> Self {
        "minimal".into()
    }
    pub fn normal() -> Self {
        "normal".into()
    }
    pub fn detailed() -> Self {
        "detailed".into()
    }
    pub fn diagnostic() -> Self {
        "diagnostic".into()
    }
}

/// All build output files from the executed command will go in subfolders under
/// the specified path, separated by project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetArtifactsDir(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetArtifactsDir, "--artifacts-path");


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetOutput(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetOutput, "--output");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetSelfContained(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetSelfContained, "--self-contained");
impl From<bool> for DotNetSelfContained {
    fn from(value: bool) -> Self {
        DotNetSelfContained(Cow::from(if value { "true" } else { "false" }))
    }
}

/// The directory to restore packages to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotNetRestorePackagesDir(pub Cow<'static, str>);
impl_dot_cli_option!(DotNetRestorePackagesDir, "--packages");

fn create_arg_iter<'a>(
    command: &'a str,
    mut value: Option<Cow<'a, str>>,
) -> impl Iterator<Item = Cow<'a, str>> {
    let mut counter = 0;
    iter::from_fn(move || {
        if value.is_none() {
            None
        } else {
            let value = match counter {
                0 => Some(Cow::from(command)),
                1 => value.take(),
                _ => return None,
            };
            counter += 1;
            value
        }
    })
}

fn create_arg_iter_from_cli_option<O>(option: Option<&O>) -> impl Iterator<Item = &str>
where
    O: DotNetCommandLineOption,
{
    create_arg_iter(
        O::flag(),
        option.map(DotNetCommandLineOption::value).map(Cow::from),
    )
    .filter_map(|v| {
        if let Cow::Borrowed(v) = v {
            Some(v)
        } else {
            None
        }
    })
}

pub trait DotNetCommand {
    fn get_args<'a, R>(&'a self, f: impl FnOnce(&mut dyn Iterator<Item = &'a str>) -> R) -> R;
}

define_command!(
    #[command = "publish"]
    #[derive(Clone, Debug, Default)]
    pub struct Publish {
        configuration: DotNetConfiguration,
        framework: DotNetFrameWork,
        runtime: DotNetRuntimeIdentifier,
        self_contained: DotNetSelfContained,
        output: DotNetOutput,
        artifacts_dir: DotNetArtifactsDir,
        verbosity: DotNetVerbosity,
    },
    From(Build, Restore, Clean)
);

define_command!(
    #[command = "build"]
    #[derive(Clone, Debug, Default)]
    pub struct Build {
        configuration: DotNetConfiguration,
        framework: DotNetFrameWork,
        runtime: DotNetRuntimeIdentifier,
        self_contained: DotNetSelfContained,
        output: DotNetOutput,
        artifacts_dir: DotNetArtifactsDir,
        verbosity: DotNetVerbosity,
    },
    From(Publish, Restore, Clean)
);

define_command!(
    #[command = "restore"]
    #[derive(Clone, Debug, Default)]
    pub struct Restore {
        packages: DotNetRestorePackagesDir,
        runtime: DotNetRuntimeIdentifier,
        verbosity: DotNetVerbosity,
    },
    From(Publish, Build, Clean)
);

define_command!(
    #[command = "clean"]
    /// Note that the `DotNetOutput` option should be the build output and not the publish output, otherwise some
    /// build files might not be cleaned correctly. That means that the `DotNetOutput` should be `None` to clean
    /// after a `Publish` command.
    #[derive(Clone, Debug, Default)]
    pub struct Clean {
        configuration: DotNetConfiguration,
        framework: DotNetFrameWork,
        runtime: DotNetRuntimeIdentifier,
        output: DotNetOutput,
        verbosity: DotNetVerbosity,
    },
    From(Publish, Restore, Build)
);

#[derive(Clone, Debug)]
pub struct DotNetInvoker<C> {
    command_data: C,
    project_path: Option<PathBuf>,
}
impl DotNetInvoker<()> {
    pub fn new() -> Self {
        Self {
            command_data: (),
            project_path: None,
        }
    }

    pub fn publish(self) -> DotNetInvoker<Publish> {
        DotNetInvoker {
            command_data: Default::default(),
            project_path: self.project_path,
        }
    }
    pub fn build(self) -> DotNetInvoker<Build> {
        DotNetInvoker {
            command_data: Default::default(),
            project_path: self.project_path,
        }
    }
    pub fn restore(self) -> DotNetInvoker<Restore> {
        DotNetInvoker {
            command_data: Default::default(),
            project_path: self.project_path,
        }
    }
    pub fn clean(self) -> DotNetInvoker<Clean> {
        DotNetInvoker {
            command_data: Default::default(),
            project_path: self.project_path,
        }
    }
}
impl Default for DotNetInvoker<()> {
    fn default() -> Self {
        Self::new()
    }
}
impl<C> DotNetInvoker<C> {
    /// The path of the .NET Core project. Will be used as the current working directory when `dotnet` is invoked.
    pub fn project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = Some(path.into());
        self
    }
    /// Convert this command into another command and keep arguments that are used for the new command.
    pub fn into_command<D>(self) -> DotNetInvoker<D>
    where
        D: From<C>,
    {
        DotNetInvoker {
            command_data: self.command_data.into(),
            project_path: self.project_path,
        }
    }
}
impl<C> DotNetInvoker<C>
where
    C: DotNetCommand,
{
    pub fn get_command(&self) -> Command {
        let mut command = Command::new("dotnet");
        command.stdout(std::io::stderr());
        if let Some(path) = self.project_path.as_ref() {
            // Start with the project's path as the current working directory:
            command.current_dir(path);
        }
        self.command_data.get_args(|args| {
            // Apply command's arguments:
            command.args(args);
        });
        command
    }
    pub fn invoke(&self) -> std::io::Result<std::process::ExitStatus> {
        self.get_command().status()
    }
}
/// Allow calling methods that are implemented on the command struct.
impl<C> Deref for DotNetInvoker<C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.command_data
    }
}
impl<C> DerefMut for DotNetInvoker<C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.command_data
    }
}
setter!(packages, DotNetRestorePackagesDir);
setter!(framework, DotNetFrameWork);
setter!(runtime, DotNetRuntimeIdentifier);
setter!(configuration, DotNetConfiguration);
setter!(verbosity, DotNetVerbosity);
setter!(self_contained, DotNetSelfContained);
setter!(output_dir, DotNetOutput);
setter!(artifacts_dir, DotNetArtifactsDir);
