use flate2::read::GzDecoder;
use bytes::Bytes;
use std::io::Error;
use std::time::SystemTime;
use reqwest::blocking::{RequestBuilder, Response};
use std::ffi::{OsStr, OsString};
use std::fs::{DirEntry, File, Metadata};
use std::process::{ExitStatus, Command};
use std::path::{Path, PathBuf};
use std::{collections::HashMap, env, fs};

use reqwest::blocking::Client;

fn extract_tarball(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    fs::create_dir_all(dst)?;
    let file: File = std::fs::File::open(src)?;
    let ar: GzDecoder<File> = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(ar);
    archive.unpack(dst)?;
    Ok(())
}

fn download_and_extract(url: &str, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::blocking::get(url)?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: {}", resp.status()).into());
    }
    let bytes: Bytes = resp.bytes()?;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp: PathBuf = dst.with_extension("tar.gz");
    std::fs::write(&tmp, &bytes)?;
    extract_tarball(&tmp, dst)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn resolve_latest_release_tag(repo: &str) -> Option<String> {
    let client: Client = Client::builder()
        .user_agent("pacm-v8-build/1.0")
        .build()
        .ok()?;

    let mut request: RequestBuilder = client.get(format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ));

    if let Ok(token) = env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
    }

    let response: Response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }

    let payload: serde_json::Value = response.json().ok()?;
    payload
        .get("tag_name")
        .and_then(|name| name.as_str())
        .map(|name: &str| name.to_string())
}

fn library_spec(path: &Path) -> Option<(String, &'static str)> {
    let filename: &str = path.file_name()?.to_str()?;
    let lowercase: String = filename.to_ascii_lowercase();

    const PRIMARY_ARTIFACTS: [&str; 4] = [
        "v8_monolith.lib",
        "libv8_monolith.a",
        "libv8_monolith.so",
        "libv8_monolith.dylib",
    ];

    if PRIMARY_ARTIFACTS
        .iter()
        .any(|candidate: &&str| lowercase == *candidate)
    {
        return None;
    }

    if let Some(name) = filename.strip_suffix(".lib") {
        return Some((name.to_string(), "static"));
    }

    if let Some(name) = filename.strip_suffix(".a") {
        let lib_name: String = name.strip_prefix("lib").unwrap_or(name).to_string();
        return Some((lib_name, "static"));
    }

    if let Some(name) = filename.strip_suffix(".dylib") {
        let lib_name: String = name.strip_prefix("lib").unwrap_or(name).to_string();
        return Some((lib_name, "dylib"));
    }

    if let Some(index) = filename.find(".so") {
        let base: &str = &filename[..index];
        if !base.is_empty() {
            let lib_name: String = base.strip_prefix("lib").unwrap_or(base).to_string();
            return Some((lib_name, "dylib"));
        }
    }

    None
}

fn collect_libs_recursively(
    dir: &Path,
    libs: &mut HashMap<String, &'static str>,
    search_dirs: &mut Vec<PathBuf>,
) {
    if !dir.exists() {
        return;
    }

    let entries: Vec<_> = match fs::read_dir(dir) {
        Ok(iter) => iter.filter_map(Result::ok).map(|e: DirEntry| e.path()).collect(),
        Err(_) => return,
    };

    for path in &entries {
        if path.is_file() {
            if let Some((name, kind)) = library_spec(path) {
                libs.entry(name).or_insert(kind);
                if let Some(parent) = path.parent() {
                    if !search_dirs.iter().any(|p: &PathBuf| p == parent) {
                        search_dirs.push(parent.to_path_buf());
                    }
                }
            }
        }
    }

    for path in entries {
        if path.is_dir() {
            collect_libs_recursively(&path, libs, search_dirs);
        }
    }
}

