use std::ops::{Bound, Deref, Index, IndexMut, RangeBounds};
use std::slice::SliceIndex;
use std::sync::Arc;

use bitflags::bitflags;

#[cfg_attr(target_os = "linux", path = "impl/linux.rs")]
#[cfg_attr(target_os = "windows", path = "impl/win.rs")]
mod r#impl;

use r#impl::{OwnedFileDescriptor, RawFileDescriptor};

/// A copy-on-write view into the content of a [`Snapshot`],
/// similar to [`CowView`] but with `'static` lifetime.
/// See [`View`] for more details.
pub type ArcView = View<Arc<Snapshot>>;

/// A copy-on-write view into the content of a [`Snapshot`].
/// Changes to this view do not affect the root snapshot.
/// See [`View`] for more details.
pub type CowView<'a> = View<&'a Snapshot>;

/// A mutable view into the content of a [`Snapshot`].
/// Changes to this view are reflected in the root snapshot.
/// See [`View`] for more details.
pub type MutView<'a> = View<&'a mut Snapshot>;

/// A representation of a memory snapshot.
/// To access the content of the snapshot you need to create a
/// [`View`] with one of:
/// * [`view`](Snapshot::view): A copy-on-write view where
///   changes do not affect the root snapshot.
/// * [`view_mut`](Snapshot::view_mut): A mutable view where
///   changes are reflected in the root snapshot.
/// * [`view_arc`](Snapshot::view_arc): A copy-on-write view
///   with `'static` lifetime where changes do not affect the
///   root snapshot.
#[derive(Debug)]
pub struct Snapshot {
    file: OwnedFileDescriptor,
    size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Cow,
    Mutable,
}

/// A view into the content of a [`Snapshot`] that can be used to
/// read or write into it.
///
/// There are two types of views:
/// * [`CowView`]: A copy-on-write view where changes to the view
///   do not affect the root snapshot.
/// * [`MutView`]: A mutable view where changes to the view
///   modify the root snapshot.
///
/// Normal rust borrowing semantics apply, where only one mutable
/// view can exist at a time, and multiple immutable views
/// can exist simultaneously.
/// Both views are tied to the lifetime with which they borrow the
/// snapshot, so they cannot outlive the snapshot they reference.
///
/// A third type of view [`ArcView`] is similar to [`CowView`]
/// but must be created from a reference-counted [`Arc<Snapshot>`].
/// Unlike [`CowView`], it has no lifetime requirements.
#[derive(Debug)]
pub struct View<S> {
    fd: RawFileDescriptor,
    ptr: *mut u8,
    size: usize,
    mode: ViewMode,
    _snapshot: S,
}

unsafe impl<S> Send for View<S> {}
unsafe impl<S> Sync for View<S> {}

impl Snapshot {
    /// Create a new snapshot from a file.
    /// The snapshot is populated with the content of the file.
    pub fn from_file(file: std::fs::File) -> std::io::Result<Self> {
        Self::from_file_impl(file)
    }

    /// Create a new snapshot with zeroed content of the given size.
    /// The actual snapshot size will be rounded up to the next system page size.
    pub fn zeroed(size: usize) -> std::io::Result<Self> {
        Self::zeroed_impl(size)
    }

    /// Create a new snapshot from a byte slice.
    /// The snapshot is populated with the content of the slice.
    /// The actual snapshot size will be rounded up to the next system page size.
    pub fn from_slice(buf: &[u8]) -> std::io::Result<Self> {
        let mut this = Self::zeroed(buf.len())?;
        this.view_mut()?.as_mut_slice()[0..buf.len()].copy_from_slice(buf);
        Ok(this)
    }

    /// Create a new snapshot cloned from this snapshot.
    /// The new snapshot will have the same content as this snapshot.
    /// The new snapshot is independent of this snapshot, meaning
    /// that changes to either snapshot will not affect the other.
    ///
    /// Note: This method copies the entire content of the snapshot and
    /// depending on its size, it can be slow.
    pub fn try_clone(&self) -> std::io::Result<Self> {
        Self::from_slice(self.view()?.as_slice())
    }
}

impl Snapshot {
    /// Create a copy-on-write view into the content of this snapshot.
    /// Changes to this view do not affect the snapshot.
    /// The view holds an immutable borrow of the snapshot, and has a
    /// lifetime tied to this borrow.
    pub fn view(&self) -> std::io::Result<CowView> {
        CowView::new(self, self.as_raw_fd(), self.size, ViewMode::Cow)
    }

