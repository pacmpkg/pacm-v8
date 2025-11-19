use flate2::read::GzDecoder;
use std::ffi::{OsStr, OsString};
use std::fs::{DirEntry, File, Metadata};
use std::io::{BufReader, Error, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, SystemTime};
use std::{collections::HashMap, env, fs};

use reqwest::blocking::Client;

fn extract_tarball(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    fs::create_dir_all(dst)?;
    let file: File = std::fs::File::open(src)?;
    let mut reader: BufReader<File> = BufReader::new(file);
    let mut magic: [u8; 6] = [0; 6];
    let read: usize = reader.read(&mut magic)?;
    reader.seek(SeekFrom::Start(0))?;

    let is_gzip: bool = read >= 2 && magic[0] == 0x1F && magic[1] == 0x8B;

    if is_gzip {
        let decoder: GzDecoder<BufReader<File>> = GzDecoder::new(reader);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(dst)?;
    } else {
        let mut archive = tar::Archive::new(reader);
        archive.unpack(dst)?;
    }
    Ok(())
}

fn download_and_extract(url: &str, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure destination parent exists
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    // Prepare HTTP client with long timeouts and proper UA
    let builder = Client::builder()
        .user_agent("pacm-v8-build/1.0")
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(60));

    let client = builder.build()?;

    // Download with a few retries, streaming to disk
    let tmp: PathBuf = dst.with_extension("download");
    let attempts: usize = 3;
    let mut last_err: Option<String> = None;
    for attempt in 1..=attempts {
        let mut req = client.get(url).header("Accept", "application/octet-stream");
        if let Ok(token) = env::var("GITHUB_TOKEN") {
            if !token.is_empty() {
                req = req.bearer_auth(token);
            }
        }

        match req.send() {
            Ok(mut resp) => {
                if !resp.status().is_success() {
                    last_err = Some(format!("HTTP status {}", resp.status()));
                } else {
                    let mut file = File::create(&tmp)?;
                    std::io::copy(&mut resp, &mut file)?;
                    // Try to extract and return
                    extract_tarball(&tmp, dst)?;
                    let _ = std::fs::remove_file(&tmp);
                    return Ok(());
                }
            }
            Err(err) => {
                last_err = Some(err.to_string());
            }
        }

        if attempt < attempts {
            let backoff = 2_u64.pow(attempt as u32);
            println!(
                "cargo:warning=Download attempt {attempt} failed; retrying in {backoff}s"
            );
            std::thread::sleep(Duration::from_secs(backoff));
        }
    }

    Err(format!(
        "Download failed after {attempts} attempts: {}",
        last_err.unwrap_or_else(|| "unknown error".into())
    )
    .into())
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

fn has_prebuilt_markers(path: &Path) -> bool {
    path.join("include").exists()
        || path.join("v8.h").exists()
        || path.join("lib").join("v8_monolith.lib").exists()
        || path.join("v8_monolith.lib").exists()
        || path.join("lib").join("libv8_monolith.a").exists()
        || path.join("libv8_monolith.a").exists()
        || path.join("lib").join("libv8_monolith.so").exists()
        || path.join("libv8_monolith.so").exists()
        || path.join("lib").join("libv8_monolith.dylib").exists()
        || path.join("libv8_monolith.dylib").exists()
}

