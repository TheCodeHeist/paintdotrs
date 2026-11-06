//! Builds Paint.RS packages for Linux: a .deb (Debian, Ubuntu, Mint, Pop!_OS...) and an .rpm
//! (Fedora, openSUSE, RHEL/Alma/Rocky...), from the program built by `cargo build --release`.
//!
//! Run it through ../package.sh. Everything is plain Rust, so no C compiler or system packaging
//! tools are needed.

use std::io::Write;
use std::path::{Path, PathBuf};

const NAME: &str = "paint-rs";
const SUMMARY: &str = "Layered image and photo editor";
const DESCRIPTION: &str = "Paint.RS is a layered image editor for Linux with selection, painting, shape, \
text and gradient tools, 14 layer blend modes, unlimited undo history, adjustments and effects.";
const MAINTAINER: &str = "Paint.RS <paint-rs@localhost>";
const LICENSE: &str = "LicenseRef-Unspecified";

/// Files the package installs: (source, destination, permissions).
fn payload(app: &Path) -> Vec<(PathBuf, &'static str, u32)> {
    vec![
        (app.join("target/release/pixel_easel"), "/usr/bin/paint-rs", 0o755),
        (app.join("packaging/paint-rs.desktop"), "/usr/share/applications/paint-rs.desktop", 0o644),
        (app.join("packaging/paint-rs.svg"), "/usr/share/icons/hicolor/scalable/apps/paint-rs.svg", 0o644),
        (app.join("packaging/THIRD-PARTY-NOTICES.txt"), "/usr/share/doc/paint-rs/THIRD-PARTY-NOTICES.txt", 0o644),
    ]
}

fn version(app: &Path) -> String {
    let toml = std::fs::read_to_string(app.join("Cargo.toml")).expect("read Cargo.toml");
    toml.lines()
        .find_map(|l| l.strip_prefix("version = \"").map(|v| v.trim_end_matches('"').to_string()))
        .expect("version in Cargo.toml")
}

fn main() {
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let files = payload(&app);
    for (src, _, _) in &files {
        if !src.exists() {
            eprintln!("missing {} - run `cargo build --release` in the app folder first", src.display());
            std::process::exit(1);
        }
    }
    let ver = version(&app);
    let dist = app.join("dist");
    std::fs::create_dir_all(&dist).expect("create dist/");

    let deb = dist.join(format!("{NAME}_{ver}_amd64.deb"));
    build_deb(&files, &ver, &deb).expect("build .deb");
    println!("wrote {}", deb.display());

    let rpm_path = dist.join(format!("{NAME}-{ver}-1.x86_64.rpm"));
    build_rpm(&files, &ver, &rpm_path).expect("build .rpm");
    println!("wrote {}", rpm_path.display());
}

// ---------------- .deb ----------------

fn tar_gz(entries: &[(String, Vec<u8>, u32)], with_dirs: bool) -> std::io::Result<Vec<u8>> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let mut tar = tar::Builder::new(gz);
    let mtime = 1_700_000_000;
    if with_dirs {
        // every parent folder, once, in order
        let mut dirs: Vec<String> = Vec::new();
        for (path, _, _) in entries {
            let mut acc = String::from(".");
            for part in path.trim_start_matches("./").split('/').collect::<Vec<_>>().split_last().unwrap().1 {
                acc = format!("{acc}/{part}");
                if !dirs.contains(&acc) { dirs.push(acc.clone()); }
            }
        }
        for d in dirs {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Directory);
            h.set_mode(0o755);
            h.set_size(0);
            h.set_mtime(mtime);
            h.set_uid(0);
            h.set_gid(0);
            tar.append_data(&mut h, format!("{d}/"), std::io::empty())?;
        }
    }
    for (path, data, mode) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(*mode);
        h.set_size(data.len() as u64);
        h.set_mtime(mtime);
        h.set_uid(0);
        h.set_gid(0);
        tar.append_data(&mut h, path, data.as_slice())?;
    }
    tar.into_inner()?.finish()
}

fn build_deb(files: &[(PathBuf, &str, u32)], ver: &str, out: &Path) -> std::io::Result<()> {
    let mut data_entries = Vec::new();
    let mut total = 0usize;
    for (src, dest, mode) in files {
        let bytes = std::fs::read(src)?;
        total += bytes.len();
        data_entries.push((format!(".{dest}"), bytes, *mode));
    }
    let control = format!(
        "Package: {NAME}\nVersion: {ver}\nArchitecture: amd64\nMaintainer: {MAINTAINER}\nInstalled-Size: {}\n\
         Depends: libc6 (>= 2.34), libgcc-s1, libxkbcommon0, libegl1 | libgl1, libwayland-client0 | libx11-6\n\
         Section: graphics\nPriority: optional\nDescription: {SUMMARY}\n {DESCRIPTION}\n",
        total.div_ceil(1024)
    );
    let control_tar = tar_gz(&[("./control".into(), control.into_bytes(), 0o644)], false)?;
    let data_tar = tar_gz(&data_entries, true)?;

    let file = std::fs::File::create(out)?;
    let mut ar = ar::Builder::new(file);
    let mut add = |name: &str, data: &[u8]| -> std::io::Result<()> {
        let mut h = ar::Header::new(name.as_bytes().to_vec(), data.len() as u64);
        h.set_mode(0o100644);
        h.set_mtime(1_700_000_000);
        ar.append(&h, data)
    };
    add("debian-binary", b"2.0\n")?;
    add("control.tar.gz", &control_tar)?;
    add("data.tar.gz", &data_tar)?;
    let mut f = ar.into_inner()?;
    f.flush()
}

// ---------------- .rpm ----------------

fn build_rpm(files: &[(PathBuf, &str, u32)], ver: &str, out: &Path) -> Result<(), rpm::Error> {
    // the payload is gzip-compressed (the library's default), which every RPM-based distribution reads
    let mut b = rpm::PackageBuilder::new(NAME, ver, LICENSE, "x86_64", SUMMARY);
    b.release("1").description(DESCRIPTION).packager(MAINTAINER).group("Applications/Multimedia");
    for (src, dest, mode) in files {
        b.with_file(src, rpm::FileOptions::new(*dest).permissions(*mode as u16))?;
    }
    // shared libraries by their file names, so the same package works on Fedora, openSUSE and RHEL
    for dep in ["libc.so.6()(64bit)", "libgcc_s.so.1()(64bit)", "libxkbcommon.so.0()(64bit)", "libEGL.so.1()(64bit)"] {
        b.requires(rpm::Dependency::any(dep));
    }
    b.build()?.write_file(out)
}
