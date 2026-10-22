//! Return unused allocator pages after desktop initialization or window destruction.
//!
//! This is a best-effort hint, not a working-set limit. Live allocations remain
//! valid, and normal allocation continues when the window is opened again.

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
