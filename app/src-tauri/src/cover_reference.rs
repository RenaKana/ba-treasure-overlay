use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SAMPLE_SIZE: usize = 32;

const READ_ERROR: &str = "未翻开样本文件无法读取，请清除或重新选择";
const SAVE_ERROR: &str = "未翻开样本保存失败，请重试";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CoverSample {
    pub rgb: Vec<[u8; 3]>,
}

impl CoverSample {
    pub fn valid(&self) -> bool {
        self.rgb.len() == SAMPLE_SIZE * SAMPLE_SIZE
    }
}

#[derive(Serialize, Deserialize)]
struct CoverReferenceFile {
    version: u32,
    sample_size: usize,
    samples: Vec<CoverSample>,
}

pub fn load(path: &Path) -> Result<Vec<CoverSample>, String> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(READ_ERROR.to_owned()),
    };

    let document: CoverReferenceFile =
        serde_json::from_slice(&contents).map_err(|_| READ_ERROR.to_owned())?;
    if document.version != 1
        || document.sample_size != SAMPLE_SIZE
        || document.samples.iter().any(|sample| !sample.valid())
    {
        return Err(READ_ERROR.to_owned());
    }

    Ok(document.samples)
}

pub fn save(path: &Path, samples: &[CoverSample]) -> Result<(), String> {
    if samples.iter().any(|sample| !sample.valid()) {
        return Err(SAVE_ERROR.to_owned());
    }

    let document = CoverReferenceFile {
        version: 1,
        sample_size: SAMPLE_SIZE,
        samples: samples.to_vec(),
    };
    let contents = serde_json::to_vec(&document).map_err(|_| SAVE_ERROR.to_owned())?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|_| SAVE_ERROR.to_owned())?;
    }

    let (temporary_path, mut temporary_file) =
        create_temporary_file(path).map_err(|_| SAVE_ERROR.to_owned())?;
    let write_result = temporary_file
        .write_all(&contents)
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);

    if write_result.is_err() || fs::rename(&temporary_path, path).is_err() {
        let _ = fs::remove_file(&temporary_path);
        return Err(SAVE_ERROR.to_owned());
    }

    Ok(())
}

fn create_temporary_file(destination: &Path) -> io::Result<(PathBuf, File)> {
    static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if destination.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination has no file name",
        ));
    }

    loop {
        let id = NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!(".cover-reference-{}-{id}.tmp", std::process::id());
        let temporary_path = parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();

            loop {
                let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "ba-cover-reference-test-{}-{now}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("could not create test directory: {error}"),
                }
            }
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample(seed: u8) -> CoverSample {
        CoverSample {
            rgb: (0..SAMPLE_SIZE * SAMPLE_SIZE)
                .map(|index| [seed.wrapping_add(index as u8), seed, 255 - seed])
                .collect(),
        }
    }

    #[test]
    fn roundtrips_multiple_samples_and_an_empty_list() {
        let directory = TestDirectory::new();
        let path = directory.file("samples.json");
        let samples = vec![sample(3), sample(71), sample(190)];

        assert!(load(&path).unwrap().is_empty());
        save(&path, &samples).unwrap();
        assert_eq!(load(&path).unwrap(), samples);

        save(&path, &[]).unwrap();
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn replaces_an_existing_file() {
        let directory = TestDirectory::new();
        let path = directory.file("samples.json");
        let previous = vec![sample(12)];
        let replacement = vec![sample(29), sample(88)];

        save(&path, &previous).unwrap();
        save(&path, &replacement).unwrap();

        assert_eq!(load(&path).unwrap(), replacement);
    }

    #[test]
    fn rejects_corrupt_unsupported_and_invalid_sample_documents() {
        let directory = TestDirectory::new();
        let corrupt = directory.file("corrupt.json");
        fs::write(&corrupt, b"{not-json").unwrap();
        assert_eq!(load(&corrupt).unwrap_err(), READ_ERROR);

        let unsupported = directory.file("unsupported.json");
        fs::write(
            &unsupported,
            br#"{"version":2,"sample_size":32,"samples":[]}"#,
        )
        .unwrap();
        assert_eq!(load(&unsupported).unwrap_err(), READ_ERROR);

        let unsupported_size = directory.file("unsupported-size.json");
        fs::write(
            &unsupported_size,
            br#"{"version":1,"sample_size":31,"samples":[]}"#,
        )
        .unwrap();
        assert_eq!(load(&unsupported_size).unwrap_err(), READ_ERROR);

        let invalid_shape = directory.file("invalid-shape.json");
        fs::write(
            &invalid_shape,
            br#"{"version":1,"sample_size":32,"samples":[{"rgb":[]}] }"#,
        )
        .unwrap();
        assert_eq!(load(&invalid_shape).unwrap_err(), READ_ERROR);
    }

    #[test]
    fn failed_save_leaves_the_previous_file_unchanged() {
        let directory = TestDirectory::new();
        let path = directory.file("samples.json");
        let previous = vec![sample(44)];
        save(&path, &previous).unwrap();
        let original_contents = fs::read(&path).unwrap();

        let invalid = CoverSample { rgb: Vec::new() };
        assert_eq!(save(&path, &[invalid]).unwrap_err(), SAVE_ERROR);

        assert_eq!(fs::read(&path).unwrap(), original_contents);
        assert_eq!(load(&path).unwrap(), previous);
    }
}
