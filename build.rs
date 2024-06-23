#[cfg(not(target_os = "windows"))]
fn main() {}

#[cfg(all(
    target_os = "windows",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn main() -> anyhow::Result<()> {
    let out_dir = std::env::var("OUT_DIR")?;
    let out_dir = std::path::Path::new(&out_dir);

    let mut bytes = Vec::new();
    ureq::get("https://npcap.com/dist/npcap-sdk-1.13.zip")
        .call()?
        .into_reader()
        .read_to_end(&mut bytes)?;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;

        let outpath = match file.enclosed_name() {
            Some(path) => path,
            None => continue,
        };

        if !outpath.starts_with("Lib") {
            continue;
        }

        let outpath = out_dir.join(outpath);

        if !file.is_dir() {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                }
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }

    let arch_dir = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "ARM64"
    } else {
        unreachable!();
    };

    println!(
        "cargo::rustc-env=LIB={}",
        out_dir.join("Lib").join(arch_dir).to_string_lossy()
    );

    Ok(())
}
