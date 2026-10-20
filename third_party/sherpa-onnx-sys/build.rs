use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

const VERSION: &str = "1.13.8";
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const LIBRARIES: &[&str] = &[
    "sherpa-onnx-c-api",
    "sherpa-onnx-core",
    "kaldi-decoder-core",
    "sherpa-onnx-kaldifst-core",
    "sherpa-onnx-fstfar",
    "sherpa-onnx-fst",
    "kaldi-native-fbank-core",
    "kissfft-float",
    "onnxruntime",
    "ssentencepiece_core",
];

fn main() {
    if let Err(error) = build() {
        panic!("BoBe's recognition-only speech runtime: {error}");
    }
}

fn build() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=SHERPA_ONNX_ARCHIVE_DIR");
    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }
    if env::var_os("CARGO_FEATURE_SHARED").is_some() {
        return Err("BoBe uses the pinned static recognition-only runtime".into());
    }
    let os = env::var("CARGO_CFG_TARGET_OS")?;
    let arch = env::var("CARGO_CFG_TARGET_ARCH")?;
    let (platform, expected) = match (os.as_str(), arch.as_str()) {
        ("macos", "aarch64") => (
            "osx-arm64",
            "3d7f9b8a496694af13d9802c33b8133231e397bdef302f543d19468765e83136",
        ),
        ("macos", "x86_64") => (
            "osx-x64",
            "8ffc3ede9f997fec547b5c99b8e2073b8e04fd48a5f65e1a6d5d314ad0106ad9",
        ),
        ("linux", "x86_64") => (
            "linux-x64",
            "36f2ebd0b9aa09248ff6461a30dcb7edfe7eecf2deb67dfb0ceb3aaf4906051f",
        ),
        _ => return Err(format!("no verified recognition-only runtime for {os}/{arch}").into()),
    };
    let stem = format!("sherpa-onnx-v{VERSION}-{platform}-static-no-tts-lib");
    let filename = format!("{stem}.tar.bz2");
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    let archive_path = out.join(&filename);
    prepare_archive(&archive_path, &filename, expected)?;
    let extracted = out.join(&stem);
    if extracted.exists() {
        fs::remove_dir_all(&extracted)?;
    }
    let decoder = bzip2::read::BzDecoder::new(File::open(&archive_path)?);
    tar::Archive::new(decoder).unpack(&out)?;
    let lib = extracted.join("lib");
    for entry in fs::read_dir(&lib)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.contains("espeak") || name.contains("piper_phonemize") {
            return Err(
                format!("unexpected speech-synthesis library in ASR archive: {name}").into(),
            );
        }
    }
    println!("cargo::rustc-link-search=native={}", lib.display());
    for name in LIBRARIES {
        if !lib.join(format!("lib{name}.a")).is_file() {
            return Err(format!("recognition-only runtime is missing lib{name}.a").into());
        }
        println!("cargo::rustc-link-lib=static={name}");
    }
    match os.as_str() {
        "macos" => {
            println!("cargo::rustc-link-lib=dylib=c++");
            println!("cargo::rustc-link-lib=framework=Foundation");
        }
        "linux" => {
            for name in ["stdc++", "m", "pthread", "dl"] {
                println!("cargo::rustc-link-lib=dylib={name}");
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn prepare_archive(path: &Path, filename: &str, expected: &str) -> Result<(), Box<dyn Error>> {
    if path.is_file() && verify_archive(path, expected).is_ok() {
        return Ok(());
    }
    let staging = path.with_extension("download");
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut output = File::create(&staging)?;
        let mut input: Box<dyn Read> =
            if let Some(directory) = env::var_os("SHERPA_ONNX_ARCHIVE_DIR") {
                Box::new(File::open(PathBuf::from(directory).join(filename))?)
            } else {
                let url = format!(
                    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v{VERSION}/{filename}"
                );
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_connect(Some(Duration::from_secs(30)))
                    .timeout_recv_response(Some(Duration::from_secs(120)))
                    .timeout_recv_body(Some(Duration::from_secs(900)))
                    .build()
                    .into();
                Box::new(agent.get(&url).call()?.into_body().into_reader())
            };
        let bytes = std::io::copy(&mut input.by_ref().take(MAX_ARCHIVE_BYTES + 1), &mut output)?;
        if bytes > MAX_ARCHIVE_BYTES {
            return Err("native speech archive exceeds its size limit".into());
        }
        output.flush()?;
        output.sync_all()?;
        verify_archive(&staging, expected)?;
        fs::rename(&staging, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

fn verify_archive(path: &Path, expected: &str) -> Result<(), Box<dyn Error>> {
    if fs::metadata(path)?.len() > MAX_ARCHIVE_BYTES {
        return Err("native speech archive exceeds its size limit".into());
    }
    let mut source = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = source.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    let actual: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected {
        return Err("native speech archive SHA-256 mismatch".into());
    }
    Ok(())
}
