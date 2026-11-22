//! The progress fd: a third channel beside stdout and stderr (ADR-0010).

use std::fs::File;
use std::io::Write;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Mutex;

use bitplane_core::{ErrorEnvelope, ProgressEvent};

/// The caller's fd, checked and made close-on-exec.
pub struct ProgressFd {
    file: Mutex<File>,
}

impl ProgressFd {
    /// Takes `fd` over, or says why it cannot be written to — before anything is
    /// fetched or created.
    pub fn adopt(fd: RawFd) -> Result<ProgressFd, ErrorEnvelope> {
        if (0..=2).contains(&fd) {
            return Err(bad_progress_fd(format!(
                "fd {fd} is stdin, stdout or stderr, which already carry something else"
            )));
        }

        // SAFETY: fcntl only inspects or flags the descriptor; nothing is read
        // through it.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 {
            return Err(bad_progress_fd(format!("fd {fd} is not open")));
        }
        if flags & libc::O_ACCMODE == libc::O_RDONLY {
            return Err(bad_progress_fd(format!("fd {fd} is open for reading only")));
        }

        // Not inherited by git or a script: a daemonised child holding the fd
        // would keep the reader from ever seeing EOF.
        // SAFETY: as above.
        unsafe {
            let descriptor = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, descriptor | libc::FD_CLOEXEC);
        }

        Ok(ProgressFd {
            // SAFETY: the fd is open, and nothing else in this process owns it.
            file: Mutex::new(unsafe { File::from_raw_fd(fd) }),
        })
    }

    /// One line, written whole. A failed write is ignored: a reader that went
    /// away costs only the progress.
    pub fn report(&self, event: &ProgressEvent) {
        let line = format!("{}\n", event.to_json());
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn bad_progress_fd(message: String) -> ErrorEnvelope {
    ErrorEnvelope::usage("bad_progress_fd", message)
        .with_remedy("Open the fd for writing before running bp, e.g. `3>progress.jsonl`.")
}
