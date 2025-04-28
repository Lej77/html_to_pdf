fn main() {
    #[cfg(feature = "include_exe")]
    {
        use std::fs;
        use std::path::PathBuf;

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let dst = PathBuf::from(&out_dir).join("./HtmlToPdf");
        fs::create_dir_all(&dst)
            .expect("Failed to create directory to contain built artifacts in OUT_DIR");

        let runtime = dotnet_cli::DotNetRuntimeIdentifier::from_build_env_vars()
            .expect("Failed to determine .Net runtime identifier for target triple");

        let build_status = dotnet_cli::DotNetInvoker::new()
            .project_path("./HtmlToPdf")
            .publish()
            .runtime(runtime)
            .configuration(dotnet_cli::DotNetConfiguration::release())
            .self_contained(true)
            .artifacts_dir(dst.to_str().expect("OUT_DIR should be UTF8").to_owned())
            .output_dir(format!("{out_dir}/HtmlToPdf_Publish"))
            .get_command()
            .arg("--property:PublishAot=true")
            .arg("./HtmlToPdf.csproj")
            .status()
            .unwrap();
        assert!(
            build_status.success(),
            "Build of C# HtmlToPdf should succeed."
        );

        // Generate compressed include macro with path to ".dll" file since the macro can't specify path's relative to env!("OUT_DIR"):
        fs::write(
        PathBuf::from(&out_dir).join("compressed.rs"),
        format!(
            r#####"
fn embedded_converter() -> &'static [u8] {{
    ::include_flate::flate!(pub static EMBEDDED_CONVERTER_DATA: [u8] from r####"{}"####);
    &*EMBEDDED_CONVERTER_DATA
}}
"#####,
            PathBuf::from(&out_dir)
                .join(format!(
                    "HtmlToPdf_Publish/HtmlToPdf{}",
                    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() { ".exe" } else { "" }
                ))
                .to_str()
                .expect("the OUT_DIR should be valid UTF-8")
        ),
    )
    .unwrap();
    }
}
