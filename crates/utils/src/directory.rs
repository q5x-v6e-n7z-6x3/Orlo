//! Owned directory streams for operations relative to validated descriptors.

use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::fd::IntoRawFd;

pub struct Directory(*mut libc::DIR);

impl Directory {
    pub fn new(file: File) -> io::Result<Self> {
        let fd = file.into_raw_fd();
        // SAFETY: fd is owned here and transfers to the stream only on success.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: fdopendir failed, leaving this descriptor owned here.
            unsafe {
                libc::close(fd);
            }
            return Err(error);
        }
        Ok(Self(stream))
    }

    /// The returned name is valid until the next read or this stream is dropped.
    pub fn next_name(&mut self) -> io::Result<Option<&CStr>> {
        // SAFETY: errno is thread-local and the stream remains owned through this read.
        unsafe {
            *libc::__error() = 0;
            let entry = libc::readdir(self.0);
            if entry.is_null() {
                return if *libc::__error() == 0 { Ok(None) } else { Err(io::Error::last_os_error()) };
            }
            Ok(Some(CStr::from_ptr((*entry).d_name.as_ptr())))
        }
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: this object exclusively owns a non-null directory stream.
        unsafe {
            libc::closedir(self.0);
        }
    }
}
