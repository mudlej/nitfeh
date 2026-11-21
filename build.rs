use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;

const STATIC_LIBS: [&str; 5] = ["avformat", "avcodec", "swresample", "avutil", "opus"];

fn main() {
    #[cfg(feature = "gui")]
    compile_ui();

    // The exe's icon, and the manifest that keeps a console from appearing on double-click.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/windows");
        winresource::WindowsResource::new()
            .set_icon("assets/windows/istma.ico")
            .set_manifest_file("assets/windows/app.manifest")
            .compile()
            .expect("the Windows resources do not compile");
    }

    let target = env::var("TARGET").unwrap();
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let deps = env::var_os("ISTMA_DEPS").map(PathBuf::from).unwrap_or_else(|| root.join("work/deps").join(&target));

    println!("cargo:rerun-if-env-changed=ISTMA_DEPS");
    println!("cargo:rerun-if-changed=native/transcode.c");
    println!("cargo:rerun-if-changed=native/transcode.h");
    println!("cargo:rerun-if-changed={}", deps.join(".stamp").display());

    if !deps.join("lib/libavformat.a").exists() {
        panic!(
            "FFmpeg and libopus are not built for {target}: run `just deps {target}` (looked in {})",
            deps.display()
        );
    }

    cc::Build::new()
        .file("native/transcode.c")
        .include("native")
        .include(deps.join("include"))
        .flag_if_supported("-Werror=implicit-function-declaration")
        .flag_if_supported("-Werror=int-conversion")
        .compile("istma_native");

    println!("cargo:rustc-link-search=native={}", deps.join("lib").display());
    for lib in STATIC_LIBS {
        println!("cargo:rustc-link-lib=static={lib}");
    }

    // The system libraries FFmpeg and opus found at configure time (bcrypt on Windows, m on Linux...).
    let mut system = BTreeSet::new();
    let mut frameworks = BTreeSet::new();
    for pc in fs::read_dir(deps.join("lib/pkgconfig")).unwrap().flatten() {
        let text = fs::read_to_string(pc.path()).unwrap_or_default();
        for line in text.lines().filter(|l| l.starts_with("Libs")) {
            let mut words = line.split_whitespace().skip(1);
            while let Some(word) = words.next() {
                if word == "-framework" {
                    frameworks.extend(words.next().map(str::to_owned));
                } else if let Some(lib) = word.strip_prefix("-l")
                    && !STATIC_LIBS.contains(&lib)
                {
                    system.insert(lib.to_owned());
                }
            }
        }
    }
    for lib in system {
        println!("cargo:rustc-link-lib={lib}");
    }
    for framework in frameworks {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

#[cfg(feature = "gui")]
fn compile_ui() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let material = root.join("ui/material/material.slint");
    let config =
        slint_build::CompilerConfiguration::new().with_library_paths([("material".to_owned(), material)].into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the interface does not compile");
}
