# memsnap

A library for creating memory snapshots and views with copy-on-write semantics.

## Overview

`memsnap` has two main components: `Snapshot` and `View`. The `Snapshot` represents an opaque memory region that can be created from various sources (like files or byte slices), while the `View` provides a way to access and manipulate that memory region with different semantics.

`memsnap` allows you to:
- Create memory snapshots from files, byte slices, or zero-initialized memory
- Create copy-on-write views that allow you to modify memory without affecting the original data
- Create mutable views for direct modification of a snapshot

Currently `memsnap` uses `mmap` on Unix and `MapViewOfFile3` on Windows.

## Usage

### Basic Example

```rust
use memsnap::Snapshot;

fn main() -> std::io::Result<()> {
    // Create snapshot from a byte slice
    let snapshot = Snapshot::from_slice(b"Hello, World!")?;

    // Create a copy-on-write view
    let mut cow_view = snapshot.view()?;
    
    // Modifications to the view don't affect the original memory
    cow_view[0] = b'h';
    assert_eq!(&cow_view[..5], b"hello");
    
    // Create another view to verify the original is unchanged
    let original_view = snapshot.view()?;
    assert_eq!(&original_view[..5], b"Hello");

    Ok(())
}
```

### Mutable Views

```rust
use memsnap::Snapshot;

fn main() -> std::io::Result<()> {
    let mut snapshot = Snapshot::from_slice(b"Hello, World!")?;
    
    // Create a mutable view
    {
        let mut mut_view = snapshot.view_mut()?;
        mut_view[0] = b'J';
        mut_view[7] = b'R';
    }
    
    // Changes are reflected in the original snapshot
    let view = snapshot.view()?;
    assert_eq!(&view[..13], b"Jello, Rorld!");

    Ok(())
}
```

### Working with Files

```rust
use memsnap::Snapshot;
use std::fs::File;
use std::io::Write as _;
use std::env::temp_dir;

fn main() -> std::io::Result<()> {
    // Create a temporary file and write some data to it
    let root = tempfile::tempdir()?;
    let mut file = File::create_new(root.path().join("example.txt"))?;
    file.write_all(b"Hello, File!")?;

    // Create a snapshot from that file
    let snapshot = Snapshot::from_file(file)?;
    
    // Work with the file content
    let view = snapshot.view()?;
    assert_eq!(&view[..12], b"Hello, File!");
    
    Ok(())
}
```

### Arc-based Views for 'static Lifetime

```rust
use memsnap::Snapshot;
use std::sync::Arc;

fn main() -> std::io::Result<()> {
    let snapshot = Arc::new(Snapshot::from_slice(b"Shared data")?);
    
    // Create a view with no lifetime restrictions
    let view = snapshot.view_arc()?;
    
    // The view can be sent to other threads
    std::thread::spawn(move || {
        println!("Data from thread: {:?}", &view[..11]);
    }).join().unwrap();
    
    Ok(())
}
```

### Taking new Snapshots

```rust
use memsnap::Snapshot;

fn main() -> std::io::Result<()> {
    let mut snapshot1 = Snapshot::from_slice(b"Original")?;
    let mut view1 = snapshot1.view_mut()?;

    // Modify the view
    view1[0] = b'M';
    
    // Create a snapshot of the current state
    let snapshot2 = view1.take_snapshot()?;
    
    // Further modifications don't affect the snapshot
    view1[1] = b'X';
    
    let view2 = snapshot2.view()?;
    assert_eq!(&view2[..8], b"Mriginal");
    assert_eq!(&view1[..8], b"MXiginal");
    
    Ok(())
}
```
