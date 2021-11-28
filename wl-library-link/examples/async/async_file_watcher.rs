use std::{
    ffi::CStr,
    fs,
    os::raw::c_uint,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use wl_library_link::{
    self as wll,
    sys::{self, mint, MArgument, LIBRARY_FUNCTION_ERROR, LIBRARY_NO_ERROR},
    AsyncTaskObject, DataStore,
};

/// Start an asynchronous task that will watch for modifications to a file.
///
/// See `RustLink/Tests/AsyncExamples.wlt` for example usage of this function.
#[no_mangle]
pub extern "C" fn start_file_watcher(
    lib_data: sys::WolframLibraryData,
    arg_count: mint,
    args: *mut MArgument,
    res: MArgument,
) -> c_uint {
    let args = unsafe { std::slice::from_raw_parts(args, arg_count as usize) };

    if args.len() != 2 {
        return LIBRARY_FUNCTION_ERROR;
    }

    if wll::initialize(lib_data).is_err() {
        return LIBRARY_FUNCTION_ERROR;
    }

    let pause_interval_ms =
        u64::try_from(unsafe { *args[0].integer }).expect("i64 interval overflows u64");

    let path: &CStr = unsafe { CStr::from_ptr(*args[1].utf8string) };
    let path: PathBuf = match path.to_str() {
        Ok(s) => PathBuf::from(s),
        Err(_) => return LIBRARY_FUNCTION_ERROR,
    };

    // Spawn a new thread, which will run in the background and check for file
    // modifications.
    let task_id = wll::spawn_async_task_with_thread(move |id: AsyncTaskObject| {
        file_watch_thread_function(id, pause_interval_ms, &path)
    });

    unsafe {
        *res.integer = task_id.id();
    }

    LIBRARY_NO_ERROR
}

/// This function is called first from the spawned background thread.
fn file_watch_thread_function(
    async_object: wll::AsyncTaskObject,
    pause_interval_ms: u64,
    path: &PathBuf,
) {
    let mut prev_changed: Option<SystemTime> = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok();

    // Stateful closure which checks if the file at `path` has been modified since the
    // last time this closure was called (and `prev_changed was updated). Using a closure
    // simplifies the control flow in the main `loop` below, which should sleep on every
    // iteration regardless of how this function returns.
    let mut check_for_modification = || -> Option<_> {
        let changed: Option<fs::Metadata> = fs::metadata(path).ok();

        let notify: Option<SystemTime> = match (&prev_changed, changed) {
            (Some(prev), Some(latest)) => {
                let latest: SystemTime = match latest.modified() {
                    Ok(latest) => latest,
                    Err(_) => return None,
                };

                if *prev != latest {
                    prev_changed = Some(latest.clone());
                    Some(latest)
                } else {
                    None
                }
            },
            // TODO: Notify on file removal?
            (Some(_prev), None) => None,
            (None, Some(latest)) => latest.modified().ok(),
            (None, None) => None,
        };

        let time = notify?;

        let since_epoch = match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration,
            Err(_) => return None,
        };

        let since_epoch = since_epoch.as_secs();

        Some(since_epoch)
    };

    loop {
        if !async_object.is_alive() {
            break;
        }

        // Check to see if the file has been modified. If it has, raise an async event
        // called "change", and attach the modification timestamp as event data.
        if let Some(modification) = check_for_modification() {
            let mut data = DataStore::new();
            data.add_i64(modification as i64);

            async_object.raise_async_event("change", data);
        }

        // Wait for a bit before polling again for any changes to the file.
        std::thread::sleep(Duration::from_millis(pause_interval_ms));
    }
}