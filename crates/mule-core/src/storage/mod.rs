// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

pub mod fadvise;
pub mod finalizer;
pub mod part_file;

pub use fadvise::{
    advise_dontneed_all, advise_dontneed_range, advise_sequential, EvictingFileReader,
    FADVISE_WINDOW_BYTES,
};
pub use finalizer::{
    copy_chunked, copy_chunked_with_progress, finalize_file, finalize_file_with_progress,
    CollisionStrategy,
};
pub use part_file::{AllocationPolicy, PartFile, PartFileIoStats};
