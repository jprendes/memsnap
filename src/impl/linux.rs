use std::ops::Range;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr::null_mut;

use libc::{
    MAP_FAILED, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE, MAP_SHARED, PROT_EXEC, PROT_NONE, PROT_READ,
    PROT_WRITE,
};

pub type OwnedFileDescriptor = OwnedFd;
pub type RawFileDescriptor = RawFd;

use super::{effective_size, Access, Snapshot, View, ViewMode};

impl Snapshot {
    pub(super) fn from_file_impl(file: std::fs::File) -> std::io::Result<Self> {
        let size = file.metadata()?.len() as usize;
        let size = size.next_multiple_of(page_size::get());
        let file = file.into();

        Ok(Self { file, size })
    }

    pub(super) fn zeroed_impl(size: usize) -> std::io::Result<Self> {
        let size = size.next_multiple_of(page_size::get());
        let fd = unsafe { libc::memfd_create(c"hyperlight_snapshot".as_ptr() as _, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.set_len(size as u64)?;
        let file = file.into();

        Ok(Self { file, size })
    }

    pub(super) fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl<S> View<S> {
    pub(super) fn new(
        snapshot: S,
        fd: RawFd,
        size: usize,
        mode: ViewMode,
    ) -> std::io::Result<Self> {
        let ptr = unsafe {
            libc::mmap(
                null_mut(),
                effective_size(size),
                PROT_READ | PROT_WRITE,
                mode.as_posix() | MAP_NORESERVE,
                fd,
                0,
            )
        };
        if ptr == MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }

        let ptr = ptr as *mut u8;

        Ok(Self {
            fd,
            ptr,
            size,
            mode,
            _snapshot: snapshot,
        })
    }
}

impl<S> View<S> {
    pub(super) fn restore_impl(&mut self) -> std::io::Result<()> {
        let new_ptr = unsafe {
            libc::mmap(
                self.ptr as _,
                effective_size(self.size),
                PROT_READ | PROT_WRITE,
                self.mode.as_posix() | MAP_NORESERVE | MAP_FIXED,
                self.fd,
                0,
            )
        };
        if new_ptr == MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn protect_impl(
        &mut self,
        offset: Range<usize>,
        allow: Access,
    ) -> std::io::Result<()> {
        let res = unsafe {
            libc::mprotect(
                self.ptr.add(offset.start) as _,
                offset.len(),
                allow.as_posix(),
            )
        };
        if res < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

impl<S> Drop for View<S> {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr as _, self.size);
        }
    }
}

impl Access {
    fn as_posix(&self) -> libc::c_int {
        let mut access = 0;
        if *self == Access::NONE {
            access = PROT_NONE;
        } else {
            if self.contains(Access::READ) {
                access |= PROT_READ;
            }
            if self.contains(Access::WRITE) {
                access |= PROT_WRITE | PROT_READ;
            }
            if self.contains(Access::EXEC) {
                access |= PROT_EXEC;
            }
        }
        access
    }
}

impl ViewMode {
    fn as_posix(&self) -> libc::c_int {
        match self {
            ViewMode::Cow => MAP_PRIVATE,
            ViewMode::Mutable => MAP_SHARED,
        }
    }
}
