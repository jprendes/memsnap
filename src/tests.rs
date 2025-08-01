use std::hint::black_box;
use std::io::Write as _;
use std::sync::Arc;

use segv_test::assert_segv;

use super::{Access, Snapshot};

#[test]
fn test_zeroed() {
    // Test that MemorySnapshot::zeroed genertes a snapshot full of zeros
    // of at least the requested size (it may be larger due to alignment)
    let snapshot = Snapshot::zeroed(1).unwrap();
    let view = snapshot.view().unwrap();
    assert!(view.len() >= 1);
    assert!(view.as_slice().iter().all(|&b| b == 0));
}

#[test]
fn test_from_slice() {
    // Test that MemorySnapshot::from_slice genertes a snapshot initialized to
    // the contents of the slice.
    // The resulting allocation may be larger than the slice due to alignment.
    let snapshot = Snapshot::from_slice(b"hello slice").unwrap();
    let view = snapshot.view().unwrap();
    assert_eq!(&view[..11], b"hello slice");
}

#[test]
fn test_from_file() {
    // Test that MemorySnapshot::from_file creates a snapshot initialized to
    // the contents of the file.
    let d = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create_new(d.path().join("tempfile")).unwrap();
    f.write_all(b"hello file").unwrap();
    let snapshot = Snapshot::from_file(f).unwrap();
    let view = snapshot.view().unwrap();
    assert_eq!(&view[..10], b"hello file");
}

#[test]
fn test_view_mut() {
    // Test that mutating a snapshot view with view_mut actually mutates the
    // original snapshot.
    let mut snapshot = Snapshot::zeroed(10).unwrap();

    snapshot.view_mut().unwrap().as_mut_slice()[0..10].copy_from_slice(b"0123456789");

    let view = snapshot.view().unwrap();
    assert_eq!(&view[..10], b"0123456789");
}

#[test]
fn test_view_restore() {
    // Test that restoring a view works and that it restores the original
    // contents of the snapshot without changing the view's address.
    let snapshot = Snapshot::from_slice(b"0123456789").unwrap();

    let mut view = snapshot.view().unwrap();
    view[0..10].copy_from_slice(b"9876543210");
    assert_eq!(&view[..10], b"9876543210");

    let ptr = view.as_ptr();

    view.restore().unwrap();

    assert_eq!(&view[..10], b"0123456789");

    let new_ptr = view.as_ptr();

    assert_eq!(ptr, new_ptr);
}

#[test]
fn test_view_cow() {
    // Test that mutating a snapshot view with view_cow does not mutate the
    // original snapshot.
    // CoW views of the same snapshot should not interfere with each other.
    let snapshot = Snapshot::zeroed(10).unwrap();

    let mut view1 = snapshot.view().unwrap();
    view1.as_mut_slice()[0..10].copy_from_slice(b"0123456789");

    let view2 = snapshot.view().unwrap();

    assert_eq!(&view1[..10], b"0123456789");
    assert_ne!(&view2[..10], b"0000000000");
}

#[test]
fn test_view_arc() {
    // Test that an Arc-wrapped snapshot can be cow view and the lifetime
    // of the view is independent of the lifetime of the Arc-wrapped snapshot.
    let snapshot = Snapshot::from_slice(b"hello world").unwrap();
    let snapshot = Arc::new(snapshot);

    let view = snapshot.view_arc().unwrap();

    drop(snapshot);

    assert_eq!(&view[..11], b"hello world");
}

#[test]
fn test_try_clone_snapshot() {
    // Test that cloning a snapshot works and that mutating the original snapshot
    // does not affect the cloned snapshot.
    let mut snapshot1 = Snapshot::from_slice(b"hello world").unwrap();
    let snapshot2 = snapshot1.try_clone().unwrap();

    let mut view1 = snapshot1.view_mut().unwrap();
    view1[0..11].copy_from_slice(b"hello slice");

    let view2 = snapshot2.view().unwrap();
    assert_eq!(&view2[..11], b"hello world");
}

#[test]
fn test_take_snapshot() {
    // Test that taking a snapshot from a view works and that mutating the
    // original view does not affect the new snapshot.
    let mut snapshot1 = Snapshot::from_slice(b"hello world").unwrap();
    let mut view1 = snapshot1.view_mut().unwrap();
    view1[0..11].copy_from_slice(b"hello slice");

    let snapshot2 = view1.take_snapshot().unwrap();
    let view2 = snapshot2.view().unwrap();

    assert_eq!(&view2[..11], b"hello slice");

    view1[0..11].copy_from_slice(b"hello world");

    assert_eq!(&view2[..11], b"hello slice");
}

#[test]
fn test_protect_none() {
    // Test that protecting a view with MemoryAccess::NONE causes a
    // segmentation fault when reading from it
    let mut snapshot1 = Snapshot::from_slice(b"hello world").unwrap();
    let mut view = snapshot1.view_mut().unwrap();
    view.protect(.., Access::NONE).unwrap();

    assert_segv!(black_box(view[0]));
    assert_segv!(view[0] = 1);
}

#[test]
fn test_protect_read() {
    // Test that protecting a view with MemoryAccess::READ can successfully
    // read from that memory
    let mut snapshot1 = Snapshot::from_slice(b"hello world").unwrap();
    let mut view = snapshot1.view_mut().unwrap();
    view.protect(.., Access::READ).unwrap();

    black_box(view[0]);
    assert_segv!(view[0] = 1);
}

#[test]
fn test_protect_write() {
    // Test that protecting a view with MemoryAccess::WRITE can successfully
    // write to that memory
    let mut snapshot1 = Snapshot::from_slice(b"hello world").unwrap();
    let mut view = snapshot1.view_mut().unwrap();
    view.protect(.., Access::WRITE).unwrap();

    black_box(view[0]);
    view[0] = 1;
}

#[test]
fn test_empty_snapshot() {
    // Test that protecting a view with MemoryAccess::WRITE can successfully
    // write to that memory
    let snapshot1 = Snapshot::from_slice(&[]).unwrap();
    let view = snapshot1.view().unwrap();
    assert_eq!(view.len(), 0);
}

#[test]
fn test_empty_file() {
    // Test that MemorySnapshot::from_file creates a snapshot initialized to
    // the contents of the file.
    let d = tempfile::tempdir().unwrap();
    let f = std::fs::File::create_new(d.path().join("tempfile")).unwrap();
    let snapshot = Snapshot::from_file(f).unwrap();
    let view = snapshot.view().unwrap();
    assert_eq!(&view[..10], b"hello file");
}
