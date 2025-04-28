fn main() {
    #[cfg(feature = "include_exe")]
    {
        use std::path::{Path, PathBuf};
        use std::{fs, io};
        use dotnet_cli::DotNetConfiguration;

        if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
            // .Net framework programs can only run on Windows.
            return;
        }

        /// <https://stackoverflow.com/questions/26958489/how-to-copy-a-folder-recursively-in-rust>
        fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
            fs::create_dir_all(&dst)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                let ty = entry.file_type()?;
                if ty.is_dir() {
                    copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
                } else {
                    fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
                }
            }
            Ok(())
        }

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let dst = PathBuf::from(&out_dir).join("./HtmlToPdf_Framework");
        copy_dir_all("./HtmlToPdf_Framework", &dst)
            .expect("Failed to copy C# source code to OUT_DIR");

        let runtime = dotnet_cli::DotNetRuntimeIdentifier::from_build_env_vars()
            .expect("Failed to determine .Net runtime identifier for target triple");

        let build_status = dotnet_cli::DotNetInvoker::new()
            .project_path(&dst)
            .restore()
            .packages("./packages")
            .runtime(runtime.clone())
            .get_command()
            .arg("./HtmlToPdf_Framework.csproj")
            .status()
            .unwrap();
        assert!(build_status.success(), "restore of NuGet packages should succeed");

        let build_status = dotnet_cli::DotNetInvoker::new()
            .project_path(&dst)
            .build()
            .runtime(runtime)
            .configuration(DotNetConfiguration::release())
            .get_command()
            .arg("./HtmlToPdf_Framework.csproj")
            .status()
            .unwrap();
        assert!(build_status.success(), "Build of C# HtmlToPdf_Framework should succeed.");
    }
}
