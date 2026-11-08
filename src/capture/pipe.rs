//! Telling pipes apart from regular files.
//!
//! Whether a pipe will keep delivering can't be known ahead of time, `tcpdump -w - |` and
//! `cat file.pcap |` look the same. What can be known is whether its contents arrive over time
//! (any pipe) and whether it can be reopened once it ends (only a named pipe).

use std::path::Path;

/// Whether `path` is `-`, which stands for stdin
pub fn is_stdin(path: &Path) -> bool {
    path == Path::new("-")
}

/// Whether `path` is a pipe of any kind, whose contents arrive as they're written rather than
/// all being there up front
pub fn is_pipe(path: &Path) -> bool {
    if is_stdin(path) {
        return true;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;

        // Following symlinks, so /dev/stdin and process substitution count too
        std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_fifo())
    }

    // Anonymous pipes don't have a path on Windows, only named ones do
    #[cfg(not(unix))]
    is_named_pipe(path)
}

/// Whether `path` is a pipe that a new writer can connect to once the current one is done, as
/// opposed to a regular file or a one-off pipe like stdin or process substitution
pub fn is_named_pipe(path: &Path) -> bool {
    if is_stdin(path) {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;

        // Not following symlinks, /dev/stdin and /dev/fd/N link to one-off pipes that are fifos too
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_fifo())
    }

    #[cfg(windows)]
    {
        // Named pipes can only be reached through the pipe namespace. Checking the path rather
        // than the file avoids opening it, which would connect to the pipe and use it up.
        let path = path.to_string_lossy().to_ascii_lowercase();
        path.starts_with(r"\\.\pipe\") || path.starts_with(r"\\?\pipe\")
    }

    #[cfg(not(any(unix, windows)))]
    false
}