fn resolve_prebuilt_root(base: &Path) -> PathBuf {
    if has_prebuilt_markers(base) {
        return base.to_path_buf();
    }

    let mut subdirs: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path: PathBuf = entry.path();
            if path.is_dir() {
                if has_prebuilt_markers(&path) {
                    return path;
                }
                subdirs.push(path);
            }
        }
    }

    if subdirs.len() == 1 {
        if let Some(dir) = subdirs.pop() {
            return resolve_prebuilt_root(&dir);
        }
    }

    base.to_path_buf()
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

    // Always pin to the current crate version's prebuilt tag
    let repo: String = env::var("V8_PREBUILT_REPO").unwrap_or_else(|_| "pacmpkg/pacm-v8".to_string());
    let effective_tag: String = format!("v8-{crate_version}");
    println!("cargo:rustc-env=PACM_V8_PREBUILT_TAG={effective_tag}");

    // Asset name convention
    let filename: String = format!("v8-{target_triple}.tar.gz");
    let download_url: String =
        format!("https://github.com/{repo}/releases/download/{effective_tag}/{filename}");

    // Always download (per crate version), isolate cache path by tag to avoid cross-version reuse
    let v8_dst: PathBuf = out_dir.join(format!("v8-prebuilt-{}-{}", target_triple, effective_tag));
    if v8_dst.exists() && v8_dst.join("include").exists() {
        println!("Found existing v8 prebuilt at {}", v8_dst.display());
    } else {
        println!(
            "Downloading v8 prebuilt from: {}",
            download_url
        );
        download_and_extract(&download_url, &v8_dst)
            .expect("Failed to download or extract v8 prebuilt. Please check if your system and architecture are supported.");
    }

    let v8_root: PathBuf = resolve_prebuilt_root(&v8_dst);

    // Erwartete Layout nach dem Extrahieren:
    // v8-prebuilt/include/...
    // v8-prebuilt/lib/<platform-libname>
    // normalize possible locations/platforms:
    let mut include_path: PathBuf = v8_root.join("include");
    if !include_path.exists() {
        // try if include is at root (tarball unpacked differently)
        if v8_root.join("v8.h").exists() {
            include_path = v8_root.clone();
        }
    }

    // Platform-specific library naming
    let is_windows: bool = cargo_target.contains("windows");
    let is_macos: bool = cargo_target.contains("apple-darwin");
    let is_linux: bool = cargo_target.contains("linux");
    let is_musl: bool = cargo_target.contains("musl");
    // Prefer new unified lib/ directory layout (monolith inside lib/). Retain backward-compatible root fallbacks.
    let lib_candidates: Vec<PathBuf> = if is_windows {
        vec![
            v8_root.join("lib").join("v8_monolith.lib"),
            v8_root.join("v8_monolith.lib"),
        ]
    } else if is_macos {
        vec![
            v8_root.join("lib").join("libv8_monolith.a"),
            v8_root.join("libv8_monolith.a"),
            v8_root.join("lib").join("libv8_monolith.dylib"),
            v8_root.join("libv8_monolith.dylib"),
        ]
    } else {
        vec![
            v8_root.join("lib").join("libv8_monolith.a"),
            v8_root.join("libv8_monolith.a"),
            v8_root.join("lib").join("libv8_monolith.so"),
            v8_root.join("libv8_monolith.so"),
        ]
    };
    let lib_path: PathBuf = lib_candidates
        .into_iter()
        .find(|p: &PathBuf| p.exists())
        .unwrap_or_else(|| {
            panic!(
            "Could not find v8 monolithic library in prebuilt at {}. Expected lib in root or lib/.",
            v8_root.display()
        )
        });

    if let Some(icu_src) = find_icudtl_dat(&v8_root, &manifest_dir) {
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

    let config_info: Option<(serde_json::Value, PathBuf)> = load_v8_build_config(&v8_root, &manifest_dir);

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

    let candidate_lib_dir: PathBuf = v8_root.join("lib");
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

                let libplatform_dir: PathBuf = obj_dir.join("libplatform");
                if libplatform_dir.exists() {
                    // Prefer existing libs if present
                    let mut had_lib = false;
                    if let Ok(entries) = fs::read_dir(&libplatform_dir) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("lib") || s.eq_ignore_ascii_case("a")).unwrap_or(false) {
                                had_lib = true;
                                break;
                            }
                        }
                    }

                    if had_lib {
                        collect_libs_recursively(
                            &libplatform_dir,
                            &mut extra_libs,
                            &mut link_search_dirs,
                        );
                    } else if let Some(lib_path) = create_static_library_from_objects(
                        &libplatform_dir,
                        &out_dir,
                        &manifest_dir,
                        &format!(
                            "v8_libplatform_{}",
                            sanitize_ident(build.file_name().to_string_lossy().as_ref())
                        ),
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
