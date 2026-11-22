//! Writing files without following links someone else planted.
//!
//! A project folder often arrives from someone else (a zip of "here's my session"), and archives
//! can contain symbolic links. `File::create` and `fs::write` follow a link at the final path
//! component, so a link named `song.azproj.tmp` or `Track.wav` pointing at one of the user's own
//! files would be truncated and overwritten the next time AudioZero saved or recorded there.
//!
//! [`create_new`] opens with `O_EXCL`, which never follows a link, and only replaces a leftover
//! *regular* file. [`ffmpeg_path`] keeps a path from being read as an FFmpeg option or URL.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

fn open_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Creates `path` for writing. If a regular file is already there (a leftover temporary file) it
/// is replaced; a symbolic link, folder or anything else is refused, and a link is never followed.
pub fn create_new(path: &Path) -> io::Result<File> {
    match open_new(path) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if std::fs::symlink_metadata(path)?.is_file() {
                std::fs::remove_file(path)?;
                open_new(path)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{} is a link or a folder, so it won't be written to",
                        path.display()
                    ),
                ))
            }
        }
        other => other,
    }
}

/// `path` as an argument for FFmpeg: with a `file:` prefix, so a name like `-i` or
/// `http://host/x.mp4` is a file name and never an option or a network location.
pub fn ffmpeg_path(path: &Path) -> OsString {
    let mut arg = OsString::from("file:");
    arg.push(path);
    arg
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::symlink;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("audiozero-safefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_and_replaces_a_regular_file() {
        let d = dir("regular");
        let p = d.join("a.tmp");
        create_new(&p).unwrap().write_all(b"one").unwrap();
        create_new(&p).unwrap().write_all(b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn never_writes_through_a_link() {
        let d = dir("link");
        let victim = d.join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        let link = d.join("song.azproj.tmp");
        symlink(&victim, &link).unwrap();
        assert!(create_new(&link).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
        // A dangling link is refused too, so it can't be used to create a file elsewhere.
        let missing = d.join("not-there");
        let dangling = d.join("Track.wav");
        symlink(&missing, &dangling).unwrap();
        assert!(create_new(&dangling).is_err());
        assert!(!missing.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ffmpeg_paths_cannot_be_options() {
        assert_eq!(ffmpeg_path(Path::new("-i")), OsString::from("file:-i"));
        assert_eq!(
            ffmpeg_path(Path::new("http://evil/x.mp4")),
            OsString::from("file:http://evil/x.mp4")
        );
    }
}
