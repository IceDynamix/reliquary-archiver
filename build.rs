#[cfg(not(target_os = "windows"))]
fn main() {}

#[cfg(target_os = "windows")]
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

    println!(
        "cargo::rustc-env=LIB={}",
        out_dir.join("Lib").join("x64").to_string_lossy()
    );

    Ok(())
}
