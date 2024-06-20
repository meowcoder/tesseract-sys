extern crate bindgen;

use std::env;
use std::fs;
use std::path::PathBuf;
#[cfg(windows)]
use vcpkg;

fn compile_tesseract() -> Vec<String> {
    let lept = std::env::var("DEP_LEPT_ROOT").expect("no DEP_LEPT_ROOT env variable");

    let dst = cmake::Config::new("tesseract")
        .define("Leptonica_DIR", format!("{lept}/lib/cmake/leptonica"))
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("SW_BUILD", "OFF")
        .define("BUILD_TRAINING_TOOLS", "OFF")
        .define("OPENMP_BUILD", "OFF")
        .define("USE_SYSTEM_ICU", "OFF")
        .define("ENABLE_LTO", "OFF")
        .define("ENABLE_NATIVE", "OFF")
        .define("DISABLE_ARCHIVE", "ON")
        .define("DISABLE_CURL", "ON")
        .define("DISABLE_TIFF", "ON")
        .define("DISABLED_LEGACY_ENGINE", "OFF")
        .define("GRAPHICS_DISABLED", "OFF")
        // avoids error in leptonica, due to missing Tiff support
        .cxxflag("-DTESSERACT_DISABLE_DEBUG_FONTS=1")
        // https://github.com/tesseract-ocr/tesseract/pull/2965
        .cxxflag("-DTESSERACT_IMAGEDATA_AS_PIX=1")
        .build();

    std::env::set_var("PKG_CONFIG_PATH", format!("{lept}/lib/pkgconfig:{}/lib/pkgconfig", dst.display()));

    let pk = pkg_config::Config::new().probe("tesseract").unwrap();
    let _ = std::fs::hard_link(dst.join(format!("lib/lib{}.a", pk.libs[0])), dst.join("lib/libtesseract.a"));

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=tesseract");

    vec![dst.join("include").display().to_string()]
}

#[cfg(windows)]
fn find_tesseract_system_lib() -> Vec<String> {
    println!("cargo:rerun-if-env-changed=TESSERACT_INCLUDE_PATHS");
    println!("cargo:rerun-if-env-changed=TESSERACT_LINK_PATHS");
    println!("cargo:rerun-if-env-changed=TESSERACT_LINK_LIBS");

    let vcpkg = || {
        let lib = vcpkg::Config::new().find_package("tesseract").unwrap();

        vec![lib
            .include_paths
            .iter()
            .map(|x| x.to_string_lossy())
            .collect::<String>()]
    };

    let include_paths = env::var("TESSERACT_INCLUDE_PATHS").ok();
    let include_paths = include_paths.as_deref().map(|x| x.split(','));
    let link_paths = env::var("TESSERACT_LINK_PATHS").ok();
    let link_paths = link_paths.as_deref().map(|x| x.split(','));
    let link_libs = env::var("TESSERACT_LINK_LIBS").ok();
    let link_libs = link_libs.as_deref().map(|x| x.split(','));
    if let (Some(include_paths), Some(link_paths), Some(link_libs)) =
        (include_paths, link_paths, link_libs)
    {
        for link_path in link_paths {
            println!("cargo:rustc-link-search={}", link_path)
        }

        for link_lib in link_libs {
            println!("cargo:rustc-link-lib={}", link_lib)
        }

        include_paths.map(|x| x.to_string()).collect::<Vec<_>>()
    } else {
        vcpkg()
    }
}

// we sometimes need additional search paths, which we get using pkg-config
// we can use tesseract installed anywhere on Linux.
// if you change install path(--prefix) to `configure` script.
// set `export PKG_CONFIG_PATH=/path-to-lib/pkgconfig` before.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn find_tesseract_system_lib() -> Vec<String> {
    let pk = pkg_config::Config::new()
        .atleast_version("4.1")
        .probe("tesseract")
        .unwrap();
    // Tell cargo to tell rustc to link the system proj shared library.
    println!("cargo:rustc-link-search=native={:?}", pk.link_paths[0]);
    println!("cargo:rustc-link-lib=tesseract");

    let mut include_paths = pk.include_paths.clone();
    include_paths
        .iter_mut()
        .map(|x| {
            if !x.ends_with("include") {
                x.pop();
            }
            x
        })
        .map(|x| x.to_string_lossy())
        .map(|x| x.to_string())
        .collect::<Vec<String>>()
}

#[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
fn find_tesseract_system_lib() -> Vec<String> {
    println!("cargo:rustc-link-lib=tesseract");
    vec![]
}

fn capi_bindings(clang_extra_include: &[String]) -> bindgen::Bindings {
    let mut capi_bindings = bindgen::Builder::default()
        .header("wrapper_capi.h")
        .allowlist_function("^Tess.*")
        .blocklist_type("Boxa")
        .blocklist_type("Pix")
        .blocklist_type("Pixa")
        .blocklist_type("_IO_FILE")
        .blocklist_type("_IO_codecvt")
        .blocklist_type("_IO_marker")
        .blocklist_type("_IO_wide_data");

    for inc in clang_extra_include {
        capi_bindings = capi_bindings.clang_arg(format!("-I{}", *inc));
    }

    capi_bindings
        .generate()
        .expect("Unable to generate capi bindings")
}

#[cfg(not(target_os = "macos"))]
fn public_types_bindings(clang_extra_include: &[String]) -> String {
    let mut public_types_bindings = bindgen::Builder::default()
        .header("wrapper_public_types.hpp")
        .allowlist_var("^k.*")
        .allowlist_var("^tesseract::k.*")
        .blocklist_item("^kPolyBlockNames")
        .blocklist_item("^tesseract::kPolyBlockNames");

    for inc in clang_extra_include {
        public_types_bindings = public_types_bindings.clang_arg(format!("-I{}", *inc));
    }

    public_types_bindings
        .generate()
        .expect("Unable to generate public types bindings")
        .to_string()
        .replace("tesseract_k", "k")
}

// MacOS clang is incompatible with Bindgen and constexpr
// https://github.com/rust-lang/rust-bindgen/issues/1948
// Hardcode the constants rather than reading them dynamically
#[cfg(target_os = "macos")]
fn public_types_bindings(_clang_extra_include: &[String]) -> &'static str {
    include_str!("src/public_types_bindings_mac.rs")
}

fn main() {
    // Tell cargo to tell rustc to link the system tesseract
    // and leptonica shared libraries.

    println!("cargo:rerun-if-env-changed=TESSERACT_BUNDLE");

    let clang_extra_include = if std::env::var_os("TESSERACT_BUNDLE").is_some() {
        compile_tesseract()
    } else {
        find_tesseract_system_lib()
    };

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    capi_bindings(&clang_extra_include)
        .write_to_file(out_path.join("capi_bindings.rs"))
        .expect("Couldn't write capi bindings!");
    fs::write(
        out_path.join("public_types_bindings.rs"),
        public_types_bindings(&clang_extra_include),
    )
    .expect("Couldn't write public types bindings!");
}
