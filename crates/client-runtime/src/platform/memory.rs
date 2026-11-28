//! Configure the desktop backend allocator and release unused pages after UI work.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn has_malloc_policy(legacy: bool, tunables: &str) -> bool {
    legacy
        || tunables
            .split(':')
            .filter_map(|entry| entry.split_once('='))
            .any(|(key, _)| {
                matches!(
                    key,
                    "glibc.malloc.mmap_threshold"
                        | "glibc.malloc.trim_threshold"
                        | "glibc.malloc.mmap_max"
                        | "glibc.malloc.top_pad"
                )
            })
}

/// Configure the owner's allocator before GUI libraries or the async runtime start.
///
/// # Safety
/// Must run at single-threaded process startup, before libraries or background
/// workers can allocate concurrently. Do not call from another thread or a signal handler.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub unsafe fn initialize_owner_allocator() {
    let legacy = [
        "MALLOC_MMAP_THRESHOLD_",
        "MALLOC_TRIM_THRESHOLD_",
        "MALLOC_MMAP_MAX_",
        "MALLOC_TOP_PAD_",
    ]
    .into_iter()
    .any(|key| std::env::var_os(key).is_some());
    let tunables = std::env::var_os("GLIBC_TUNABLES").unwrap_or_default();
    if !has_malloc_policy(legacy, &tunables.to_string_lossy()) {
        // SAFETY: the owner calls this before starting threads. The threshold
        // is a valid c_int in bytes and mallopt does not borrow program memory.
        let _ = unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024) };
    }
}

/// Best-effort reclamation of free pages; live allocations and subsequent allocations remain valid.
pub fn release_unused_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: glibc's thread-safe trim only releases completely free pages.
    unsafe {
        libc::malloc_trim(0);
    }

    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: NULL examines all zones; zero requests maximal release of
        // unused pages. libSystem supplies this API on macOS 10.7 and later.
        unsafe {
            malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
        }
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{
            Memory::{HeapOptimizeResources, HeapSetInformation},
            SystemServices::{
                HEAP_OPTIMIZE_RESOURCES_CURRENT_VERSION, HEAP_OPTIMIZE_RESOURCES_INFORMATION,
            },
        };
        let information = HEAP_OPTIMIZE_RESOURCES_INFORMATION {
            Version: HEAP_OPTIMIZE_RESOURCES_CURRENT_VERSION,
            Flags: 0,
        };
        // SAFETY: a NULL handle optimizes all LFH caches. The SDK structure is
        // initialized and remains alive for this synchronous call. Failure is
        // harmless; heaps without releasable resources can remain unchanged.
        unsafe {
            HeapSetInformation(
                std::ptr::null_mut(),
                HeapOptimizeResources,
                (&information as *const HEAP_OPTIMIZE_RESOURCES_INFORMATION).cast(),
                std::mem::size_of_val(&information),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn preserves_explicit_large_allocation_policies() {
        assert!(has_malloc_policy(true, ""));
        for entry in [
            "glibc.malloc.mmap_threshold=262144",
            "glibc.malloc.trim_threshold=1048576",
            "glibc.malloc.mmap_max=0",
            "glibc.malloc.top_pad=65536",
        ] {
            assert!(has_malloc_policy(
                false,
                &format!("glibc.malloc.arena_max=2:{entry}")
            ));
        }
        assert!(!has_malloc_policy(false, ""));
        assert!(!has_malloc_policy(false, "glibc.malloc.arena_max=2"));
    }

    #[test]
    fn reclaim_keeps_live_allocations_valid_and_allows_further_allocation() {
        let live: Vec<Vec<u8>> = (0..16).map(|n| vec![n; 65536]).collect();
        let temporary: Vec<Vec<u8>> = (0..64).map(|n| vec![n; 32768]).collect();
        drop(temporary);
        release_unused_heap();
        for (n, bytes) in live.iter().enumerate() {
            assert!(bytes.iter().all(|&byte| byte == n as u8));
        }
        let next = vec![0xa5; 131072];
        release_unused_heap();
        assert!(next.iter().all(|&byte| byte == 0xa5));
        assert_eq!(live[15][32768], 15);
    }
}
