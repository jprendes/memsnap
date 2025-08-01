use std::ops::Range;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle, RawHandle};

use windows::core::PCSTR;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingA, MapViewOfFile3, UnmapViewOfFile, UnmapViewOfFileEx, VirtualAlloc2,
    VirtualProtect, MEMORY_MAPPED_VIEW_ADDRESS, MEM_PRESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER,
    MEM_RESERVE, MEM_RESERVE_PLACEHOLDER, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READONLY, PAGE_READWRITE,
    PAGE_WRITECOPY,
};

pub type OwnedFileDescriptor = OwnedHandle;
pub type RawFileDescriptor = RawHandle;

use super::{effective_size, Access, Snapshot, View, ViewMode};

impl Snapshot {
    pub(super) fn from_file_impl(file: std::fs::File) -> std::io::Result<Self> {
        let size = file.metadata()?.len() as usize;

        // we need usize to be 8 bytes on Windows so that we can split
        // the size into high and low parts
        const _: () = assert!(std::mem::size_of::<usize>() == 8);

        let size = size.next_multiple_of(page_size::get() as _);
        let (size_low, size_high) = split_size(effective_size(size));

        let handle = unsafe {
            CreateFileMappingA(
                HANDLE(file.as_raw_handle()),
                None,
                PAGE_EXECUTE_READWRITE,
                size_high as _,
                size_low as _,
                PCSTR::null(),
            )
        }?;

        let file = unsafe { OwnedFileDescriptor::from_raw_handle(handle.0) };

        Ok(Self { file, size })
    }

    pub(super) fn zeroed_impl(size: usize) -> std::io::Result<Self> {
        // we need usize to be 8 bytes on Windows so that we can split
        // the size into high and low parts
        const _: () = assert!(std::mem::size_of::<usize>() == 8);

        let size = size.next_multiple_of(page_size::get() as _);
        let (size_low, size_high) = split_size(effective_size(size));

        let handle = unsafe {
            CreateFileMappingA(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_EXECUTE_READWRITE,
                size_high as _,
                size_low as _,
                PCSTR::null(),
            )
        }?;

        let file = unsafe { OwnedFileDescriptor::from_raw_handle(handle.0) };

        Ok(Self { file, size })
    }

    pub(super) fn as_raw_fd(&self) -> RawHandle {
        self.file.as_raw_handle()
    }
}

impl<S> View<S> {
    pub(super) fn new(
        snapshot: S,
        fd: RawHandle,
        size: usize,
        mode: ViewMode,
    ) -> std::io::Result<Self> {
        let placeholder = unsafe {
            VirtualAlloc2(
                None,
                None,
                effective_size(size),
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS.0,
                None,
            )
        };
        if placeholder.is_null() {
            return Err(std::io::Error::last_os_error())?;
        }
        let ptr = unsafe {
            MapViewOfFile3(
                HANDLE(fd),
                None,
                Some(placeholder as *const _),
                0,
                effective_size(size),
                MEM_REPLACE_PLACEHOLDER,
                mode.as_winapi().0,
                None,
            )
        };
        if ptr.Value.is_null() {
            return Err(std::io::Error::last_os_error())?;
        }
        if ptr.Value != placeholder {
            return Err(std::io::Error::other(format!(
                "Snapshot mapping failed: pointer mismatch, received {:?}, expected {:?}",
                ptr.Value, placeholder
            )))?;
        }
        let ptr = ptr.Value as _;
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
        unsafe {
            UnmapViewOfFileEx(
                MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.ptr as _,
                },
                MEM_PRESERVE_PLACEHOLDER,
            )
        }?;
        let new_ptr = unsafe {
            MapViewOfFile3(
                HANDLE(self.fd),
                None,
                Some(self.ptr as *const _),
                0,
                effective_size(self.size),
                MEM_REPLACE_PLACEHOLDER,
                self.mode.as_winapi().0,
                None,
            )
        };
        if new_ptr.Value.is_null() {
            println!("trying to map to {:?}", self.ptr);
            return Err(std::io::Error::last_os_error())?;
        }
        let new_ptr: *mut u8 = new_ptr.Value as _;
        if new_ptr != self.ptr {
            return Err(std::io::Error::other(format!(
                "Snapshot restore failed: pointer mismatch, received {:?}, expected {:?}",
                new_ptr, self.ptr
            )))?;
        }
        Ok(())
    }

    pub(super) fn protect_impl(
        &mut self,
        offset: Range<usize>,
        allow: Access,
    ) -> std::io::Result<()> {
        let mut old: PAGE_PROTECTION_FLAGS = PAGE_PROTECTION_FLAGS(0);

        unsafe {
            VirtualProtect(
                self.ptr.add(offset.start) as _,
                offset.len(),
                allow.as_winapi(self.mode),
                &mut old as *mut _,
            )
        }?;

        Ok(())
    }
}

impl<S> Drop for View<S> {
    fn drop(&mut self) {
        let _ = unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.ptr as _,
            })
        };
    }
}

impl Access {
    fn as_winapi(&self, mode: ViewMode) -> PAGE_PROTECTION_FLAGS {
        if *self == Access::NONE {
            return PAGE_NOACCESS;
        }

        let r = self.contains(Access::READ);
        let w = self.contains(Access::WRITE);
        let x = self.contains(Access::EXEC);
        let mutable = mode == ViewMode::Mutable;

        match (r, w, x, mutable) {
            // with exec
            (_, true, true, true) => PAGE_EXECUTE_READWRITE,
            (_, true, true, false) => PAGE_EXECUTE_WRITECOPY,
            (true, _, true, _) => PAGE_EXECUTE_READ,
            (_, _, true, _) => PAGE_EXECUTE,
            // without exec
            (_, true, false, true) => PAGE_READWRITE,
            (_, true, false, false) => PAGE_WRITECOPY,
            (true, _, false, _) => PAGE_READONLY,
            (_, _, false, _) => PAGE_NOACCESS,
        }
    }
}

impl ViewMode {
    fn as_winapi(&self) -> PAGE_PROTECTION_FLAGS {
        match self {
            Self::Cow => PAGE_WRITECOPY,
            Self::Mutable => PAGE_READWRITE,
        }
    }
}

fn split_size(size: usize) -> (u32, u32) {
    let high = (size >> 32) as u32;
    let low = (size & 0xFFFFFFFF) as u32;
    (low, high)
}