fn sanitize_ident(component: &str) -> String {
    component
        .chars()
        .map(|c: char| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn find_program_on_path(candidates: &[&str]) -> Option<PathBuf> {
    let path_var: OsString = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        for candidate in candidates {
            let path: PathBuf = dir.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

fn find_fallback_librarian(manifest_dir: &Path, os: &str) -> Option<PathBuf> {
    let fallback: PathBuf = manifest_dir
        .join("third_party")
        .join("v8")
        .join("buildtools")
        .join(os)
        .join("llvm-build")
        .join("Release+Asserts")
        .join("bin")
        .join(if os == "windows" { "lib.exe" } else { "lib" });
    if fallback.exists() {
        return Some(fallback);
    }
    None
}

fn find_librarian(manifest_dir: &Path) -> Option<PathBuf> {
    let win_candidates: [&str; 4] = ["lib.exe", "LLVM-LIB.EXE", "llvm-lib.exe", "LLVM-LIB.EXE"];
    let unix_candidates: [&str; 3] = ["lib", "LLVM-LIB", "llvm-lib"];

    if cfg!(target_os = "windows") {
        for candidate in &win_candidates {
            if let Some(path) = find_program_on_path(&[*candidate]) {
                return Some(path);
            }
        }
    }

    if cfg!(not(target_os = "windows")) {
        for candidate in &unix_candidates {
            if let Some(path) = find_program_on_path(&[*candidate]) {
                return Some(path);
            }
        }
    }

    let os: &str = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };

    if let Some(path) = find_fallback_librarian(manifest_dir, os) {
        return Some(path);
    }

    None
}

fn find_icudtl_dat(v8_root: &Path, manifest_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        v8_root.join("icudtl.dat"),
        v8_root.join("lib").join("icudtl.dat"),
        v8_root.join("data").join("icudtl.dat"),
        v8_root.join("resources").join("icudtl.dat"),
    ];

    // Search nested directories commonly used when unpacking prebuilts.
    if let Ok(entries) = fs::read_dir(v8_root) {
        for entry in entries.flatten() {
            let path: PathBuf = entry.path();
            if path.is_dir() {
                candidates.push(path.join("icudtl.dat"));
            }
        }
    }

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    let local_out: PathBuf = manifest_dir.join("third_party").join("v8").join("out.gn");
    if let Ok(entries) = fs::read_dir(&local_out) {
        for entry in entries.flatten() {
            let candidate: PathBuf = entry.path().join("icudtl.dat");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn find_v8_build_config_path(v8_root: &Path, manifest_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        v8_root.join("v8_build_config.json"),
        v8_root.join("lib").join("v8_build_config.json"),
    ];

    if let Ok(entries) = fs::read_dir(v8_root) {
        for entry in entries.flatten() {
            let path: PathBuf = entry.path();
            if path.is_dir() {
                candidates.push(path.join("v8_build_config.json"));
            }
        }
    }

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    let local_out: PathBuf = manifest_dir.join("third_party").join("v8").join("out.gn");
    if let Ok(entries) = fs::read_dir(&local_out) {
        for entry in entries.flatten() {
            let candidate: PathBuf = entry.path().join("v8_build_config.json");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn load_v8_build_config(
    v8_root: &Path,
    manifest_dir: &Path,
) -> Option<(serde_json::Value, PathBuf)> {
    let path: PathBuf = find_v8_build_config_path(v8_root, manifest_dir)?;
    let content: String = fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some((value, path))
}

fn latest_mtime(paths: &[PathBuf]) -> Option<std::time::SystemTime> {
    paths
        .iter()
        .filter_map(|p| fs::metadata(p).ok()?.modified().ok())
        .max()
}

fn should_regenerate(target: &Path, inputs: &[PathBuf]) -> bool {
    if !target.exists() {
        return true;
    }
    let target_m: SystemTime = match fs::metadata(target).and_then(|m: Metadata| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };

    match latest_mtime(inputs) {
        Some(newest) => newest > target_m,
        None => true,
    }
}

fn create_static_library_from_objects(
    obj_dir: &Path,
    out_dir: &Path,
    manifest_dir: &Path,
    lib_basename: &str,
) -> Option<PathBuf> {
    if !obj_dir.exists() {
        return None;
    }

    let mut objects: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(obj_dir).ok()? {
        let path: PathBuf = entry.ok()?.path();
        if path
            .extension()
            .and_then(|ext: &OsStr| ext.to_str())
            .map(|ext: &str| ext.eq_ignore_ascii_case("obj"))
            .unwrap_or(false)
        {
            objects.push(path);
        }
    }

    if objects.is_empty() {
        return None;
    }

    let lib_path: PathBuf = out_dir.join(format!("{lib_basename}.lib"));
    if !should_regenerate(&lib_path, &objects) {
        return Some(lib_path);
    }

    let librarian: PathBuf = match find_librarian(manifest_dir) {
        Some(p) => p,
        None => {
            println!(
                "cargo:warning=Could not find lib/llvm-lib; skipping custom libc++ bundling"
            );
            return None;
        }
    };

    let rsp_path: PathBuf = out_dir.join(format!("{lib_basename}.rsp"));
    let mut rsp_content: String = String::new();
    rsp_content.push_str("/nologo\n");
    rsp_content.push_str(&format!("/OUT:\"{}\"\n", lib_path.display()));
    for obj in &objects {
        rsp_content.push_str(&format!("\"{}\"\n", obj.display()));
    }

    if let Err(err) = fs::write(&rsp_path, rsp_content) {
        panic!(
            "Failed to write response file for libc++ bundling at {}: {}",
            rsp_path.display(),
            err
        );
    }

    let status: ExitStatus = Command::new(&librarian)
        .arg(format!("@{}", rsp_path.display()))
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "Failed to invoke {} to bundle libc++ objects: {}",
                librarian.display(),
                err
            );
        });

    if !status.success() {
        panic!(
            "{} failed while creating {}; see output above for details",
            librarian.display(),
            lib_path.display()
        );
    }

    let _ = fs::remove_file(&rsp_path);

    Some(lib_path)
}

fn main() {
    for file in [
        "shim.h",
        "shim_internal.h",
        "runtime.cc",
        "context.cc",
        "script.cc",
        "util.cc",
    ] {
        println!("cargo:rerun-if-changed=src/cpp/{file}");
    }

    let out_dir: PathBuf = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cargo_target: String = env::var("TARGET").unwrap();
    let manifest_dir: PathBuf = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let crate_version: String = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is not set");
    println!("cargo:rustc-env=PACM_V8_VERSION={crate_version}");

    let target_triple: String = env::var("V8_PREBUILT_TARGET")
        .or_else(|_| env::var("V8_TARGET_TRIPLE"))
        .unwrap_or_else(|_| cargo_target.clone());

    // Repo + Tag konfigurierbar
    let repo: String = env::var("V8_PREBUILT_REPO").unwrap_or_else(|_| "pacmpkg/pacm-v8".to_string());
    let default_tag: String = format!("v8-{crate_version}");
    let tag_env: Option<String> = env::var("V8_PREBUILT_TAG").ok();
    let mut effective_tag: String = tag_env.clone().unwrap_or_else(|| default_tag.clone());
    let mut use_latest_endpoint: bool = false;

    if effective_tag.eq_ignore_ascii_case("latest") {
        if let Some(resolved) = resolve_latest_release_tag(&repo) {
            println!("cargo:warning=Resolved V8 prebuilt release tag {resolved}");
            effective_tag = resolved;
        } else {
            println!(
                "cargo:warning=Could not resolve latest V8 release tag via GitHub API; falling back to releases/latest endpoint"
            );
            use_latest_endpoint = true;
        }
    } else if !effective_tag.starts_with("v8-") {
        effective_tag = format!("v8-{effective_tag}");
    }

    // Asset name convention
    let filename: String = format!("v8-{target_triple}.tar.gz");
    if !use_latest_endpoint {
        println!("cargo:rustc-env=PACM_V8_PREBUILT_TAG={effective_tag}");
    }

    let url: String = if use_latest_endpoint {
        format!("https://github.com/{repo}/releases/latest/download/{filename}")
    } else {
        format!("https://github.com/{repo}/releases/download/{effective_tag}/{filename}")
    };

    // Determine prebuilt location preference order:
    // 1) V8_PREBUILT_DIR (explicit override)
    // 2) local checkout prebuilt/<target_triple>
    // 3) unpacked artifacts/v8-<target_triple>
    // 4) download or unpack artifact into OUT_DIR/v8-prebuilt-<target_triple>
    let local_prebuilt: PathBuf = manifest_dir.join("prebuilt").join(&target_triple);
    let local_artifact_dir: PathBuf = manifest_dir.join("artifacts");
    let unpacked_artifact: PathBuf = local_artifact_dir.join(format!("v8-{target_triple}"));
    let local_artifact: PathBuf = local_artifact_dir.join(&filename);

    let v8_dst: PathBuf = if let Ok(dir) = env::var("V8_PREBUILT_DIR") {
        let p: PathBuf = PathBuf::from(dir);
        println!("Using V8 prebuilts from V8_PREBUILT_DIR={}", p.display());
        p
    } else if local_prebuilt.exists() {
        println!("Using local prebuilts at {}", local_prebuilt.display());
        local_prebuilt
    } else if unpacked_artifact.join("include").exists() || unpacked_artifact.join("v8.h").exists()
    {
        println!("Using unpacked artifact at {}", unpacked_artifact.display());
        unpacked_artifact
    } else {
        let dl: PathBuf = out_dir.join(format!("v8-prebuilt-{target_triple}"));

        if dl.exists() && dl.join("include").exists() {
            println!("Found existing v8 prebuilt at {}", dl.display());
        } else if local_artifact.exists() {
            println!("Unpacking local artifact {}", local_artifact.display());
            extract_tarball(&local_artifact, &dl)
                .expect("Failed to unpack local V8 artifact. Regenerate with scripts/build_v8.py.");
        } else {
            println!("Downloading v8 prebuilt from: {url}");
            download_and_extract(&url, &dl).expect("Failed to download or extract v8 prebuilt. Set V8_PREBUILT_REPO/TAG env vars or run scripts/build_v8.py to create prebuilds.");
        }
        dl
    };

    // Erwartete Layout nach dem Extrahieren:
    // v8-prebuilt/include/...
    // v8-prebuilt/lib/<platform-libname>
    // normalize possible locations/platforms:
    let mut include_path: PathBuf = v8_dst.join("include");
    if !include_path.exists() {
        // try if include is at root (tarball unpacked differently)
        if v8_dst.join("v8.h").exists() {
            include_path = v8_dst.clone();
        }
    }

    // Platform-specific library naming
    let is_windows: bool = cargo_target.contains("windows");
    let is_macos: bool = cargo_target.contains("apple-darwin");
    let is_linux: bool = cargo_target.contains("linux");
    let is_musl: bool = cargo_target.contains("musl");
    let lib_candidates: Vec<PathBuf> = if is_windows {
        vec![
            v8_dst.join("v8_monolith.lib"),
            v8_dst.join("lib").join("v8_monolith.lib"),
        ]
    } else {
        vec![
            v8_dst.join("libv8_monolith.a"),
            v8_dst.join("lib").join("libv8_monolith.a"),
        ]
    };
    let lib_path: PathBuf = lib_candidates
        .into_iter()
        .find(|p: &PathBuf| p.exists())
        .unwrap_or_else(|| {
            panic!(
            "Could not find v8 monolithic library in prebuilt at {}. Expected lib in root or lib/.",
            v8_dst.display()
        )
        });

    if let Some(icu_src) = find_icudtl_dat(&v8_dst, &manifest_dir) {
        let icu_dst: PathBuf = out_dir.join("icudtl.dat");
        if should_regenerate(&icu_dst, &[icu_src.clone()]) {
            fs::copy(&icu_src, &icu_dst).unwrap_or_else(|err: Error| {
                panic!(
                    "Failed to copy icudtl.dat from {} to {}: {}",
                    icu_src.display(),
                    icu_dst.display(),
                    err
                );
            });
        }
        println!(
            "cargo:rustc-env=PACM_V8_ICU_DATA_PATH={}",
            icu_dst.display()
        );
        println!("cargo:rerun-if-changed={}", icu_src.display());
    } else {
        println!(
            "cargo:warning=Could not locate icudtl.dat for V8 prebuilts; ICU-dependent features may be unavailable."
        );
    }

    let config_info: Option<(serde_json::Value, PathBuf)> = load_v8_build_config(&v8_dst, &manifest_dir);

    let lib_filename: &str = lib_path
        .file_name()
        .and_then(|s: &OsStr| s.to_str())
        .expect("Library path is not valid UTF-8");
    let mut lib_name: &str = lib_filename
        .trim_end_matches(".a")
        .trim_end_matches(".lib")
        .trim_end_matches(".so")
        .trim_end_matches(".dylib");
    if let Some(stripped) = lib_name.strip_prefix("lib") {
        lib_name = stripped;
    }

    let link_kind: String = env::var("V8_LINK_KIND").unwrap_or_else(|_| {
        match lib_path.extension().and_then(|ext: &OsStr| ext.to_str()) {
            Some("so") | Some("dylib") => "dylib".to_string(),
            _ => "static".to_string(),
        }
    });

    // Compile shim.cc and link against prebuilt V8
    let mut build: cc::Build = cc::Build::new();
    build.cpp(true).include("src/cpp").include(&include_path);

    for source in [
        "src/cpp/runtime.cc",
        "src/cpp/context.cc",
        "src/cpp/script.cc",
        "src/cpp/util.cc",
    ] {
        build.file(source);
    }

    let is_msvc = cargo_target.contains("msvc");
    if is_msvc {
        build.flag_if_supported("/std:c++20");
        build.flag_if_supported("/Zc:__cplusplus");
        build.static_crt(true);
    } else {
        build.flag_if_supported("-std=c++20");
    }

    if let Some((config, config_path)) = config_info {
        println!("cargo:rerun-if-changed={}", config_path.display());
        if config
            .get("pointer_compression")
            .and_then(|v: &serde_json::Value| v.as_bool())
            .unwrap_or(false)
        {
            build.define("V8_COMPRESS_POINTERS", None);
            if config
                .get("pointer_compression_shared_cage")
                .and_then(|v: &serde_json::Value| v.as_bool())
                .unwrap_or(false)
            {
                build.define("V8_COMPRESS_POINTERS_IN_SHARED_CAGE", None);
            }
        }
        if config
            .get("sandbox")
            .and_then(|v: &serde_json::Value| v.as_bool())
            .unwrap_or(false)
        {
            build.define("V8_ENABLE_SANDBOX", None);
        }
    }

    // If there are additional platform flags, add them via env vars if needed
    if let Ok(extra) = env::var("CXXFLAGS") {
        for flag in extra.split_whitespace() {
            build.flag(flag);
        }
    }

    build.compile("shim");

    // Linker flags to use the prebuilt library
    // Link search path:
    let lib_dir: &Path = lib_path.parent().unwrap();
    let mut link_search_dirs: Vec<PathBuf> = vec![lib_dir.to_path_buf()];
    let mut extra_libs: HashMap<String, &'static str> = HashMap::new();

    let candidate_lib_dir: PathBuf = v8_dst.join("lib");
    if candidate_lib_dir.exists() {
        collect_libs_recursively(&candidate_lib_dir, &mut extra_libs, &mut link_search_dirs);
    }

    let local_v8_out: PathBuf = manifest_dir.join("third_party").join("v8").join("out.gn");
    if local_v8_out.exists() {
        if let Ok(builds) = fs::read_dir(&local_v8_out) {
            for build in builds.flatten() {
                if build
                    .file_name()
                    .to_string_lossy()
                    .contains("host_build_tools")
                {
                    continue;
                }

                let obj_dir: PathBuf = build.path().join("obj");
                if !obj_dir.exists() {
                    continue;
                }
                let third_party: PathBuf = obj_dir.join("third_party");
                if third_party.exists() {
                    collect_libs_recursively(
                        &third_party.join("icu"),
                        &mut extra_libs,
                        &mut link_search_dirs,
                    );
                }

                if is_windows {
                    let libcxx_obj: PathBuf = obj_dir
                        .join("buildtools")
                        .join("third_party")
                        .join("libc++")
                        .join("libc++");
                    if libcxx_obj.exists() {
                        let build_name: OsString = build.file_name();
                        let build_tag: std::borrow::Cow<'_, str> = build_name.to_string_lossy();
                        let lib_basename: String =
                            format!("v8_libcxx_{}", sanitize_ident(build_tag.as_ref()));
                        if let Some(lib_path) = create_static_library_from_objects(
                            &libcxx_obj,
                            &out_dir,
                            &manifest_dir,
                            &lib_basename,
                        ) {
                            if let Some(parent) = lib_path.parent() {
                                if !link_search_dirs.iter().any(|p: &PathBuf| p == parent) {
                                    link_search_dirs.push(parent.to_path_buf());
                                }
                            }
                            if let Some(stem) = lib_path.file_stem().and_then(|s: &std::ffi::OsStr| s.to_str()) {
                                extra_libs.entry(stem.to_string()).or_insert("static");
                            }
                        }
                    }
                }
            }
        }
    }

    extra_libs.remove(lib_name);

    for dir in &link_search_dirs {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    println!("cargo:rustc-link-lib={link_kind}={lib_name}");

    let mut extra_pairs: Vec<_> = extra_libs.into_iter().collect();
    extra_pairs.sort_by(|a: &(String, &str), b: &(String, &str)| a.0.cmp(&b.0));
    for (name, kind) in extra_pairs {
        println!("cargo:rustc-link-lib={kind}={name}");
    }

    if is_windows {
        for lib in [
            "dbghelp", "winmm", "ws2_32", "user32", "advapi32", "ole32", "oleaut32", "shell32",
        ] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    } else if is_macos {
        for lib in ["c++", "m", "pthread"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
        for framework in ["CoreFoundation", "CoreServices"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    } else if is_linux {
        for lib in ["stdc++", "m", "pthread", "dl"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
        if !is_musl {
            println!("cargo:rustc-link-lib=dylib=rt");
        }
    }

    // Provide include location for crate users (optional)
    println!("cargo:include={}", include_path.display());
}
