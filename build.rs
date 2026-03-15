use std::{env, fs, path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=shaders");

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);

    let shaders_dir = Path::new("shaders");
    if !shaders_dir.exists() {
        return;
    }

    for entry in fs::read_dir(shaders_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("slang") {
            let file_stem = path.file_stem().unwrap().to_str().unwrap();
            let out_file = out_path.join(format!("{}.spv", file_stem));

            let status = Command::new("slangc")
                .arg(path.to_str().unwrap())
                .arg("-target").arg("spirv")
                .arg("-o").arg(out_file.to_str().unwrap())
                .status()
                .expect("failed to execute slangc");

            if !status.success() {
                panic!("slangc failed to compile {:?}", path);
            }
        }
    }
}
