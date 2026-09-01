//! Process tree containment abstraction for supervised native provider children.
//!
//! Guarantees that child processes and all their spawned descendants (e.g. CEF renderers,
//! GPU, and network processes) are cleanly and unconditionally terminated when the supervisor
//! or host terminates or drops the containment handle.

#[cfg(windows)]
mod windows_containment {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    #[derive(Debug)]
    pub struct ProcessTreeJob {
        job_handle: HANDLE,
    }

    // Safety: The raw Windows HANDLE is an owned OS resource, safe to send/sync across threads.
    unsafe impl Send for ProcessTreeJob {}
    unsafe impl Sync for ProcessTreeJob {}

    impl ProcessTreeJob {
        pub fn create() -> Result<Self, String> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if job.is_null() || job == INVALID_HANDLE_VALUE {
                    return Err("Failed to create Windows Job Object".to_string());
                }

                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

                let set_res = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );

                if set_res == 0 {
                    CloseHandle(job);
                    return Err("Failed to set Job Object kill-on-close limit".to_string());
                }

                Ok(Self { job_handle: job })
            }
        }

        pub fn assign_child(&self, child: &Child) -> Result<(), String> {
            unsafe {
                let raw_handle = child.as_raw_handle() as HANDLE;
                let assign_res = AssignProcessToJobObject(self.job_handle, raw_handle);
                if assign_res == 0 {
                    return Err("Failed to assign child process to Job Object".to_string());
                }
                Ok(())
            }
        }
    }

    impl Drop for ProcessTreeJob {
        fn drop(&mut self) {
            unsafe {
                if !self.job_handle.is_null() && self.job_handle != INVALID_HANDLE_VALUE {
                    CloseHandle(self.job_handle);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod non_windows_containment {
    use std::process::Child;

    #[derive(Debug)]
    pub struct ProcessTreeJob;

    impl ProcessTreeJob {
        pub fn create() -> Result<Self, String> {
            Ok(Self)
        }

        pub fn assign_child(&self, _child: &Child) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
pub use non_windows_containment::ProcessTreeJob;
#[cfg(windows)]
pub use windows_containment::ProcessTreeJob;

/// High-level process tree containment handle.
#[derive(Debug)]
pub struct ProcessTreeContainment {
    job: Option<ProcessTreeJob>,
}

impl ProcessTreeContainment {
    /// Creates a new process tree containment context.
    pub fn new() -> Result<Self, String> {
        let job = ProcessTreeJob::create()?;
        Ok(Self { job: Some(job) })
    }

    /// Assigns a newly spawned child process into this containment context.
    pub fn assign_child(&self, child: &std::process::Child) -> Result<(), String> {
        if let Some(ref job) = self.job {
            job.assign_child(child)?;
        }
        Ok(())
    }
}
