//! The workflow of a downstream tool (a terrain builder) that uses only the public API:
//! generate a key, convert tiles to PAA on many threads, pack and sign, read entries back.
#![cfg(all(feature = "sign", feature = "image"))]

use std::fs::{self, File};
use std::io::BufReader;
use std::thread;

use dayzmake_core::image::{DynamicImage, Rgba, RgbaImage};
use dayzmake_core::paa::{Paa, PaaFormat};
use dayzmake_core::pbo::{PBO, PackOptions};
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign, BISignVersion, signature_path};
use dayzmake_core::warning::Warnings;

fn tile(seed: u8) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(64, 64, |x, y| {
        let v = u8::try_from((x * 3 + y * 2) % 256).unwrap_or(0);
        Rgba([v, seed.wrapping_mul(17), 255 - v, 255])
    }))
}

#[test]
fn naves_workflow_in_parallel() {
    let tmp = tempfile::tempdir().unwrap();
    let secrets = tmp.path().join("secrets");
    let stage = tmp.path().join("stage");
    let data = stage.join("data");
    let addons = tmp.path().join("addons");
    let keys = tmp.path().join("keys");
    for dir in [&secrets, &data, &addons, &keys] {
        fs::create_dir_all(dir).unwrap();
    }

    // 1. Key generation on the first build.
    let key = BIPrivateKey::generate(1024, "naves_test".to_string()).unwrap();
    key.write(&mut File::create(secrets.join("naves_test.biprivatekey")).unwrap())
        .unwrap();
    let public = key.to_public_key().unwrap();
    public
        .write(&mut File::create(secrets.join("naves_test.bikey")).unwrap())
        .unwrap();

    // 2. Tiles on 16 threads, each with its own warnings.
    let jobs: Vec<(String, PaaFormat, &str)> = (0..8)
        .map(|i| (format!("s_{i}.paa"), PaaFormat::Dxt1, "DXT1"))
        .chain((0..8).map(|i| (format!("m_{i}.paa"), PaaFormat::Dxt5, "DXT5")))
        .collect();
    // Each thread also packs its own folder with `PackOptions::build()`: an already rapified
    // `.cpp` makes `from_directory` warn once per file. The thread count of such files varies, and
    // every sink must hold exactly its own warnings.
    let results: Vec<Warnings> = thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .enumerate()
            .map(|(n, (name, format, _))| {
                let data = &data;
                let root = tmp.path();
                scope.spawn(move || {
                    let mut warnings = Warnings::new();
                    let image = tile(u8::try_from(n).unwrap());
                    let paa = Paa::from_dynamic_image(&image, *format).unwrap();
                    let mut out = File::create(data.join(name)).unwrap();
                    paa.write(&mut out).unwrap();

                    let own = root.join(format!("own_{n}"));
                    fs::create_dir_all(&own).unwrap();
                    for i in 0..=(n % 5) {
                        fs::write(own.join(format!("t{n}_{i}.cpp")), b"\0raP\0\0\0\0").unwrap();
                    }
                    PBO::from_directory(&own, &PackOptions::build(), None, &mut warnings).unwrap();
                    warnings
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (n, warnings) in results.iter().enumerate() {
        let mut files: Vec<&str> = warnings
            .warnings()
            .iter()
            .map(|w| w.file.as_deref().unwrap_or(""))
            .collect();
        files.sort_unstable();
        let expected: Vec<String> = (0..=(n % 5)).map(|i| format!("t{n}_{i}.cpp")).collect();
        assert_eq!(files, expected, "thread {n}: {:?}", warnings.warnings());
    }
    for (name, _, compression) in &jobs {
        let mut file = BufReader::new(File::open(data.join(name)).unwrap());
        let paa = Paa::read(&mut file).unwrap();
        assert_eq!(paa.compression(), *compression, "{name}");
        assert_eq!((paa.width(), paa.height()), (64, 64), "{name}");
    }

    // 3. Pack, sign, publish the public key.
    let mut warnings = Warnings::new();
    let pbo = PBO::from_directory(&stage, &PackOptions::pack(), None, &mut warnings).unwrap();
    assert!(warnings.warnings().is_empty(), "{:?}", warnings.warnings());
    let pbo_path = addons.join("naves_test.pbo");
    pbo.write(&mut File::create(&pbo_path).unwrap()).unwrap();
    // A later build reads the key back from secrets/.
    let key = BIPrivateKey::read(&mut File::open(secrets.join("naves_test.biprivatekey")).unwrap())
        .unwrap();
    let signature = key.sign(&pbo, BISignVersion::V3).unwrap();
    let sig_path = signature_path(&pbo_path, key.name()).unwrap();
    signature
        .write(&mut File::create(&sig_path).unwrap())
        .unwrap();
    fs::copy(
        secrets.join("naves_test.bikey"),
        keys.join("naves_test.bikey"),
    )
    .unwrap();

    let written = PBO::read(&mut BufReader::new(File::open(&pbo_path).unwrap())).unwrap();
    let public =
        BIPublicKey::read(&mut File::open(keys.join("naves_test.bikey")).unwrap()).unwrap();
    let signature = BISign::read(&mut File::open(&sig_path).unwrap()).unwrap();
    public.verify(&written, &signature).unwrap();

    // 4. Read selected entries without loading the rest.
    let wanted = ["data\\s_3.paa", "data\\m_5.paa"];
    let selection = PBO::read_entries(&mut File::open(&pbo_path).unwrap(), |name| {
        wanted.contains(&name)
    })
    .unwrap();
    assert_eq!(selection.entries().len(), 2);
    for entry in selection.entries() {
        let file = entry.name().rsplit('\\').next().unwrap();
        assert_eq!(entry.data().unwrap(), fs::read(data.join(file)).unwrap());
    }
}
