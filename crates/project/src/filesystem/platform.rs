//! Native filesystem operations.

use orlo_utils::objc::{class, objc_msgSend, pool, sel, Id};
use std::ffi::{c_char, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

/// Move `path` to the Trash as Finder does (`NSFileManager trashItemAtURL:`), a link itself
/// rather than what it points to. Where it went, or why not.
pub fn trash(path: &Path) -> std::io::Result<PathBuf> {
    let link_or_file = std::fs::symlink_metadata(path)?;
    let is_dir = link_or_file.is_dir();
    let bytes = CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    pool(|| {
        // SAFETY: each objc_msgSend is cast to the message's signature; every pointer passed is
        // a live object from the runtime or a NUL-terminated buffer outliving the call.
        unsafe {
            let id: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let url_of: unsafe extern "C" fn(Id, Id, *const c_char, u8, Id) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let trash_item: unsafe extern "C" fn(Id, Id, Id, *mut Id, *mut Id) -> u8 =
                std::mem::transmute(objc_msgSend as *const ());
            let utf8: unsafe extern "C" fn(Id, Id) -> *const c_char = std::mem::transmute(objc_msgSend as *const ());
            let fm = id(class(c"NSFileManager"), sel(c"defaultManager"));
            let url = url_of(
                class(c"NSURL"),
                sel(c"fileURLWithFileSystemRepresentation:isDirectory:relativeToURL:"),
                bytes.as_ptr(),
                is_dir as u8,
                std::ptr::null_mut(),
            );
            if fm.is_null() || url.is_null() {
                return Err(std::io::Error::other("the Trash is not available"));
            }
            let (mut out, mut err): (Id, Id) = (std::ptr::null_mut(), std::ptr::null_mut());
            let sel_trash = sel(c"trashItemAtURL:resultingItemURL:error:");
            if trash_item(fm, sel_trash, url, &mut out, &mut err) != 0 {
                let p = (!out.is_null())
                    .then(|| utf8(out, sel(c"fileSystemRepresentation")))
                    .filter(|p| !p.is_null())
                    .map(|p| PathBuf::from(std::ffi::OsStr::from_bytes(CStr::from_ptr(p).to_bytes())));
                return Ok(p.unwrap_or_default());
            }
            let why = (!err.is_null())
                .then(|| id(err, sel(c"localizedDescription")))
                .filter(|d| !d.is_null())
                .map(|d| utf8(d, sel(c"UTF8String")))
                .filter(|p| !p.is_null())
                .map(|p| CStr::from_ptr(p).to_string_lossy().into_owned())
                .unwrap_or_else(|| "the file could not be moved to the Trash".into());
            Err(std::io::Error::other(why))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_and_a_link_go_to_the_trash_and_the_test_takes_them_back_out() {
        let dir = std::env::temp_dir().join(format!("orlo-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target")).unwrap();
        let file = dir.join("orlo-trash-test.txt");
        std::fs::write(&file, "x").unwrap();
        let link = dir.join("orlo-trash-link");
        std::os::unix::fs::symlink(dir.join("target"), &link).unwrap();
        for p in [&file, &link] {
            let went = trash(p).unwrap();
            assert!(std::fs::symlink_metadata(p).is_err(), "gone from its folder");
            // the user's Trash is left as it was
            if std::fs::symlink_metadata(&went).is_ok() {
                std::fs::remove_file(&went).unwrap();
            }
        }
        assert!(dir.join("target").is_dir(), "the link's target stays");
        assert!(trash(&dir.join("missing")).is_err());
    }
}