    /// Create a mutable view into the content of this snapshot.
    /// Changes to this view are reflected in the root snapshot.
    /// The view holds a mutable borrow of the snapshot, and has a
    /// lifetime tied to this borrow.
    /// Only one mutable view can exist at a time.
    pub fn view_mut(&mut self) -> std::io::Result<MutView> {
        MutView::new(self, self.as_raw_fd(), self.size, ViewMode::Mutable)
    }

    /// Create a copy-on-write view into the content of this snapshot
    /// through an [`Arc`].
    /// Changes to this view do not affect the snapshot.
    /// The view has no lifetime requirements.
    pub fn view_arc(self: &Arc<Self>) -> std::io::Result<ArcView> {
        ArcView::new(self.clone(), self.as_raw_fd(), self.size, ViewMode::Cow)
    }
}

impl<S> View<S> {
    /// Returns the length of the view in bytes.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns `true` if the view is empty.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Returns a slice containing the entire view.
    /// This is equicalent to `&view[..]`,
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    /// Returns a mutable slice containing the entire view.
    /// This is equicalent to `&mut view[..]`,
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    /// Returns the base pointer of the view.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Returns the base mutable pointer of the view.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// Creates a new snapshot from the current content of this view,
    /// including any changes made to it.
    ///
    /// Note: This method copies the entire content of the view and
    /// depending on the size of the snapshot, it can be slow.
    pub fn take_snapshot(&self) -> std::io::Result<Snapshot> {
        // TODO: be clever in the case where the memory hasn't been modified
        // and just return the root snapshot.
        // This would probably require different implementations for each
        // View alias, since the optimization doesn't work for
        // MutView.
        Snapshot::from_slice(self.as_slice())
    }

    /// Restrict the access permissions of a memory region on this view.
    /// The `region` parameter specifies the range of bytes to protect,
    /// and the `allow` parameter specifies the access permissions to allow.
    /// The range must be page-aligned and within the bounds of the view.
    /// The access permissions can be combined using bitwise OR.
    #[allow(dead_code)] // this feature is still experimental
    pub(crate) fn protect(
        &mut self,
        region: impl RangeBounds<usize>,
        allow: Access,
    ) -> std::io::Result<()> {
        let start = match region.start_bound() {
            Bound::Included(&s) => s,
            Bound::Excluded(&s) => s + 1,
            Bound::Unbounded => 0,
        };
        let end = match region.end_bound() {
            Bound::Included(&s) => s + 1,
            Bound::Excluded(&s) => s,
            Bound::Unbounded => self.size,
        };

        if end <= start || end > self.size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid range for memory protection",
            ));
        }

        if start != start.next_multiple_of(page_size::get())
            || end != end.next_multiple_of(page_size::get())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Memory protection range must be page-aligned",
            ));
        }

        self.protect_impl(start..end, allow)
    }

    /// Discard any changes made to this copy-on-write view, restoring
    /// it to the original content of the root snapshot.
    /// Restoring a view also reverts any memory protection applied to the view.
    /// Restoring a view does not change its address.
    pub fn restore(&mut self) -> std::io::Result<()> {
        if self.mode == ViewMode::Mutable {
            // For mutable views, restoring is a no-op since they always
            // reflect the root snapshot.
            return Ok(());
        }
        self.restore_impl()
    }
}

impl<I: SliceIndex<[u8]>, S: Deref<Target = Snapshot>> Index<I> for View<S> {
    type Output = I::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        Index::index(self.as_slice(), index)
    }
}

impl<I: SliceIndex<[u8]>, S: Deref<Target = Snapshot>> IndexMut<I> for View<S> {
    #[inline]
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        IndexMut::index_mut(self.as_mut_slice(), index)
    }
}

bitflags! {
    /// Access permissions for a memory region.
    /// These flags can be used to control the type of access allowed
    /// to regions of a view with the [`protect`](View::protect) method.
    /// The flags can be combined using bitwise OR.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct Access: u8 {
        /// No access is allowed to the memory region.
        const NONE = 0x00;

        /// Only read access is allowed to the memory region.
        const READ = 0x01;

        /// Read and write access are allowed to the memory region.
        const WRITE = 0x02;

        /// Read and execute access is allowed to the memory region.
        const EXEC = 0x04;
    }
}

/// Returns the system page size in bytes.
/// This is the granularity at which memory allocation is done on the system.
pub fn page_size() -> usize {
    page_size::get()
}

#[cfg(test)]
mod tests;

#[cfg(doctest)]
mod readme;
