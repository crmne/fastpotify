//! Windows executables carry their icon and version information as
//! resources, and link GLEW for libprojectM; everywhere else there is
//! nothing to do.

/// The names vcpkg has given the static GLEW library, best first. The
/// port has moved between them across versions and triplets, and linking
/// the wrong one fails a long way from here, so take whichever is on
/// disk rather than trusting a name.
#[cfg(windows)]
const GLEW_NAMES: &[&str] = &["glew32s", "libglew32", "glew32"];

/// The GLEW library actually sitting in `lib`, by the name the linker
/// wants. `None` means the folder holds no library this build knows, and
/// the caller says so with the listing rather than leaving the linker to
/// report a missing name with nothing to go on.
#[cfg(windows)]
fn glew_library(lib: &std::path::Path) -> Option<&'static str> {
    let present: Vec<String> = std::fs::read_dir(lib)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.eq_ignore_ascii_case("lib") {
                Some(path.file_stem()?.to_str()?.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect();
    GLEW_NAMES
        .iter()
        .copied()
        .find(|name| present.iter().any(|found| found == name))
}

fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/fastpotify.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/fastpotify.ico")
            .set("ProductName", "Fastpotify")
            .set("FileDescription", "Fastpotify");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
        // libprojectM reads OpenGL through GLEW on Windows, and as a static
        // library leaves GLEW itself to whoever links it: the copy vcpkg
        // installed (`vcpkg install glew:x64-windows-static-md`), from the
        // same root projectm-sys builds with.
        if std::env::var_os("CARGO_FEATURE_MILKDROP").is_some() {
            println!("cargo:rerun-if-env-changed=VCPKG_INSTALLATION_ROOT");
            if let Some(root) = std::env::var_os("VCPKG_INSTALLATION_ROOT") {
                let lib = std::path::Path::new(&root)
                    .join("installed")
                    .join("x64-windows-static-md")
                    .join("lib");
                println!("cargo:rustc-link-search=native={}", lib.display());
                match glew_library(&lib) {
                    Some(name) => println!("cargo:rustc-link-lib=static={name}"),
                    None => {
                        // Name what is there. The linker's own complaint
                        // names only what is missing, which is the one
                        // thing already known.
                        let listing = std::fs::read_dir(&lib)
                            .map(|entries| {
                                entries
                                    .flatten()
                                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_else(|error| format!("unreadable: {error}"));
                        println!(
                            "cargo:warning=no GLEW library in {}; it holds: {listing}",
                            lib.display()
                        );
                    }
                }
            }
        }
    }
}
